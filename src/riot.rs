//! HTTP access to the Riot Client's local API and Riot's game servers.
//!
//! Every call here is a read the game client itself makes: the lobby and Riot IDs. The only PUT is
//! the name lookup, which takes a list of player ids and changes nothing.

use crate::local_files::{GameLogInfo, Lockfile, game_log_path, lockfile_path, parse_game_log, parse_lockfile};
use crate::presence::{Presence, parse_presences};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Base64 of the platform JSON the Windows client sends.
fn client_platform() -> String {
    let json = r#"{"platformType":"PC","platformOS":"Windows","platformOSVersion":"10.0.19042.1.256.64bit","platformChipset":"Unknown"}"#;
    base64::engine::general_purpose::STANDARD.encode(json)
}

#[derive(Debug, Clone)]
struct Tokens {
    access: String,
    entitlement: String,
}

/// Request counters shown in the status corner.
#[derive(Debug, Default)]
pub struct NetStats {
    pub started: AtomicU32,
    pub finished: AtomicU32,
    pub rate_limited_until: Mutex<Option<Instant>>,
}

pub struct Riot {
    local: reqwest::Client,
    remote: reqwest::Client,
    lock: Lockfile,
    pub log: GameLogInfo,
    pub puuid: String,
    tokens: tokio::sync::RwLock<Tokens>,
    permits: Semaphore,
    /// When Riot's latest Retry-After ends.
    retry_at: Mutex<Option<Instant>>,
    dump_dir: Option<PathBuf>,
    last_presences: Mutex<Option<Value>>,
    pub net: Arc<NetStats>,
}

/// Connects to the running Riot Client. Fails when the client isn't running or isn't signed in.
pub async fn connect(dump_dir: Option<PathBuf>) -> Result<Riot> {
    let lock_path = lockfile_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let lock = parse_lockfile(&std::fs::read_to_string(&lock_path).context("Riot Client is not running")?)?;
    let log_path = game_log_path().ok_or_else(|| anyhow!("LOCALAPPDATA is not set"))?;
    let log = parse_game_log(&std::fs::read_to_string(&log_path).context("VALORANT has never been started on this PC")?)?;
    // The local API serves a self-signed certificate on 127.0.0.1; only that client skips verification.
    let local = reqwest::Client::builder().tls_danger_accept_invalid_certs(true).timeout(Duration::from_secs(5)).build()?;
    let remote = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let (tokens, puuid) = fetch_tokens(&local, &lock).await?;
    if let Some(d) = &dump_dir {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    Ok(Riot {
        local,
        remote,
        lock,
        log,
        puuid,
        tokens: tokio::sync::RwLock::new(tokens),
        permits: Semaphore::new(4),
        retry_at: Mutex::default(),
        dump_dir,
        last_presences: Mutex::default(),
        net: Arc::default(),
    })
}

async fn local_get(client: &reqwest::Client, lock: &Lockfile, path: &str) -> Result<(StatusCode, Value)> {
    let resp = client
        .get(format!("https://127.0.0.1:{}{}", lock.port, path))
        .basic_auth("riot", Some(&lock.password))
        .send()
        .await
        .context("Riot Client is not responding")?;
    let status = resp.status();
    Ok((status, resp.json().await.unwrap_or(Value::Null)))
}

async fn fetch_tokens(client: &reqwest::Client, lock: &Lockfile) -> Result<(Tokens, String)> {
    let (status, v) = local_get(client, lock, "/entitlements/v1/token").await?;
    if !status.is_success() {
        bail!("Riot Client is running but not signed in");
    }
    let s = |k: &str| v[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("token response missing {k}"));
    Ok((Tokens { access: s("accessToken")?, entitlement: s("token")? }, s("subject")?))
}

impl Riot {
    pub fn pd(&self) -> String {
        format!("https://pd.{}.a.pvp.net", self.log.shard)
    }

    pub async fn presences(&self) -> Result<Vec<Presence>> {
        let (status, v) = local_get(&self.local, &self.lock, "/chat/v4/presences").await?;
        if !status.is_success() {
            bail!("presences returned {status}");
        }
        if self.dump_dir.is_some() {
            let mut last = self.last_presences.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref() != Some(&v) {
                dump(self.dump_dir.as_deref(), "presences", &v);
                *last = Some(v.clone());
            }
        }
        Ok(parse_presences(&v))
    }

    /// A request to the game's player data server. `Ok(None)` means 404.
    pub async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Option<Value>> {
        let url = format!("{}{path}", self.pd());
        let _permit = self.permits.acquire().await?;
        self.net.started.fetch_add(1, Ordering::Relaxed);
        let result = self.send_with_retry(method, &url, body).await;
        self.net.finished.fetch_add(1, Ordering::Relaxed);
        if let Ok(Some(v)) = &result {
            dump(self.dump_dir.as_deref(), path, v);
        }
        result
    }

    async fn send_with_retry(&self, method: Method, url: &str, body: Option<&Value>) -> Result<Option<Value>> {
        let path = error_path(url);
        // Until Riot's Retry-After ends, asking again only earns another 429.
        if self.retry_at.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t > Instant::now()) {
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
            // reqwest's error holds the URL, which can hold player ids; the context names the path.
            let resp = req.send().await.map_err(reqwest::Error::without_url).with_context(|| format!("request failed for {path}"))?;
            let status = resp.status();
            if status == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                // Waiting here would hold up everything else, such as noticing that the match started.
                // The caller asks again on its own schedule, and the footer counts the wait down.
                let until = Instant::now() + retry_wait(resp.headers().get("retry-after").and_then(|h| h.to_str().ok()));
                *self.retry_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
                *self.net.rate_limited_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
                return Err(anyhow::Error::new(RateLimited).context(path));
            }
            let text = resp.text().await.unwrap_or_default();
            let expired = status == StatusCode::UNAUTHORIZED || (status == StatusCode::BAD_REQUEST && text.contains("BAD_CLAIMS"));
            if expired && !refreshed {
                let (tokens, _) = fetch_tokens(&self.local, &self.lock).await?;
                *self.tokens.write().await = tokens;
                refreshed = true;
                continue;
            }
            if !status.is_success() {
                bail!("{status} from {path}");
            }
            return Ok(Some(serde_json::from_str(&text).unwrap_or(Value::Null)));
        }
    }

    // ---- endpoints ----

    /// Riot ids for the given players, as (puuid, name, tag).
    pub async fn names(&self, puuids: &[String]) -> Result<Vec<(String, String, String)>> {
        let body = Value::from(puuids.to_vec());
        let v = self.send(Method::PUT, "/name-service/v2/players", Some(&body)).await?;
        Ok(v.and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|n| {
                Some((
                    n["Subject"].as_str()?.to_lowercase(),
                    n["GameName"].as_str()?.to_string(),
                    n["TagLine"].as_str().unwrap_or("").to_string(),
                ))
            })
            .collect())
    }
}

