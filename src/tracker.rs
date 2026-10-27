//! Player stats from tracker.gg, fetched by its own web page in a hidden browser (see `browser`).
//!
//! tracker.gg has no public Valorant API. Its staff allowed this app to call the endpoints its site
//! uses, as a browser would, until they offer one.

use crate::browser::{Browser, HOME, Response};
use crate::riot::{NetStats, dump};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
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
    /// Started by the first request.
    Browser { profile_dir: PathBuf, home: String, open: tokio::sync::Mutex<Option<Arc<Browser>>> },
    #[cfg(test)]
    Canned(std::sync::Mutex<std::collections::VecDeque<Response>>),
}

impl Transport {
    async fn get(&self, url: &str) -> Result<Response> {
        match self {
            Self::Browser { profile_dir, home, open } => {
                let browser = {
                    let mut open = open.lock().await;
                    let browser = match open.take() {
                        Some(b) => b,
                        None => {
                            let (dir, home) = (profile_dir.clone(), home.clone());
                            let start = tokio::task::spawn_blocking(move || Browser::start(&dir, &home, REQUEST_TIMEOUT));
                            let started = tokio::time::timeout(REQUEST_TIMEOUT, start).await;
                            Arc::new(started.map_err(|_| anyhow!("WebView2 didn't start within {} s", REQUEST_TIMEOUT.as_secs()))???)
                        }
                    };
                    *open = Some(browser.clone());
                    browser
                };
                let result = browser.get(url).await;
                // A browser that timed out or stopped may be stuck; the next request starts a fresh one.
                if result.is_err() {
                    let mut open = open.lock().await;
                    if open.as_ref().is_some_and(|b| Arc::ptr_eq(b, &browser)) {
                        *open = None;
                    }
                }
                result
            }
            #[cfg(test)]
            Self::Canned(responses) => Ok(responses.lock().unwrap().pop_front().expect("a canned response for every request")),
        }
    }
}

pub struct Tracker {
    transport: Transport,
    permits: Semaphore,
    net: Arc<NetStats>,
    dump_dir: Option<PathBuf>,
}

/// How long the browser may take to start, and a request to answer, page load included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

impl Tracker {
    /// `profile_dir` holds the browser's cookies between runs.
    pub fn new(net: Arc<NetStats>, dump_dir: Option<PathBuf>, profile_dir: PathBuf) -> Self {
        Self::with(Transport::Browser { profile_dir, home: HOME.into(), open: Default::default() }, net, dump_dir)
    }

    fn with(transport: Transport, net: Arc<NetStats>, dump_dir: Option<PathBuf>) -> Self {
        Self { transport, permits: Semaphore::new(2), net, dump_dir }
    }

    /// A tracker that answers each request with the next of `responses`.
    #[cfg(test)]
    pub(crate) fn canned(responses: Vec<Response>) -> Self {
        Self::with(Transport::Canned(std::sync::Mutex::new(responses.into())), Arc::default(), None)
    }

    /// GET on tracker.gg's API. `Ok(None)` means 404: no such player, or no data of that kind.
    async fn get(&self, path: &str) -> Result<Option<Value>> {
        let _permit = self.permits.acquire().await?;
        self.net.started.fetch_add(1, Ordering::Relaxed);
        let result = self.request(path).await;
        self.net.finished.fetch_add(1, Ordering::Relaxed);
        if let Ok(Some(v)) = &result {
            dump(self.dump_dir.as_deref(), &format!("tracker{}", path.split('?').next().unwrap_or(path)), v);
        }
        result
    }

    async fn request(&self, path: &str) -> Result<Option<Value>> {
        let label = path.split('?').next().unwrap_or(path);
        let resp = self.transport.get(&format!("{API}{path}")).await.with_context(|| format!("tracker.gg request failed for {label}"))?;
        let status = resp.status;
        if status == 404 {
            return Ok(None);
        }
        // Cloudflare's block page is a 403, or unreadable from the page (status 0) since it carries no
        // CORS headers.
        if status == 403 || status == 0 {
            bail!("tracker.gg refused the request ({}) for {label}", if status == 0 { resp.body.as_str() } else { "403" });
        }
        if status == 451 {
            return Err(Private.into());
        }
        if !(200..300).contains(&status) {
            bail!("{status} from tracker.gg {label}");
        }
        Ok(Some(serde_json::from_str(&resp.body).with_context(|| format!("tracker.gg {label} isn't JSON"))?))
    }

    pub async fn profile(&self, riot_id: &str) -> Result<Option<Value>> {
        self.get(&format!("/profile/riot/{riot_id}?")).await
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// Current rank; tier 0 is unranked.
    pub tier: u32,
    /// Ranked rating within the tier, when tracker.gg shows it; it often doesn't.
    pub rr: Option<u32>,
    pub peak_tier: u32,
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

/// Rank and peak from `/profile/riot/{id}`. Its season entry is the latest act the player has played,
/// which isn't the current one until they play a game in it; until then they're unranked.
pub fn parse_profile(v: &Value, current_act: &str) -> Profile {
    let segments = v["data"]["segments"].as_array().into_iter().flatten();
    let Some(season) = segments.into_iter().find(|s| s["type"] == "season") else { return Profile::default() };
    let stats = &season["stats"];
    let current = season["attributes"]["seasonId"].as_str().is_some_and(|a| a.eq_ignore_ascii_case(current_act));
    Profile {
        tier: if current { tier_from_icon(&stats["rank"]) } else { 0 },
        rr: if current { stats["rank"]["value"].as_f64().map(|v| v.max(0.0).round() as u32) } else { None },
        peak_tier: tier_from_icon(&stats["peakRank"]),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn rank(tier: u32, rr: u32) -> Value {
        json!({ "value": rr, "metadata": { "iconUrl": format!("https://trackercdn.com/cdn/tracker.gg/valorant/icons/tiersv2/{tier}.png") } })
    }

    fn season(act: &str, tier: u32, rr: u32, peak: u32) -> Value {
        json!({ "type": "season", "attributes": { "seasonId": act, "playlist": "competitive" },
                "stats": { "rank": rank(tier, rr), "peakRank": rank(peak, 0) } })
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
            season("A6", 24, 23, 25)
        ]}});
        let p = parse_profile(&v, "a6");
        assert_eq!((p.tier, p.rr, p.peak_tier), (24, Some(23), 25));
        let mut hidden = v.clone();
        hidden["data"]["segments"][1]["stats"]["rank"]["value"] = Value::Null;
        assert_eq!(parse_profile(&hidden, "a6").rr, None, "tracker.gg often leaves RR out; it isn't 0");
    }

    #[test]
    fn no_games_this_act_is_unranked_but_keeps_peak() {
        let v = json!({ "data": { "segments": [season("a5", 20, 50, 21)] } });
        let p = parse_profile(&v, "a6");
        assert_eq!((p.tier, p.rr, p.peak_tier), (0, None, 21));
        assert_eq!(parse_profile(&json!({ "data": { "segments": [] } }), "a6"), Profile::default());
    }

    pub(crate) fn answer(status: u16, body: &str) -> Response {
        Response { status, body: body.into() }
    }
}
