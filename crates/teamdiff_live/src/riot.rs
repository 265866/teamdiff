//! HTTP access to the Riot Client's local API and Riot's game servers.
//!
//! Every call here is a read the game client itself makes: the lobby, Riot IDs, and your own match
//! results. Nothing here asks about another player's stats; those come from tracker.gg. The only PUT is
//! the name lookup, which takes a list of player ids and changes nothing.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::engine::general_purpose::STANDARD;
use jiff::Timestamp;
use reqwest::{Client, Method, StatusCode};
use serde_json::Value;
use teamdiff_core::content::Content;
use teamdiff_core::local_files::{self, GameLogInfo, Lockfile};
use teamdiff_core::model::RiotId;
use teamdiff_core::net_stats::NetStats;
use teamdiff_core::presence::{self, Presence};
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

use crate::mutex;

/// The valorant-api.com lists that [`fetch_content`] reads, in the order
/// [`Content::from_responses`] takes them.
const CONTENT_PATHS: [&str; 4] = [
    "agents?isPlayableCharacter=true",
    "maps",
    "competitivetiers",
    "seasons",
];

/// Base64 of the platform JSON the Windows client sends.
fn client_platform() -> String {
    use base64::Engine as _;

    let json = r#"{"platformType":"PC","platformOS":"Windows","platformOSVersion":"10.0.19042.1.256.64bit","platformChipset":"Unknown"}"#;
    STANDARD.encode(json)
}

/// The tokens the game servers take, which the Riot Client hands out once you're signed in.
#[derive(Debug, Clone)]
struct Tokens {
    access: String,
    entitlement: String,
}

/// What the Riot Client's token endpoint says about the account that's signed in.
#[derive(Debug)]
struct SignIn {
    tokens: Tokens,
    /// The account's PUUID.
    puuid: String,
}

/// An answer from the Riot Client's local API, whose body is `null` when it isn't JSON.
#[derive(Debug)]
struct LocalResponse {
    status: StatusCode,
    body: Value,
}

/// A player's Riot ID, as Riot's name service returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayerName {
    /// The player's PUUID, in lowercase.
    pub(crate) puuid: String,
    /// The player's Riot ID.
    pub(crate) riot_id: RiotId,
}

/// A connection to the running Riot Client and, through its tokens, to Riot's game servers.
#[derive(Debug)]
pub(crate) struct Riot {
    local: Client,
    remote: Client,
    lock: Lockfile,
    /// The region, shard and client version from the game's log.
    pub(crate) log: GameLogInfo,
    /// Your PUUID.
    pub(crate) puuid: String,
    tokens: RwLock<Tokens>,
    permits: Semaphore,
    /// When Riot's latest Retry-After ends.
    retry_at: Mutex<Option<Instant>>,
    dump_dir: Option<PathBuf>,
    last_presences: Mutex<Option<Value>>,
    /// The request counters, which the tracker.gg client and the footer share.
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the tracker.gg client counts into the same counters, and the model hands \
                      them to the footer, each for as long as it lives"
        )
    )]
    pub(crate) net: Arc<NetStats>,
}