/// A request Riot refused with 429, or one not sent because its Retry-After hasn't ended.
#[derive(Debug)]
pub struct RateLimited;

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rate limited (429)")
    }
}

impl std::error::Error for RateLimited {}

/// The path of a game-server URL, as errors name it. Errors reach the footer, and from there
/// screenshots, so player and match ids are left out.
fn error_path(url: &str) -> String {
    let path = url.split_once(".a.pvp.net").map_or(url, |(_, p)| p).split('?').next().unwrap_or(url);
    path.split('/').map(|s| if is_id(s) { "…" } else { s }).collect::<Vec<_>>().join("/")
}

/// Riot's player and match ids are UUIDs.
fn is_id(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// How long to wait after a 429: the server's `Retry-After` seconds, capped at a day so an absurd value
/// can't overflow the deadline, else five seconds.
fn retry_wait(retry_after: Option<&str>) -> Duration {
    const MAX_RETRY_AFTER: u64 = 86_400;
    let secs = retry_after.and_then(|s| s.trim().parse::<u64>().ok()).map(|s| s.min(MAX_RETRY_AFTER));
    Duration::from_secs(secs.unwrap_or(5))
}

/// With `--dump`, saves a raw response under the dump directory for checking field names.
pub fn dump(dir: Option<&Path>, label: &str, v: &Value) {
    let Some(dir) = dir else { return };
    let name: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let stamp = chrono::Utc::now().format("%H%M%S%3f");
    let _ = std::fs::write(dir.join(format!("{stamp}_{}.json", name.trim_matches('_'))), serde_json::to_vec_pretty(v).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_paths_leave_out_ids() {
        let id = ["0".repeat(8), "0".repeat(4), "0".repeat(4), "0".repeat(4), "0".repeat(12)].join("-");
        let url = format!("https://glz-eu-1.eu.a.pvp.net/core-game/v1/players/{id}?x=1");
        assert_eq!(error_path(&url), "/core-game/v1/players/…");
        assert_eq!(error_path("https://pd.eu.a.pvp.net/name-service/v2/players"), "/name-service/v2/players");
        assert!(!is_id(&id.replace('-', "0")), "an id has its dashes");
    }

    #[test]
    fn retry_wait_follows_retry_after_within_a_day() {
        assert_eq!(retry_wait(Some("7")), Duration::from_secs(7));
        assert_eq!(retry_wait(Some(" 7 ")), Duration::from_secs(7));
        assert_eq!(retry_wait(None), Duration::from_secs(5), "no header");
        assert_eq!(retry_wait(Some("Wed, 21 Oct 2026 07:28:00 GMT")), Duration::from_secs(5), "a date counts as no header");
        let absurd = retry_wait(Some("18446744073709551615"));
        assert_eq!(absurd, Duration::from_secs(86_400));
        let _ = Instant::now() + absurd;
    }
}
