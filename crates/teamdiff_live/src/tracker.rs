//! Player stats from tracker.gg, fetched by its own web page in a hidden browser (see
//! `teamdiff_browser`).
//!
//! tracker.gg has no public Valorant API. Its staff allowed this app to call the endpoints its site
//! uses, as a browser would, until they offer one. `teamdiff_core::tracker` reads the answers.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use teamdiff_browser::{Browser, HOME, Response};
use teamdiff_core::net_stats::NetStats;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio::task;
use tokio::time::{self, Instant};

use crate::{mutex, riot};

const API: &str = "https://api.tracker.gg/api/v2/valorant/standard";

/// How long requests pause after tracker.gg refuses one even from a freshly loaded page.
const BLOCK_PAUSE: Duration = Duration::from_secs(300);
/// The longest `Retry-After` waited out inside a request; longer ones pause every request instead.
const MAX_INLINE_WAIT: u64 = 30;
/// The longest `Retry-After` read, a day, so an absurd value can't overflow the deadline.
const MAX_RETRY_AFTER: u64 = 86_400;
/// Cuts a `/matches/{id}` response down to each player's Riot ID, team and party.
pub(crate) const MATCH_PLAYERS: &str = r#"(v) => ({ data: { segments: (v?.data?.segments ?? []).filter((s) => s?.type === "player-summary").map((s) => ({ type: s.type, attributes: { platformUserIdentifier: s.attributes?.platformUserIdentifier }, metadata: { teamId: s.metadata?.teamId, partyId: s.metadata?.partyId } })) } })"#;
/// How long the browser stays open without a request.
const IDLE: Duration = Duration::from_secs(120);
/// How long the browser may take to start, and a request to answer, page load included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A refusal for a player who hasn't made their tracker.gg profile public by signing in there,
/// which tracker.gg answers with 451.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub(crate) struct Private;

impl Display for Private {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("this tracker.gg profile is private")
    }
}

impl Error for Private {}

/// Where tracker.gg requests go. The app sends them from the browser, [`BrowserTransport`]; the
/// tests answer them from a list, so the retry and pause logic of [`Tracker`] runs on canned
/// answers without a browser.
pub(crate) trait Transport {
    /// GETs `url`, with the body cut down in the page by the JavaScript function `trim`.
    async fn get(&self, url: &str, trim: Option<&'static str>) -> Result<Response>;

    /// Loads the page requests are sent from again, which can lift a Cloudflare block.
    async fn reload(&self) -> Result<()>;

    /// Lets go of what the transport holds open once it has gone unused for a while.
    fn close_if_idle(&self);
}

/// The browser while it is open, and when a request last took it.
#[derive(Debug)]
struct OpenBrowser {
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "each request holds the browser until it answers, so the idle check can \
                      tell from the count whether one is still running"
        )
    )]
    browser: Arc<Browser>,
    used: Instant,
}

/// The browser, started by the first request and closed after [`IDLE`] without one, so its
/// memory (about 100 MB) is only used while stats load.
#[derive(Debug)]
pub(crate) struct BrowserTransport {
    profile_dir: PathBuf,
    home: String,
    open: AsyncMutex<Option<OpenBrowser>>,
}

impl Transport for BrowserTransport {
    async fn get(&self, url: &str, trim: Option<&'static str>) -> Result<Response> {
        let browser = {
            let mut open = self.open.lock().await;
            let browser = if let Some(open) = open.take() {
                open.browser
            } else {
                let (dir, home) = (self.profile_dir.clone(), self.home.clone());
                let start =
                    task::spawn_blocking(move || Browser::start(&dir, &home, REQUEST_TIMEOUT));
                let started = time::timeout(REQUEST_TIMEOUT, start).await;
                Arc::new(started.map_err(|_| {
                    anyhow!(
                        "WebView2 didn't start within {} s",
                        REQUEST_TIMEOUT.as_secs()
                    )
                })???)
            };
            *open = Some(OpenBrowser {
                browser: Arc::clone(&browser),
                used: Instant::now(),
            });
            browser
        };
        let result = browser.get_trimmed(url, trim).await;
        if result.is_err() {
            let mut open = self.open.lock().await;
            if open
                .as_ref()
                .is_some_and(|o| Arc::ptr_eq(&o.browser, &browser))
            {
                *open = None;
            }
        }
        result
    }

    async fn reload(&self) -> Result<()> {
        match self.open.lock().await.as_ref() {
            Some(open) => open.browser.reload(),
            None => Ok(()),
        }
    }

