//! Player stats from tracker.gg, fetched by its own web page in a hidden browser (see `browser`).
//!
//! tracker.gg has no public Valorant API. Its staff allowed this app to call the endpoints its site
//! uses, as a browser would, until they offer one.

use crate::browser::{Browser, HOME, Response};
use crate::riot::{NetStats, dump};
use crate::stats::{ActRecord, PlayerMatch, Record, RecordSource};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const API: &str = "https://api.tracker.gg/api/v2/valorant/standard";

/// tracker.gg answers 451 for a player who hasn't made their profile public by signing in there.
#[derive(Debug)]
pub struct Private;

impl std::fmt::Display for Private {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this tracker.gg profile is private")
    }
}

impl std::error::Error for Private {}

/// Where requests go: the browser, or canned answers in tests.
enum Transport {
    /// Started by the first request and closed after `IDLE` without one, so its memory (about
    /// 100 MB) is only used while stats load.
    Browser { profile_dir: PathBuf, home: String, open: tokio::sync::Mutex<Option<(Arc<Browser>, Instant)>> },
    #[cfg(test)]
    Canned {
        responses: Mutex<std::collections::VecDeque<Response>>,
        sent: std::sync::atomic::AtomicUsize,
        reloads: std::sync::atomic::AtomicUsize,
    },
}

impl Transport {
    async fn get(&self, url: &str) -> Result<Response> {
        match self {
            Self::Browser { profile_dir, home, open } => {
                let browser = {
                    let mut open = open.lock().await;
                    let browser = match open.take() {
                        Some((b, _)) => b,
                        None => {
                            let (dir, home) = (profile_dir.clone(), home.clone());
                            let start = tokio::task::spawn_blocking(move || Browser::start(&dir, &home, REQUEST_TIMEOUT));
                            let started = tokio::time::timeout(REQUEST_TIMEOUT, start).await;
                            Arc::new(started.map_err(|_| anyhow!("WebView2 didn't start within {} s", REQUEST_TIMEOUT.as_secs()))???)
                        }
                    };
                    *open = Some((browser.clone(), Instant::now()));
                    browser
                };
                let result = browser.get(url).await;
                // A browser that timed out or stopped may be stuck; the next request starts a fresh one.
                if result.is_err() {
                    let mut open = open.lock().await;
                    if open.as_ref().is_some_and(|(b, _)| Arc::ptr_eq(b, &browser)) {
                        *open = None;
                    }
                }
                result
            }
            #[cfg(test)]
            Self::Canned { responses, sent, .. } => {
                sent.fetch_add(1, Ordering::Relaxed);
                Ok(responses.lock().unwrap().pop_front().expect("a canned response for every request"))
            }
        }
    }