/// Connects to the running Riot Client. Fails when the client isn't running or isn't signed in.
pub(crate) async fn connect(dump_dir: Option<PathBuf>) -> Result<Riot> {
    let lock_path =
        local_files::lockfile_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let lock = local_files::parse_lockfile(
        &fs::read_to_string(&lock_path).context("Riot Client is not running")?,
    )?;
    let log_path =
        local_files::game_log_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let log = local_files::parse_game_log(
        &fs::read_to_string(&log_path).context("VALORANT has never been started on this PC")?,
    )?;
    let local = Client::builder()
        .tls_danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()?;
    let remote = Client::builder().timeout(Duration::from_secs(15)).build()?;
    let SignIn { tokens, puuid } = fetch_tokens(&local, &lock).await?;
    if let Some(d) = &dump_dir {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    Ok(Riot {
        local,
        remote,
        lock,
        log,
        puuid,
        tokens: RwLock::new(tokens),
        permits: Semaphore::new(4),
        retry_at: Mutex::default(),
        dump_dir,
        last_presences: Mutex::default(),
        net: Arc::default(),
    })
}

/// GETs `path` from the Riot Client's local API, which serves a self-signed certificate on
/// 127.0.0.1 with the lockfile's password.
async fn local_get(client: &Client, lock: &Lockfile, path: &str) -> Result<LocalResponse> {
    let resp = client
        .get(format!("https://127.0.0.1:{}{}", lock.port, path))
        .basic_auth("riot", Some(&lock.password))
        .send()
        .await
        .context("Riot Client is not responding")?;
    let status = resp.status();
    Ok(LocalResponse {
        status,
        body: resp.json().await.unwrap_or(Value::Null),
    })
}

/// Asks the Riot Client for the signed-in account's tokens and PUUID.
async fn fetch_tokens(client: &Client, lock: &Lockfile) -> Result<SignIn> {
    let response = local_get(client, lock, "/entitlements/v1/token").await?;
    if !response.status.is_success() {
        bail!("Riot Client is running but not signed in");
    }
    let v = &response.body;
    let s = |k: &str| {
        v[k].as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("token response missing {k}"))
    };
    Ok(SignIn {
        tokens: Tokens {
            access: s("accessToken")?,
            entitlement: s("token")?,
        },
        puuid: s("subject")?,
    })
}

/// Which of Riot's game servers a request goes to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Host {
    /// The platform server: match history, ranks, names and accolades.
    Pd,
    /// The game-loop server: agent select and the running match.
    Glz,
}

impl Riot {
    /// The platform server of your shard.
    fn pd(&self) -> String {
        format!("https://pd.{}.a.pvp.net", self.log.shard)
    }

    /// The game-loop server of your region and shard.
    fn glz(&self) -> String {
        format!(
            "https://glz-{}-1.{}.a.pvp.net",
            self.log.region, self.log.shard
        )
    }

    /// Everyone's presence the Riot Client knows: you and your friends. With `--dump`, saves the
    /// response whenever it changes.
    pub(crate) async fn presences(&self) -> Result<Vec<Presence>> {
        let response = local_get(&self.local, &self.lock, "/chat/v4/presences").await?;
        if !response.status.is_success() {
            bail!("presences returned {}", response.status);
        }
        if self.dump_dir.is_some() {
            let mut last = mutex::lock(&self.last_presences);
            if last.as_ref() != Some(&response.body) {
                dump(self.dump_dir.as_deref(), "presences", &response.body);
                *last = Some(response.body.clone());
            }
        }
        Ok(presence::parse_presences(&response.body))
    }

    /// GET on a game server. `Ok(None)` means 404, which several endpoints use for "not in a match yet".
    pub(crate) async fn get(&self, host: Host, path: &str) -> Result<Option<Value>> {
        self.send(Method::GET, host, path, None).await
    }

    /// Sends a request to a game server, at most four at a time, and saves the answer with
    /// `--dump`. `Ok(None)` means 404.
    pub(crate) async fn send(
        &self,
        method: Method,
        host: Host,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        let base = match host {
            Host::Pd => self.pd(),
            Host::Glz => self.glz(),
        };
        let url = format!("{base}{path}");
        let _permit = self.permits.acquire().await?;
        self.net.started.fetch_add(1, Ordering::Relaxed);
        let result = self.send_with_retry(method, &url, body).await;
        self.net.finished.fetch_add(1, Ordering::Relaxed);
        if let Ok(Some(v)) = &result
            && saved_by_dump(path)
        {
            dump(self.dump_dir.as_deref(), path, v);
        }
        result
    }