    fn close_if_idle(&self) {
        let Ok(mut open) = self.open.try_lock() else {
            return;
        };
        if open
            .as_ref()
            .is_some_and(|o| o.used.elapsed() > IDLE && Arc::strong_count(&o.browser) == 1)
        {
            *open = None;
        }
    }
}

/// Why and until when no request is sent.
#[derive(Debug, Clone, Copy)]
struct Pause {
    until: Instant,
    why: &'static str,
}

/// The API of tracker.gg, two requests at a time, with its waits and blocks handled.
#[derive(Debug)]
pub(crate) struct Tracker<T = BrowserTransport> {
    transport: T,
    permits: Semaphore,
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the Riot client counts into the same counters, and the model hands them to \
                      the footer, each for as long as it lives"
        )
    )]
    net: Arc<NetStats>,
    dump_dir: Option<PathBuf>,
    /// No request is sent until then: after tracker.gg refuses one twice, so a block isn't met
    /// with a retry every few seconds, or when it asked for a wait too long to hold a request slot
    /// through.
    paused: Mutex<Option<Pause>>,
}

impl Tracker {
    /// A tracker that sends its requests from a browser whose cookies are kept in `profile_dir`
    /// between runs, counting them in `net` and saving their answers under `dump_dir`.
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the counters come from the Riot client, which counts into them too"
        )
    )]
    pub(crate) fn new(net: Arc<NetStats>, dump_dir: Option<PathBuf>, profile_dir: PathBuf) -> Self {
        let transport = BrowserTransport {
            profile_dir,
            home: HOME.into(),
            open: AsyncMutex::default(),
        };
        Self::with(transport, net, dump_dir)
    }
}

