//! Tests of the browser against a local site, each with a throwaway profile.

use std::sync::Arc;
use std::time::Duration;

use teamdiff_browser::test_support::{Profile, Site};
use teamdiff_browser::{Browser, Response};
use tokio::runtime::Builder;
use tokio::time;

/// How long each request may take, page load included. A refused page answers only once a request
/// times out after its load finished, so the refused-page test waits the full time; on a fresh CI
/// runner the first page load took longer than 8 s.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Runs `test` on a multi-threaded runtime, as the app runs the browser's callers.
fn block_on<F: Future>(test: F) -> F::Output {
    Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the test runtime should start")
        .block_on(test)
}

/// GETs `url` from the page without a trim.
async fn get(browser: &Browser, url: &str) -> Response {
    browser
        .get_trimmed(url, None)
        .await
        .expect("the browser should answer the request")
}

#[test]
fn the_page_reports_status_retry_after_and_body() {
    block_on(async {
        let site = Site::start().await.unwrap();
        let profile = Profile::try_new("browser-test").unwrap();
        let b = Browser::start(profile.dir(), &format!("{}/", site.page), TIMEOUT).unwrap();
        let api = &site.api;
        let ok = get(&b, &format!("{api}/data")).await;
        assert_eq!(
            ok,
            Response {
                status: 200,
                retry_after: None,
                body: r#"{"ok":1}"#.into()
            },
            "a plain request reads the page's answer"
        );
        for i in 0..20 {
            let path = if i % 2 == 0 { "/busy" } else { "/busy/a%23b?" };
            let busy = get(&b, &format!("{api}{path}")).await;
            assert_eq!(
                (busy.status, busy.retry_after.as_deref()),
                (503, Some("7")),
                "read from the full response, not the page, whichever of the two arrives first, \
                 and for a URL shaped like the app's, with an escaped `#` and an empty query: \
                 {path}"
            );
        }
        assert_eq!(
            get(&b, &format!("{api}/nope")).await.status,
            404,
            "a missing path reads as its status"
        );
        let trim = Some("(v) => ({ n: v.ok + 1 })");
        assert_eq!(
            b.get_trimmed(&format!("{api}/data"), trim)
                .await
                .unwrap()
                .body,
            r#"{"n":2}"#,
            "cut down in the page"
        );
        let nope = b.get_trimmed(&format!("{api}/nope"), trim).await.unwrap();
        assert_eq!(
            (nope.status, nope.body.as_str()),
            (404, ""),
            "an error body isn't trimmed"
        );
        let html = b.get_trimmed(&format!("{api}/html"), trim).await.unwrap();
        assert_eq!(
            html.status, 200,
            "a body the trim can't read isn't a failed request: {html:?}"
        );
        assert!(html.body.starts_with("unreadable"), "{html:?}");
        let blocked = get(&b, &format!("{api}/blocked")).await;
        assert_eq!(
            blocked.status, 0,
            "a block page without CORS headers can't be read: {blocked:?}"
        );
        drop(b);
    });
}

#[test]
fn a_reload_sends_requests_in_flight_again() {
    block_on(async {
        let site = Site::start().await.unwrap();
        let profile = Profile::try_new("reload-test").unwrap();
        let b =
            Arc::new(Browser::start(profile.dir(), &format!("{}/", site.page), TIMEOUT).unwrap());
        assert_eq!(
            get(&b, &format!("{}/data", site.api)).await.status,
            200,
            "the page loads before the reload"
        );
        let slow = tokio::spawn({
            let (b, url) = (Arc::clone(&b), format!("{}/slow", site.api));
            async move { get(&b, &url).await }
        });
        time::timeout(TIMEOUT, site.slow_requests(1))
            .await
            .expect("the page should send /slow")
            .unwrap();
        b.reload().unwrap();
        assert_eq!(
            slow.await.unwrap().status,
            200,
            "the site never answers the first /slow, so only the request the reloaded page sent \
             again can answer"
        );
        time::timeout(TIMEOUT, site.slow_requests(2))
            .await
            .expect("the reloaded page should send /slow again")
            .unwrap();
    });
}

#[test]
fn a_refused_page_answers_as_a_refusal() {
    block_on(async {
        let site = Site::start().await.unwrap();
        let profile = Profile::try_new("refused-test").unwrap();
        let b = Browser::start(profile.dir(), &format!("{}/blocked", site.page), TIMEOUT).unwrap();
        let r = get(&b, &format!("{}/data", site.api)).await;
        assert_eq!(r.status, 0, "a refused page answers with no status: {r:?}");
        assert!(r.body.contains("HTTP 403"), "{r:?}");
    });
}