    /// Sends the request once, and once more with fresh tokens when Riot says they expired. A
    /// 429 isn't waited out here: waiting would hold up everything else, such as noticing that
    /// the match started, so the caller asks again on its own schedule and the footer counts the
    /// wait down.
    async fn send_with_retry(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        let path = error_path(url);
        if mutex::lock(&self.retry_at).is_some_and(|t| t > Instant::now()) {
            return Err(anyhow::Error::new(RateLimited).context(path));
        }
        let mut refreshed = false;
        loop {
            let tokens = self.tokens.read().await.clone();
            let mut req = self
                .remote
                .request(method.clone(), url)
                .bearer_auth(&tokens.access)
                .header("X-Riot-Entitlements-JWT", &tokens.entitlement)
                .header("X-Riot-ClientVersion", &self.log.client_version)
                .header("X-Riot-ClientPlatform", client_platform());
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req
                .send()
                .await
                .map_err(reqwest::Error::without_url)
                .with_context(|| format!("request failed for {path}"))?;
            let status = resp.status();
            if status == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                let until = Instant::now()
                    + retry_wait(
                        resp.headers()
                            .get("retry-after")
                            .and_then(|h| h.to_str().ok()),
                    );
                *mutex::lock(&self.retry_at) = Some(until);
                *mutex::lock(&self.net.rate_limited_until) = Some(until);
                return Err(anyhow::Error::new(RateLimited).context(path));
            }
            let text = resp.text().await.unwrap_or_default();
            let expired = status == StatusCode::UNAUTHORIZED
                || (status == StatusCode::BAD_REQUEST && text.contains("BAD_CLAIMS"));
            if expired && !refreshed {
                let sign_in = fetch_tokens(&self.local, &self.lock).await?;
                *self.tokens.write().await = sign_in.tokens;
                refreshed = true;
                continue;
            }
            if !status.is_success() {
                bail!("{status} from {path}");
            }
            return Ok(Some(serde_json::from_str(&text).unwrap_or(Value::Null)));
        }
    }

    /// The match id of your agent select, if you're in one.
    pub(crate) async fn pregame_match_id(&self) -> Result<Option<String>> {
        let v = self
            .get(Host::Glz, &format!("/pregame/v1/players/{}", self.puuid))
            .await?;
        Ok(v.and_then(|v| v["MatchID"].as_str().map(str::to_string)))
    }

    /// The lobby of an agent select.
    pub(crate) async fn pregame_match(&self, id: &str) -> Result<Option<Value>> {
        self.get(Host::Glz, &format!("/pregame/v1/matches/{id}"))
            .await
    }

    /// The match id of your running match, if you're in one.
    pub(crate) async fn coregame_match_id(&self) -> Result<Option<String>> {
        let v = self
            .get(Host::Glz, &format!("/core-game/v1/players/{}", self.puuid))
            .await?;
        Ok(v.and_then(|v| v["MatchID"].as_str().map(str::to_string)))
    }

    /// The lobby of a running match.
    pub(crate) async fn coregame_match(&self, id: &str) -> Result<Option<Value>> {
        self.get(Host::Glz, &format!("/core-game/v1/matches/{id}"))
            .await
    }

    /// Your own latest RR changes.
    pub(crate) async fn my_comp_updates(&self, count: u32) -> Result<Option<Value>> {
        let path = format!(
            "/mmr/v1/players/{}/competitiveupdates?startIndex=0&endIndex={count}&queue=competitive",
            self.puuid
        );
        self.get(Host::Pd, &path).await
    }

    /// A finished match's stats.
    pub(crate) async fn match_details(&self, id: &str) -> Result<Option<Value>> {
        self.get(Host::Pd, &format!("/match-details/v1/matches/{id}"))
            .await
    }

    /// Riot IDs for the given players.
    pub(crate) async fn names(&self, puuids: &[String]) -> Result<Vec<PlayerName>> {
        let body = Value::from(puuids.to_vec());
        let v = self
            .send(
                Method::PUT,
                Host::Pd,
                "/name-service/v2/players",
                Some(&body),
            )
            .await?;
        Ok(v.as_ref().map_or_default(parse_names))
    }

    /// What each accolade id means.
    pub(crate) async fn goldstar_defs(&self) -> Result<Option<Value>> {
        self.get(Host::Pd, "/goldstars/v1/goldstars").await
    }

    /// Your own accolades.
    pub(crate) async fn player_goldstars(&self) -> Result<Option<Value>> {
        self.get(Host::Pd, &format!("/goldstars/v1/players/{}", self.puuid))
            .await
    }
}