    async fn reload(&self) -> Result<()> {
        match self {
            Self::Browser { open, .. } => match open.lock().await.as_ref() {
                Some((b, _)) => b.reload(),
                None => Ok(()),
            },
            #[cfg(test)]
            Self::Canned { reloads, .. } => {
                reloads.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    fn close_if_idle(&self) {
        match self {
            Self::Browser { open, .. } => {
                let Ok(mut open) = open.try_lock() else { return };
                // Requests still running hold their own handle to the browser.
                if open.as_ref().is_some_and(|(b, used)| used.elapsed() > IDLE && Arc::strong_count(b) == 1) {
                    *open = None;
                }
            }
            #[cfg(test)]
            Self::Canned { .. } => {}
        }
    }
}

pub struct Tracker {
    transport: Transport,
    permits: Semaphore,
    net: Arc<NetStats>,
    dump_dir: Option<PathBuf>,
    /// No request is sent until then, and why: after tracker.gg refuses one twice, so a block isn't
    /// met with a retry every few seconds, or when it asked for a wait too long to hold a request slot
    /// through.
    paused: Mutex<Option<(Instant, &'static str)>>,
}

/// How long requests pause after tracker.gg refuses one even from a freshly loaded page.
const BLOCK_PAUSE: Duration = Duration::from_secs(300);
/// The longest `Retry-After` waited out inside a request; longer ones pause every request instead.
const MAX_INLINE_WAIT: u64 = 30;
const MAX_RETRY_AFTER: u64 = 86_400;
/// How long the browser stays open without a request.
const IDLE: Duration = Duration::from_secs(120);
/// How long the browser may take to start, and a request to answer, page load included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

impl Tracker {
    /// `profile_dir` holds the browser's cookies between runs.
    pub fn new(net: Arc<NetStats>, dump_dir: Option<PathBuf>, profile_dir: PathBuf) -> Self {
        Self::with(Transport::Browser { profile_dir, home: HOME.into(), open: Default::default() }, net, dump_dir)
    }

    fn with(transport: Transport, net: Arc<NetStats>, dump_dir: Option<PathBuf>) -> Self {
        Self { transport, permits: Semaphore::new(2), net, dump_dir, paused: Mutex::default() }
    }

    /// A tracker that answers each request with the next of `responses`.
    #[cfg(test)]
    pub(crate) fn canned(responses: Vec<Response>) -> Self {
        let transport =
            Transport::Canned { responses: Mutex::new(responses.into()), sent: Default::default(), reloads: Default::default() };
        Self::with(transport, Arc::default(), None)
    }

    /// Closes the browser once it has gone `IDLE` without a request.
    pub fn close_if_idle(&self) {
        self.transport.close_if_idle();
    }

    /// Pauses requests for `wait`, unless an existing pause already lasts longer.
    fn pause(&self, wait: Duration, why: &'static str) {
        let until = Instant::now() + wait;
        let mut paused = self.paused.lock().unwrap_or_else(|e| e.into_inner());
        if paused.is_none_or(|(t, _)| until > t) {
            *paused = Some((until, why));
        }
    }

    /// GET on tracker.gg's API. `Ok(None)` means 404: no such player, or no data of that kind.
    async fn get(&self, path: &str) -> Result<Option<Value>> {
        let _permit = self.permits.acquire().await?;
        if let Some((until, why)) = *self.paused.lock().unwrap_or_else(|e| e.into_inner())
            && until > Instant::now()
        {
            bail!("{why}; trying again in {} min", until.saturating_duration_since(Instant::now()).as_secs().div_ceil(60));
        }
        self.net.started.fetch_add(1, Ordering::Relaxed);
        let result = self.get_with_retry(path).await;
        self.net.finished.fetch_add(1, Ordering::Relaxed);
        if let Ok(Some(v)) = &result {
            dump(self.dump_dir.as_deref(), &format!("tracker{}", path.split('?').next().unwrap_or(path)), v);
        }
        result
    }

    async fn get_with_retry(&self, path: &str) -> Result<Option<Value>> {
        let label = path.split('?').next().unwrap_or(path);
        let mut reloaded = false;
        for attempt in 0..5u64 {
            let resp =
                self.transport.get(&format!("{API}{path}")).await.with_context(|| format!("tracker.gg request failed for {label}"))?;
            let status = resp.status;
            if status == 404 {
                return Ok(None);
            }
            // Only the seconds form of Retry-After is read; an HTTP date counts as no header. Capped at a
            // day so an absurd value can't overflow the deadline.
            let retry_after = resp.retry_after.as_deref().and_then(|s| s.trim().parse::<u64>().ok()).map(|s| s.min(MAX_RETRY_AFTER));
            // A 503 is retried here only when the server says when; otherwise the caller retries later.
            if status == 429 || (status == 503 && retry_after.is_some()) {
                let wait = retry_after.unwrap_or(5 * (attempt + 1));
                let until = Instant::now() + Duration::from_secs(wait);
                *self.net.rate_limited_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
                if wait > MAX_INLINE_WAIT {
                    self.pause(Duration::from_secs(wait), "tracker.gg asked to wait");
                    bail!("tracker.gg asked to wait {wait} s ({status}) on {label}");
                }
                tokio::time::sleep_until(until.into()).await;
                continue;
            }
            // Cloudflare's block page is a 403, or unreadable from the page (status 0) since it carries no
            // CORS headers. Loading the page again lets Cloudflare check the browser, which can lift it.
            if status == 403 || status == 0 {
                if !reloaded {
                    reloaded = true;
                    self.transport.reload().await?;
                    continue;
                }
                self.pause(BLOCK_PAUSE, "tracker.gg refused a request");
                bail!("tracker.gg refused the request ({}) for {label}", if status == 0 { resp.body.as_str() } else { "403" });
            }
            if status == 451 {
                return Err(Private.into());
            }
            if !(200..300).contains(&status) {
                bail!("{status} from tracker.gg {label}");
            }
            return Ok(Some(serde_json::from_str(&resp.body).with_context(|| format!("tracker.gg {label} isn't JSON"))?));
        }
        bail!("tracker.gg still asks to wait (429/503) on {label}")
    }

    pub async fn profile(&self, riot_id: &str) -> Result<Option<Value>> {
        self.get(&format!("/profile/riot/{riot_id}?")).await
    }

    pub async fn season(&self, riot_id: &str, act_id: &str) -> Result<Option<Value>> {
        self.get(&format!("/profile/riot/{riot_id}/segments/season?playlist=competitive&seasonId={act_id}&source=web")).await
    }

    pub async fn maps(&self, riot_id: &str, act_id: &str) -> Result<Option<Value>> {
        self.get(&format!("/profile/riot/{riot_id}/segments/map?playlist=competitive&seasonId={act_id}&source=web")).await
    }

    pub async fn matches(&self, riot_id: &str) -> Result<Option<Value>> {
        self.get(&format!("/matches/riot/{riot_id}?platform=pc&type=competitive")).await
    }
}

/// `name#tag` as it appears in tracker.gg's URLs: every byte outside the unreserved set is
/// percent-encoded, so `#` becomes `%23`.
pub fn riot_id_path(name: &str, tag: &str) -> String {
    format!("{name}#{tag}")
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A player's competitive record and rank in one act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActStats {
    pub act_id: String,
    pub tier: u32,
    pub record: Record,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Current rank; tier 0 is unranked.
    pub tier: u32,
    /// Ranked rating within the tier, when tracker.gg shows it; it often doesn't.
    pub rr: Option<u32>,
    pub peak_tier: u32,
    /// The latest act the player has played.
    pub acts: Vec<ActStats>,
}

impl Profile {
    /// This act's record, else last act's.
    pub fn act_record(&self, current: &str, previous: Option<&str>) -> Option<ActRecord> {
        let played = |id: &str| self.acts.iter().find(|a| a.act_id == id).filter(|a| a.record.games() > 0).map(|a| a.record);
        if let Some(record) = played(current) {
            return Some(ActRecord { record, source: RecordSource::ThisAct });
        }
        previous.and_then(played).map(|record| ActRecord { record, source: RecordSource::PreviousAct })
    }
}

/// One of a player's recent competitive matches, from their side.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecentMatch {
    pub map_id: String,
    /// `None` for a draw.
    pub won: Option<bool>,
    pub rounds_won: u32,
    pub rounds_lost: u32,
    pub player: PlayerMatch,
}

/// The tier in one of tracker.gg's rank icon URLs (`.../tiersv2/24.png`), which use today's numbering.
fn tier_from_icon(stat: &Value) -> u32 {
    stat["metadata"]["iconUrl"]
        .as_str()
        .and_then(|u| u.rsplit('/').next())
        .and_then(|f| f.strip_suffix(".png"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn stat_u32(stats: &Value, key: &str) -> u32 {
    stats[key]["value"].as_f64().map(|v| v.max(0.0).round() as u32).unwrap_or(0)
}

fn record(stats: &Value) -> Record {
    Record { wins: stat_u32(stats, "matchesWon"), losses: stat_u32(stats, "matchesLost") }
}

/// Segments from either response shape: `{ segments: [...] }` (profile) or a bare list (segment endpoints).
fn segments(v: &Value) -> impl Iterator<Item = &Value> {
    let data = &v["data"];
    data.as_array().or_else(|| data["segments"].as_array()).into_iter().flatten()
}

/// Rank, peak and latest act from `/profile/riot/{id}`. Its season entry is the latest act the player
/// has played, which isn't the current one until they play a game in it; until then they're unranked.
pub fn parse_profile(v: &Value, current_act: &str) -> Profile {
    let Some(season) = segments(v).find(|s| s["type"] == "season") else { return Profile::default() };
    let stats = &season["stats"];
    let act = parse_season(v);
    let current = act.as_ref().is_some_and(|a| a.act_id.eq_ignore_ascii_case(current_act));
    Profile {
        tier: if current { tier_from_icon(&stats["rank"]) } else { 0 },
        rr: if current { stats["rank"]["value"].as_f64().map(|v| v.max(0.0).round() as u32) } else { None },
        peak_tier: tier_from_icon(&stats["peakRank"]),
        acts: act.into_iter().collect(),
    }
}

/// One act's rank and record from a response's season entry (`/segments/season?seasonId=...`).
pub fn parse_season(v: &Value) -> Option<ActStats> {
    let s = segments(v).find(|s| s["type"] == "season")?;
    Some(ActStats {
        act_id: s["attributes"]["seasonId"].as_str()?.to_lowercase(),
        tier: tier_from_icon(&s["stats"]["rank"]),
        record: record(&s["stats"]),
    })
}

/// Wins and losses per map (by display name) from `/segments/map`, most played first.
pub fn parse_maps(v: &Value) -> Vec<(String, Record)> {
    let mut maps: Vec<(String, Record)> = segments(v)
        .filter(|s| s["type"] == "map")
        .filter_map(|s| Some((s["metadata"]["name"].as_str()?.to_string(), record(&s["stats"]))))
        .filter(|(_, r)| r.games() > 0)
        .collect();
    maps.sort_by(|(a, ra), (b, rb)| rb.games().cmp(&ra.games()).then_with(|| a.cmp(b)));
    maps
}

/// The newest `count` matches from `/matches/riot/{id}`.
pub fn parse_matches(v: &Value, count: usize) -> Vec<RecentMatch> {
    v["data"]["matches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let seg = m["segments"].as_array()?.iter().find(|s| s["type"] == "overview")?;
            let st = &seg["stats"];
            let n = |k: &str| stat_u32(st, k);
            let won = match m["metadata"]["result"].as_str() {
                Some("victory") => Some(true),
                Some("defeat") => Some(false),
                _ => None,
            };
            Some(RecentMatch {
                map_id: m["metadata"]["map"].as_str().unwrap_or("").to_string(),
                won,
                rounds_won: n("roundsWon"),
                rounds_lost: n("roundsLost"),
                player: PlayerMatch {
                    agent_id: seg["metadata"]["agent"].as_str().unwrap_or("").to_string(),
                    kills: n("kills"),
                    deaths: n("deaths"),
                    assists: n("assists"),
                    score: n("score"),
                    rounds: n("roundsPlayed"),
                    damage: n("damage"),
                    headshots: n("dealtHeadshots"),
                    bodyshots: n("dealtBodyshots"),
                    legshots: n("dealtLegshots"),
                    performance: st["performance"]["value"].as_f64(),
                    ..Default::default()
                },
            })
        })
        .take(count)
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn stat(v: Value) -> Value {
        json!({ "value": v, "metadata": {} })
    }

    fn rank(tier: u32, rr: u32) -> Value {
        json!({ "value": rr, "metadata": { "iconUrl": format!("https://trackercdn.com/cdn/tracker.gg/valorant/icons/tiersv2/{tier}.png") } })
    }

    fn season(act: &str, tier: u32, rr: u32, peak: u32, w: u32, l: u32) -> Value {
        json!({ "type": "season", "attributes": { "seasonId": act, "playlist": "competitive" },
                "stats": { "rank": rank(tier, rr), "peakRank": rank(peak, 0), "matchesWon": stat(json!(w)), "matchesLost": stat(json!(l)), "matchesTied": stat(json!(1)) } })
    }

    #[test]
    fn riot_ids_are_percent_encoded() {
        assert_eq!(riot_id_path("blue berry", "TAG"), "blue%20berry%23TAG");
        assert_eq!(riot_id_path("한", "DEMO"), "%ED%95%9C%23DEMO");
        assert_eq!(riot_id_path("a.b_c~d-e", "1"), "a.b_c~d-e%231");
    }

    #[test]
    fn profile_reads_the_current_act() {
        let v = json!({ "data": { "segments": [
            { "type": "agent", "attributes": { "seasonId": "A6" }, "stats": {} },
            season("A6", 24, 23, 25, 29, 7)
        ]}});
        let p = parse_profile(&v, "a6");
        assert_eq!((p.tier, p.rr, p.peak_tier), (24, Some(23), 25));
        let mut hidden = v.clone();
        hidden["data"]["segments"][1]["stats"]["rank"]["value"] = Value::Null;
        assert_eq!(parse_profile(&hidden, "a6").rr, None, "tracker.gg often leaves RR out; it isn't 0");
        assert_eq!(
            p.acts,
            [ActStats { act_id: "a6".into(), tier: 24, record: Record { wins: 29, losses: 7 } }],
            "ties don't count as losses"
        );
        assert_eq!(
            p.act_record("a6", Some("a5")),
            Some(ActRecord { record: Record { wins: 29, losses: 7 }, source: RecordSource::ThisAct })
        );
    }

    #[test]
    fn no_games_this_act_is_unranked_but_keeps_peak_and_last_act() {
        let v = json!({ "data": { "segments": [season("a5", 20, 50, 21, 65, 68)] } });
        let p = parse_profile(&v, "a6");
        assert_eq!((p.tier, p.rr, p.peak_tier), (0, None, 21));
        let r = p.act_record("a6", Some("a5")).unwrap();
        assert_eq!((r.record, r.source), (Record { wins: 65, losses: 68 }, RecordSource::PreviousAct));
        assert_eq!(p.act_record("a6", Some("a4")), None, "an act older than the last one isn't this act's record");
        assert_eq!(parse_profile(&json!({ "data": { "segments": [] } }), "a6"), Profile::default());
    }

    #[test]
    fn season_endpoint_reads_its_season_entry() {
        let a = parse_season(&json!({ "data": [{ "type": "weapon" }, season("A4", 17, 0, 18, 30, 27)] })).unwrap();
        assert_eq!(a, ActStats { act_id: "a4".into(), tier: 17, record: Record { wins: 30, losses: 27 } });
        assert_eq!(parse_season(&json!({ "data": [{ "type": "weapon" }] })), None, "an unplayed act has no season entry");
    }

    #[test]
    fn maps_skip_unplayed_and_sort_by_games() {
        let map = |name: &str, w: u32, l: u32| json!({ "type": "map", "metadata": { "name": name }, "stats": { "matchesWon": stat(json!(w)), "matchesLost": stat(json!(l)) } });
        let v = json!({ "data": [map("Bind", 5, 2), map("Abyss", 4, 1), { "type": "map-top-agent", "metadata": { "name": "Jett" }, "stats": {} }, map("Pearl", 0, 0), map("Ascent", 3, 4)] });
        let maps = parse_maps(&v);
        let names: Vec<_> = maps.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Ascent", "Bind", "Abyss"]);
        assert_eq!(maps[1].1, Record { wins: 5, losses: 2 });
    }

    #[test]
    fn matches_read_the_players_side() {
        let st = |k: &str, v: f64| (k.to_string(), stat(json!(v)));
        let stats: serde_json::Map<String, Value> = [
            st("roundsWon", 13.0),
            st("roundsLost", 7.0),
            st("kills", 24.0),
            st("deaths", 13.0),
            st("assists", 7.0),
            st("score", 7037.0),
            st("roundsPlayed", 20.0),
            st("damage", 4300.0),
            st("dealtHeadshots", 4.0),
            st("dealtBodyshots", 40.0),
            st("dealtLegshots", 0.0),
            st("performance", 400.97),
        ]
        .into_iter()
        .collect();
        let m = |id: &str, result: &str| {
            json!({ "attributes": { "id": id }, "metadata": { "map": "92584fbe", "result": result },
                    "segments": [{ "type": "overview", "metadata": { "agent": "add6443a" }, "stats": stats.clone() }] })
        };
        let v =
            json!({ "data": { "matches": [m("m1", "victory"), m("m2", "defeat"), m("m3", "draw"), { "attributes": { "id": "bad" } }] } });
        let all = parse_matches(&v, 5);
        assert_eq!(all.len(), 3, "a match without the player's segment is skipped");
        let first = &all[0];
        assert_eq!((first.won, first.rounds_won, first.rounds_lost, first.map_id.as_str()), (Some(true), 13, 7, "92584fbe"));
        assert_eq!((first.player.kills, first.player.deaths, first.player.assists, first.player.acs()), (24, 13, 7, 352));
        assert_eq!(first.player.hs_pct(), Some(9));
        assert_eq!(first.player.performance, Some(400.97));
        assert_eq!((all[1].won, all[2].won), (Some(false), None));
        assert_eq!(parse_matches(&v, 1).len(), 1);
    }

    pub(crate) fn answer(status: u16, retry_after: Option<&str>, body: &str) -> Response {
        Response { status, retry_after: retry_after.map(str::to_string), body: body.into() }
    }

    fn counts(t: &Tracker) -> (usize, usize) {
        let Transport::Canned { sent, reloads, .. } = &t.transport else { unreachable!() };
        (sent.load(Ordering::Relaxed), reloads.load(Ordering::Relaxed))
    }

    #[tokio::test]
    async fn a_503_with_retry_after_waits_then_retries() {
        let t = Tracker::canned(vec![answer(503, Some("1"), ""), answer(200, None, r#"{"data":{}}"#)]);
        let start = Instant::now();
        let v = t.get("/x").await.unwrap();
        assert_eq!(v, Some(json!({ "data": {} })));
        assert_eq!(counts(&t).0, 2);
        assert!(start.elapsed() >= Duration::from_secs(1), "waited for Retry-After");
    }

    #[tokio::test]
    async fn a_503_without_a_seconds_retry_after_fails_for_the_caller_to_retry() {
        for retry_after in [None, Some("Wed, 21 Oct 2026 07:28:00 GMT")] {
            let t = Tracker::canned(vec![answer(503, retry_after, "")]);
            let err = t.get("/x").await.unwrap_err();
            assert!(err.to_string().contains("503"), "{err:#}");
            assert_eq!(counts(&t).0, 1, "no retry inside the request: {retry_after:?}");
        }
    }

    #[tokio::test]
    async fn a_long_retry_after_pauses_every_request_instead_of_holding_a_slot() {
        for status in [429, 503] {
            let t = Tracker::canned(vec![answer(status, Some("3600"), "")]);
            let start = Instant::now();
            assert!(t.get("/x").await.unwrap_err().to_string().contains("3600 s"), "{status}");
            let err = t.get("/y").await.unwrap_err();
            assert!(err.to_string().contains("trying again in 60 min"), "{status}: {err:#}");
            assert!(start.elapsed() < Duration::from_secs(5), "neither call waited: {status}");
            assert_eq!(counts(&t).0, 1, "the paused call sent nothing: {status}");
        }
    }

    #[tokio::test]
    async fn an_absurd_retry_after_is_capped_instead_of_overflowing() {
        let t = Tracker::canned(vec![answer(503, Some("18446744073709551615"), "")]);
        let err = t.get("/x").await.unwrap_err();
        assert!(err.to_string().contains("86400 s"), "{err:#}");
    }

    #[tokio::test]
    async fn a_refusal_reloads_the_page_and_tries_once_more() {
        for refusal in [answer(0, None, "TypeError: Failed to fetch"), answer(403, None, "")] {
            let t = Tracker::canned(vec![refusal.clone(), answer(200, None, r#"{"data":{}}"#)]);
            assert_eq!(t.get("/x").await.unwrap(), Some(json!({ "data": {} })), "{refusal:?}");
            assert_eq!(counts(&t), (2, 1), "one reload, then the retry went through: {refusal:?}");
        }
    }

    #[tokio::test]
    async fn a_refusal_after_a_reload_pauses_every_request() {
        let t = Tracker::canned(vec![answer(0, None, "TypeError: Failed to fetch"), answer(0, None, "TypeError: Failed to fetch")]);
        let err = t.get("/x").await.unwrap_err();
        assert!(err.to_string().contains("Failed to fetch"), "{err:#}");
        let err = t.get("/y").await.unwrap_err();
        assert!(err.to_string().contains("trying again in 5 min"), "{err:#}");
        assert_eq!(counts(&t), (2, 1), "one reload, and nothing sent while paused");
    }

    #[test]
    fn a_shorter_pause_never_cuts_a_longer_one() {
        let t = Tracker::canned(Vec::new());
        t.pause(Duration::from_secs(300), "blocked");
        t.pause(Duration::from_secs(45), "asked to wait");
        let (until, why) = t.paused.lock().unwrap().unwrap();
        assert_eq!(why, "blocked");
        assert!(until > Instant::now() + Duration::from_secs(290));
        t.pause(Duration::from_secs(600), "asked to wait");
        assert_eq!(t.paused.lock().unwrap().unwrap().1, "asked to wait", "a longer pause still extends it");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_browser_closes_only_when_idle_and_reopens_on_demand() {
        let (page, api) = crate::browser::tests::site().await;
        let profile = crate::browser::tests::Profile::new("idle-test");
        let transport = Transport::Browser { profile_dir: profile.0.clone(), home: format!("{page}/"), open: Default::default() };
        let t = Tracker::with(transport, Arc::default(), None);
        let Transport::Browser { open, .. } = &t.transport else { unreachable!() };
        let data = format!("{api}/data");
        assert_eq!(t.transport.get(&data).await.unwrap().status, 200, "the first request opens it");
        let first = open.lock().await.as_ref().unwrap().0.clone();
        t.close_if_idle();
        assert!(open.lock().await.is_some(), "just used");
        open.lock().await.as_mut().unwrap().1 -= IDLE * 2;
        t.close_if_idle();
        assert!(open.lock().await.is_some(), "a request is still holding it");
        drop(first);
        t.close_if_idle();
        assert!(open.lock().await.is_none(), "idle and unused");
        assert_eq!(t.transport.get(&data).await.unwrap().status, 200, "the next request opens a new one");
        assert!(open.lock().await.is_some());
    }

    /// Checks that tracker.gg still accepts these requests, with a ranked player who has games this
    /// act: `TEAMDIFF_LIVE_ID='name#tag' cargo test live -- --ignored --nocapture`. Setting
    /// `TEAMDIFF_LIVE_PRIVATE_ID` to a player with a private profile also checks that case.
    #[tokio::test]
    #[ignore = "calls tracker.gg"]
    async fn live_tracker_gg() {
        let id = std::env::var("TEAMDIFF_LIVE_ID").expect("set TEAMDIFF_LIVE_ID to a Riot ID");
        let (name, tag) = id.split_once('#').expect("name#tag");
        let t = Tracker::new(Arc::default(), None, std::env::temp_dir().join("teamdiff-live-test"));
        let path = riot_id_path(name, tag);
        let v = t.profile(&path).await.unwrap().expect("the player exists");
        let act = v["data"]["metadata"]["defaultSeason"].as_str().unwrap().to_lowercase();
        let profile = parse_profile(&v, &act);
        assert!(profile.tier > 0 && profile.peak_tier >= profile.tier, "{profile:?}");
        let maps = parse_maps(&t.maps(&path, &act).await.unwrap().unwrap());
        assert!(!maps.is_empty());
        let recent = parse_matches(&t.matches(&path).await.unwrap().unwrap(), 5);
        assert_eq!(recent.len(), 5);
        assert!(recent.iter().all(|m| m.player.rounds > 0 && !m.player.agent_id.is_empty() && m.player.performance.is_some()));
        assert!(t.profile("nobody%23zz9q9zz").await.unwrap().is_none(), "unknown players are a 404");
        if let Ok(id) = std::env::var("TEAMDIFF_LIVE_PRIVATE_ID") {
            let (name, tag) = id.split_once('#').expect("name#tag");
            let err = t.profile(&riot_id_path(name, tag)).await.expect_err("a private profile is refused");
            assert!(err.is::<Private>(), "{err:#}");
        }
        println!("tier {} rr {:?} peak {} act {:?}", profile.tier, profile.rr, profile.peak_tier, profile.acts);
        println!("maps {maps:?}");
        println!("kd/hs over 5: {:?}", crate::stats::pooled_kd_hs(recent.iter().map(|m| &m.player)));
    }
}