impl<T: Transport> Tracker<T> {
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the counters come from the Riot client, which counts into them too"
        )
    )]
    fn with(transport: T, net: Arc<NetStats>, dump_dir: Option<PathBuf>) -> Self {
        Self {
            transport,
            permits: Semaphore::new(2),
            net,
            dump_dir,
            paused: Mutex::default(),
        }
    }

    /// Closes the browser once it has gone [`IDLE`] without a request.
    pub(crate) fn close_if_idle(&self) {
        self.transport.close_if_idle();
    }

    /// Pauses requests for `wait`, unless an existing pause already lasts longer.
    fn pause(&self, wait: Duration, why: &'static str) {
        let until = Instant::now() + wait;
        let mut paused = mutex::lock(&self.paused);
        if paused.is_none_or(|p| until > p.until) {
            *paused = Some(Pause { until, why });
        }
    }

    /// GET on tracker.gg's API. `Ok(None)` means 404: no such player, or no data of that kind.
    async fn get(&self, path: &str) -> Result<Option<Value>> {
        self.get_trimmed(path, None).await
    }

    /// `get`, with the body cut down in the page by the JavaScript function `trim`. A trimmed
    /// answer is never dumped: trimming is how match pages, which name incognito players, are
    /// asked for.
    async fn get_trimmed(&self, path: &str, trim: Option<&'static str>) -> Result<Option<Value>> {
        let _permit = self.permits.acquire().await?;
        let paused = *mutex::lock(&self.paused);
        if let Some(Pause { until, why }) = paused
            && until > Instant::now()
        {
            bail!(
                "{why}; trying again in {} min",
                until
                    .saturating_duration_since(Instant::now())
                    .as_secs()
                    .div_ceil(60)
            );
        }
        self.net.started.fetch_add(1, Ordering::Relaxed);
        let result = self.get_with_retry(path, trim).await;
        self.net.finished.fetch_add(1, Ordering::Relaxed);
        if let (Ok(Some(v)), None) = (&result, trim) {
            riot::dump(
                self.dump_dir.as_deref(),
                &format!("tracker{}", path.split('?').next().unwrap_or(path)),
                v,
            );
        }
        result
    }

    /// Sends the request, waiting out a short `Retry-After` and reloading the page once after a
    /// refusal.
    ///
    /// Only the seconds form of `Retry-After` is read; an HTTP date counts as no header. It is
    /// capped at a day, so an absurd value can't overflow the deadline. A 503 is retried here only
    /// when the server says when; otherwise the caller retries later. Cloudflare's block page is a
    /// 403, or status 0 when the page can't read it because it carries no CORS headers; loading
    /// the page again lets Cloudflare check the browser, which can lift the block.
    async fn get_with_retry(
        &self,
        path: &str,
        trim: Option<&'static str>,
    ) -> Result<Option<Value>> {
        use anyhow::Context as _;

        let label = path.split('?').next().unwrap_or(path);
        let mut reloaded = false;
        for attempt in 0..5u64 {
            let resp = self
                .transport
                .get(&format!("{API}{path}"), trim)
                .await
                .with_context(|| format!("tracker.gg request failed for {label}"))?;
            let status = resp.status;
            if status == 404 {
                return Ok(None);
            }
            let retry_after: Option<u64> = resp
                .retry_after
                .as_deref()
                .and_then(|s| s.trim().parse().ok())
                .map(|s: u64| s.min(MAX_RETRY_AFTER));
            if status == 429 || (status == 503 && retry_after.is_some()) {
                let wait = retry_after.unwrap_or(5 * (attempt + 1));
                let until = Instant::now() + Duration::from_secs(wait);
                *mutex::lock(&self.net.rate_limited_until) = Some(until.into_std());
                if wait > MAX_INLINE_WAIT {
                    self.pause(Duration::from_secs(wait), "tracker.gg asked to wait");
                    bail!("tracker.gg asked to wait {wait} s ({status}) on {label}");
                }
                time::sleep_until(until).await;
                continue;
            }
            if status == 403 || status == 0 {
                if !reloaded {
                    reloaded = true;
                    self.transport.reload().await?;
                    continue;
                }
                self.pause(BLOCK_PAUSE, "tracker.gg refused a request");
                bail!(
                    "tracker.gg refused the request ({}) for {label}",
                    if status == 0 {
                        resp.body.as_str()
                    } else {
                        "403"
                    }
                );
            }
            if status == 451 {
                return Err(Private.into());
            }
            if !(200..300).contains(&status) {
                bail!("{status} from tracker.gg {label}");
            }
            return Ok(Some(
                serde_json::from_str(&resp.body)
                    .with_context(|| format!("tracker.gg {label} isn't JSON"))?,
            ));
        }
        bail!("tracker.gg still asks to wait (429/503) on {label}")
    }

    /// A player's profile: rank, peak and latest act.
    pub(crate) async fn profile(&self, riot_id: &str) -> Result<Option<Value>> {
        self.get(&format!("/profile/riot/{riot_id}?")).await
    }

    /// A player's rank and record in one act.
    pub(crate) async fn season(&self, riot_id: &str, act_id: &str) -> Result<Option<Value>> {
        self.get(&format!(
            "/profile/riot/{riot_id}/segments/season?playlist=competitive&seasonId={act_id}&source=web"
        ))
        .await
    }

    /// A player's record per map in one act.
    pub(crate) async fn maps(&self, riot_id: &str, act_id: &str) -> Result<Option<Value>> {
        self.get(&format!(
            "/profile/riot/{riot_id}/segments/map?playlist=competitive&seasonId={act_id}&source=web"
        ))
        .await
    }

    /// A player's recent competitive matches.
    pub(crate) async fn matches(&self, riot_id: &str) -> Result<Option<Value>> {
        self.get(&format!(
            "/matches/riot/{riot_id}?platform=pc&type=competitive"
        ))
        .await
    }

    /// Who was in one finished match, and their teams and parties. The full response is about 2 MB of
    /// round-by-round data, so the page keeps only the players. It names incognito players too; callers
    /// only compare it against names they already have.
    pub(crate) async fn match_players(&self, match_id: &str) -> Result<Option<Value>> {
        self.get_trimmed(&format!("/matches/{match_id}"), Some(MATCH_PLAYERS))
            .await
    }
}