/// Whether `--dump` may save the answer from `path`. The lobbies of agent select and the match
/// carry what the game hides, such as the enemy team's lock count, the starting side and the
/// server, so they are never saved.
fn saved_by_dump(path: &str) -> bool {
    !path.starts_with("/pregame/v1/matches/") && !path.starts_with("/core-game/v1/matches/")
}

/// The players in a name-service response. An entry without a PUUID or a game name is skipped,
/// and a missing tag line is empty.
fn parse_names(v: &Value) -> Vec<PlayerName> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            Some(PlayerName {
                puuid: n["Subject"].as_str()?.to_lowercase(),
                riot_id: RiotId {
                    name: n["GameName"].as_str()?.to_string(),
                    tag: n["TagLine"].as_str().unwrap_or("").to_string(),
                },
            })
        })
        .collect()
}

/// A request Riot refused with 429, or one not sent because its Retry-After hasn't ended.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub(crate) struct RateLimited;

impl Display for RateLimited {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("rate limited (429)")
    }
}

impl Error for RateLimited {}

/// The path of a game-server URL, as errors name it. Errors reach the footer, and from there
/// screenshots, so player and match ids are left out.
fn error_path(url: &str) -> String {
    let path = url
        .split_once(".a.pvp.net")
        .map_or(url, |(_, p)| p)
        .split('?')
        .next()
        .unwrap_or(url);
    let parts: Vec<&str> = path
        .split('/')
        .map(|s| if is_id(s) { "…" } else { s })
        .collect();
    parts.join("/")
}

/// Riot's player and match ids are UUIDs.
fn is_id(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// How long to wait after a 429: the server's `Retry-After` seconds, capped at a day so an absurd value
/// can't overflow the deadline, else five seconds.
fn retry_wait(retry_after: Option<&str>) -> Duration {
    const MAX_RETRY_AFTER: u64 = 86_400;
    let secs: Option<u64> = retry_after
        .and_then(|s| s.trim().parse().ok())
        .map(|s: u64| s.min(MAX_RETRY_AFTER));
    Duration::from_secs(secs.unwrap_or(5))
}

/// With `--dump`, saves a raw response under the dump directory for checking field names, as
/// `<UTC time>_<label>.json`.
pub(crate) fn dump(dir: Option<&Path>, label: &str, v: &Value) {
    let Some(dir) = dir else { return };
    let name: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = Timestamp::now().strftime("%H%M%S%3f");
    let file = dir.join(format!("{stamp}_{}.json", name.trim_matches('_')));
    #[expect(
        clippy::let_underscore_must_use,
        let_underscore_drop,
        reason = "--dump only helps check field names, so a response it can't save, as on a full \
                  disk, mustn't fail the request it came with"
    )]
    let _ = fs::write(file, serde_json::to_vec_pretty(v).unwrap_or_default());
}

/// Static content from valorant-api.com (names for agents, maps, ranks and acts). The four lists
/// load at once, and the first to fail fails the whole load.
pub(crate) async fn fetch_content() -> Result<Content> {
    let client = Client::builder().timeout(Duration::from_secs(15)).build()?;
    let mut requests = JoinSet::new();
    for (index, path) in CONTENT_PATHS.into_iter().enumerate() {
        let client = client.clone();
        requests.spawn(async move {
            let body = async {
                client
                    .get(format!("https://valorant-api.com/v1/{path}"))
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Value>()
                    .await
            };
            (index, body.await)
        });
    }
    let mut bodies = [Value::Null, Value::Null, Value::Null, Value::Null];
    while let Some(finished) = requests.join_next().await {
        let (index, body) = finished?;
        bodies[index] = body.context("couldn't load names from valorant-api.com")?;
    }
    let [agents, maps, tiers, seasons] = &bodies;
    Ok(Content::from_responses(agents, maps, tiers, seasons))
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use serde_json::json;

    use super::*;

    #[test]
    fn error_paths_leave_out_ids() {
        let id = [
            "0".repeat(8),
            "0".repeat(4),
            "0".repeat(4),
            "0".repeat(4),
            "0".repeat(12),
        ]
        .join("-");
        let url = format!("https://glz-eu-1.eu.a.pvp.net/core-game/v1/players/{id}?x=1");
        assert_eq!(error_path(&url), "/core-game/v1/players/…");
        assert_eq!(
            error_path("https://pd.eu.a.pvp.net/name-service/v2/players"),
            "/name-service/v2/players"
        );
        assert!(!is_id(&id.replace('-', "0")), "an id has its dashes");
    }

    #[test]
    fn retry_wait_follows_retry_after_within_a_day() {
        assert_eq!(retry_wait(Some("7")), Duration::from_secs(7));
        assert_eq!(retry_wait(Some(" 7 ")), Duration::from_secs(7));
        assert_eq!(retry_wait(None), Duration::from_secs(5), "no header");
        assert_eq!(
            retry_wait(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            Duration::from_secs(5),
            "a date counts as no header"
        );
        let absurd = retry_wait(Some("18446744073709551615"));
        assert_eq!(absurd, Duration::from_hours(24));
        assert!(
            Instant::now().checked_add(absurd).is_some(),
            "the capped wait fits in a deadline"
        );
    }

    #[test]
    fn names_read_each_player_with_a_name() {
        let v = json!([
            { "Subject": "ABC", "GameName": "Blue Berry", "TagLine": "DEMO" },
            { "Subject": "def", "GameName": "Notag" },
            { "Subject": "ghi" },
            { "GameName": "Nobody", "TagLine": "DEMO" }
        ]);
        let id = |name: &str, tag: &str| RiotId {
            name: name.into(),
            tag: tag.into(),
        };
        assert_eq!(
            parse_names(&v),
            [
                PlayerName {
                    puuid: "abc".into(),
                    riot_id: id("Blue Berry", "DEMO")
                },
                PlayerName {
                    puuid: "def".into(),
                    riot_id: id("Notag", "")
                },
            ],
            "PUUIDs are lowercased, a missing tag is empty, and an entry without a PUUID or a \
             name is skipped"
        );
        assert!(parse_names(&json!({})).is_empty(), "not a list");
    }

    #[test]
    fn the_client_platform_is_the_windows_clients() {
        use base64::Engine as _;

        let json = STANDARD.decode(client_platform()).unwrap();
        let v: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(v["platformType"], "PC");
        assert_eq!(v["platformOS"], "Windows");
    }

    #[test]
    fn match_lobbies_are_never_dumped() {
        assert!(!saved_by_dump("/pregame/v1/matches/x"));
        assert!(!saved_by_dump("/core-game/v1/matches/x"));
        assert!(saved_by_dump("/pregame/v1/players/x"));
        assert!(saved_by_dump("/match-details/v1/matches/x"));
    }

    #[test]
    fn a_dump_is_named_after_the_utc_time_and_its_label() {
        let dir = env::temp_dir().join(format!("teamdiff-dump-{}", process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        let before = Timestamp::now();
        dump(Some(&dir), "/mmr/v1/players?x=1", &json!({ "a": 1 }));
        let after = Timestamp::now();
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let (stamp, label) = names[0].split_once('_').unwrap();
        assert_eq!(label, "mmr_v1_players_x_1.json", "{names:?}");
        assert_eq!(stamp.len(), 9, "HHMMSS and milliseconds: {stamp}");
        assert!(stamp.bytes().all(|b| b.is_ascii_digit()), "{stamp}");
        let earliest = before.strftime("%H%M%S%3f").to_string();
        let latest = after.strftime("%H%M%S%3f").to_string();
        assert!(
            earliest.as_str() <= stamp && stamp <= latest.as_str() || latest < earliest,
            "{earliest} <= {stamp} <= {latest}, unless the test crossed midnight UTC"
        );
        let saved: Value = serde_json::from_slice(&fs::read(dir.join(&names[0])).unwrap()).unwrap();
        assert_eq!(saved, json!({ "a": 1 }));
        dump(None, "nowhere", &json!(null));
        fs::remove_dir_all(&dir).unwrap();
    }
}