/// `name#tag` as it appears in tracker.gg's URLs: every byte outside the unreserved set is
/// percent-encoded, so `#` becomes `%23`.
pub(crate) fn riot_id_path(name: &str, tag: &str) -> String {
    format!("{name}#{tag}")
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::future;
    use std::sync::atomic::AtomicUsize;

    use serde_json::json;
    use teamdiff_browser::test_support::{Profile, Site};
    use teamdiff_core::tracker as parse;
    use tokio::runtime::Builder;

    use super::*;

    /// A transport that answers each request with the next of its responses, and counts the
    /// requests and reloads.
    #[derive(Debug)]
    pub(crate) struct Canned {
        responses: Mutex<VecDeque<Response>>,
        sent: AtomicUsize,
        reloads: AtomicUsize,
    }

    impl Canned {
        /// A tracker that answers each request with the next of `responses`, and panics on a
        /// request past the last.
        pub(crate) fn tracker(responses: Vec<Response>) -> Tracker<Canned> {
            let transport = Canned {
                responses: Mutex::new(responses.into()),
                sent: AtomicUsize::default(),
                reloads: AtomicUsize::default(),
            };
            Tracker::with(transport, Arc::default(), None)
        }

        fn sent(&self) -> usize {
            self.sent.load(Ordering::Relaxed)
        }

        fn reloads(&self) -> usize {
            self.reloads.load(Ordering::Relaxed)
        }
    }

    impl Transport for Canned {
        fn get(
            &self,
            _url: &str,
            _trim: Option<&'static str>,
        ) -> impl Future<Output = Result<Response>> {
            self.sent.fetch_add(1, Ordering::Relaxed);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("a canned response for every request");
            future::ready(Ok(response))
        }

        fn reload(&self) -> impl Future<Output = Result<()>> {
            self.reloads.fetch_add(1, Ordering::Relaxed);
            future::ready(Ok(()))
        }

        fn close_if_idle(&self) {}
    }

    /// A response with `status`, `Retry-After` and `body`.
    pub(crate) fn answer(status: u16, retry_after: Option<&str>, body: &str) -> Response {
        Response {
            status,
            retry_after: retry_after.map(str::to_string),
            body: body.into(),
        }
    }

    /// Runs `test` on a runtime whose clock stands still until every task waits on it, then jumps
    /// to the next timer, so a test sees each wait without taking that long.
    fn block_on<F: Future>(test: F) -> F::Output {
        Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("the test runtime should start")
            .block_on(test)
    }

    #[test]
    fn riot_ids_are_percent_encoded() {
        assert_eq!(riot_id_path("blue berry", "TAG"), "blue%20berry%23TAG");
        assert_eq!(riot_id_path("한", "DEMO"), "%ED%95%9C%23DEMO");
        assert_eq!(riot_id_path("a.b_c~d-e", "1"), "a.b_c~d-e%231");
    }

    #[test]
    fn a_503_with_retry_after_waits_then_retries() {
        block_on(async {
            let t = Canned::tracker(vec![
                answer(503, Some("1"), ""),
                answer(200, None, r#"{"data":{}}"#),
            ]);
            let start = Instant::now();
            let v = t.get("/x").await.unwrap();
            assert_eq!(v, Some(json!({ "data": {} })));
            assert_eq!(t.transport.sent(), 2);
            let waited = start.elapsed();
            assert!(
                (Duration::from_secs(1)..Duration::from_secs(2)).contains(&waited),
                "waited for Retry-After, and only once: {waited:?}"
            );
        });
    }

    #[test]
    fn a_503_without_a_seconds_retry_after_fails_for_the_caller_to_retry() {
        block_on(async {
            for retry_after in [None, Some("Wed, 21 Oct 2026 07:28:00 GMT")] {
                let t = Canned::tracker(vec![answer(503, retry_after, "")]);
                let err = t.get("/x").await.unwrap_err();
                assert!(err.to_string().contains("503"), "{err:#}");
                assert_eq!(
                    t.transport.sent(),
                    1,
                    "no retry inside the request: {retry_after:?}"
                );
            }
        });
    }

    #[test]
    fn a_long_retry_after_pauses_every_request_instead_of_holding_a_slot() {
        block_on(async {
            for status in [429, 503] {
                let t = Canned::tracker(vec![answer(status, Some("3600"), "")]);
                let start = Instant::now();
                assert!(
                    t.get("/x")
                        .await
                        .unwrap_err()
                        .to_string()
                        .contains("3600 s"),
                    "{status}"
                );
                let err = t.get("/y").await.unwrap_err();
                assert!(
                    err.to_string().contains("trying again in 60 min"),
                    "{status}: {err:#}"
                );
                assert_eq!(
                    start.elapsed(),
                    Duration::ZERO,
                    "neither call waited: {status}"
                );
                assert_eq!(
                    t.transport.sent(),
                    1,
                    "the paused call sent nothing: {status}"
                );
            }
        });
    }

    #[test]
    fn an_absurd_retry_after_is_capped_instead_of_overflowing() {
        block_on(async {
            let t = Canned::tracker(vec![answer(503, Some("18446744073709551615"), "")]);
            let err = t.get("/x").await.unwrap_err();
            assert!(err.to_string().contains("86400 s"), "{err:#}");
        });
    }

    #[test]
    fn a_refusal_reloads_the_page_and_tries_once_more() {
        block_on(async {
            for refusal in [
                answer(0, None, "TypeError: Failed to fetch"),
                answer(403, None, ""),
            ] {
                let t = Canned::tracker(vec![refusal.clone(), answer(200, None, r#"{"data":{}}"#)]);
                assert_eq!(
                    t.get("/x").await.unwrap(),
                    Some(json!({ "data": {} })),
                    "{refusal:?}"
                );
                assert_eq!(
                    (t.transport.sent(), t.transport.reloads()),
                    (2, 1),
                    "one reload, then the retry went through: {refusal:?}"
                );
            }
        });
    }

    #[test]
    fn a_refusal_after_a_reload_pauses_every_request() {
        block_on(async {
            let t = Canned::tracker(vec![
                answer(0, None, "TypeError: Failed to fetch"),
                answer(0, None, "TypeError: Failed to fetch"),
            ]);
            let err = t.get("/x").await.unwrap_err();
            assert!(err.to_string().contains("Failed to fetch"), "{err:#}");
            let err = t.get("/y").await.unwrap_err();
            assert!(err.to_string().contains("trying again in 5 min"), "{err:#}");
            assert_eq!(
                (t.transport.sent(), t.transport.reloads()),
                (2, 1),
                "one reload, and nothing sent while paused"
            );
        });
    }

    #[test]
    fn a_shorter_pause_never_cuts_a_longer_one() {
        block_on(async {
            let t = Canned::tracker(Vec::new());
            let start = Instant::now();
            t.pause(Duration::from_secs(300), "blocked");
            t.pause(Duration::from_secs(45), "asked to wait");
            let pause = t.paused.lock().unwrap().unwrap();
            assert_eq!(pause.why, "blocked");
            assert_eq!(pause.until, start + Duration::from_secs(300));
            t.pause(Duration::from_secs(600), "asked to wait");
            assert_eq!(
                t.paused.lock().unwrap().unwrap().why,
                "asked to wait",
                "a longer pause still extends it"
            );
        });
    }

    /// The tests that start `WebView2`, which `.config/nextest.toml` runs one at a time.
    mod webview2 {
        use super::*;

        /// Runs `test` on a multi-threaded runtime with the real clock, as the app runs the browser.
        fn block_on_browser<F: Future>(test: F) -> F::Output {
            Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("the test runtime should start")
                .block_on(test)
        }

        #[test]
        fn the_browser_closes_only_when_idle_and_reopens_on_demand() {
            block_on_browser(async {
                let site = Site::start().await.unwrap();
                let profile = Profile::try_new("idle-test").unwrap();
                let transport = BrowserTransport {
                    profile_dir: profile.dir().to_path_buf(),
                    home: format!("{}/", site.page),
                    open: AsyncMutex::default(),
                };
                let t = Tracker::with(transport, Arc::default(), None);
                let open = &t.transport.open;
                let data = format!("{}/data", site.api);
                assert_eq!(
                    t.transport.get(&data, None).await.unwrap().status,
                    200,
                    "the first request opens it"
                );
                let first = Arc::clone(&open.lock().await.as_ref().unwrap().browser);
                t.close_if_idle();
                assert!(open.lock().await.is_some(), "just used");
                open.lock().await.as_mut().unwrap().used -= IDLE * 2;
                t.close_if_idle();
                assert!(open.lock().await.is_some(), "a request is still holding it");
                drop(first);
                t.close_if_idle();
                assert!(open.lock().await.is_none(), "idle and unused");
                assert_eq!(
                    t.transport.get(&data, None).await.unwrap().status,
                    200,
                    "the next request opens a new one"
                );
                assert!(open.lock().await.is_some());
            });
        }

        #[test]
        fn the_match_trim_keeps_players_teams_and_parties() {
            block_on_browser(async {
                let site = Site::start().await.unwrap();
                let profile = Profile::try_new("trim-test").unwrap();
                let b = Browser::start(
                    profile.dir(),
                    &format!("{}/", site.page),
                    Duration::from_secs(20),
                )
                .unwrap();
                let trim = Some(MATCH_PLAYERS);
                let r = b
                    .get_trimmed(&format!("{}/match", site.api), trim)
                    .await
                    .unwrap();
                assert!(
                    !r.body.contains("agentName") && !r.body.contains("round-summary"),
                    "{}",
                    r.body
                );
                let v: Value = serde_json::from_str(&r.body).unwrap();
                assert_eq!(
                    parse::parse_match_parties(&v),
                    [vec!["a#1".to_string(), "b#2".to_string()]]
                );
                let empty = b
                    .get_trimmed(&format!("{}/empty", site.api), trim)
                    .await
                    .unwrap();
                let v: Value = serde_json::from_str(&empty.body).unwrap();
                assert!(
                    parse::parse_match_parties(&v).is_empty(),
                    "an unexpected shape reads as no parties: {}",
                    empty.body
                );
            });
        }
    }
}
