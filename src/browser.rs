//! A hidden WebView2, the Edge engine built into Windows, that sits on a tracker.gg page and calls
//! tracker.gg's API from it the way the site's own scripts do. Cloudflare sees a real browser: a page
//! load lets it run its checks, and the cookies they earn are kept in the app's own profile.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::Foundation::{E_POINTER, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};

/// The page the browser sits on: a tracker.gg page with no scripts or ads, so the browser stays small.
/// Requests sent from it carry tracker.gg's origin and cookies, as the site's own do.
pub const HOME: &str = "https://tracker.gg/robots.txt";
/// The hidden window never draws, so the GPU process (about 40 MB) isn't needed.
const ARGS: &str = "--disable-gpu";
/// `Browser::page` while the last page load wasn't refused.
const PAGE_OK: i32 = -1;

/// A finished request. `status` is 0 when the page couldn't read the response, as with Cloudflare's
/// block page, which carries no CORS headers, or when the page itself was refused; `body` then says why.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

enum Job {
    Get(Pending),
    Reload,
}

pub struct Browser {
    jobs: mpsc::Sender<Job>,
    thread: u32,
    timeout: Duration,
    /// The HTTP status of the last page load if it was refused (0 without one), otherwise `PAGE_OK`.
    page: Arc<AtomicI32>,
}

impl Browser {
    /// Starts the browser on its own thread with its profile under `data_dir`. It opens `home` with
    /// the first request; a request gets `timeout` to answer, page load included.
    pub fn start(data_dir: &Path, home: &str, timeout: Duration) -> Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| format!("couldn't create {}", data_dir.display()))?;
        let data_dir = HSTRING::from(data_dir.as_os_str());
        let home = HSTRING::from(home);
        let page = Arc::new(AtomicI32::new(PAGE_OK));
        let (jobs, rx) = mpsc::channel();
        let (started, started_rx) = mpsc::channel();
        let page_status = page.clone();
        std::thread::Builder::new().name("tracker-browser".into()).spawn(move || {
            if let Err(e) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
                let _ = started.send(Err(e.into()));
                return;
            }
            match Window::open(&data_dir, home, page_status) {
                Ok(window) => {
                    let _ = started.send(Ok(unsafe { GetCurrentThreadId() }));
                    window.run(rx);
                }
                Err(e) => {
                    let _ = started.send(Err(e));
                }
            }
            unsafe { CoUninitialize() };
        })?;
        let thread = started_rx.recv().context("the browser thread stopped")??;
        Ok(Self { jobs, thread, timeout, page })
    }

    /// GET `url` from the page, as its scripts would.
    #[cfg(test)]
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.get_trimmed(url, None).await
    }

    /// GET `url` from the page. `trim`, a JavaScript function, cuts a successful JSON body down in the
    /// page, so a large response never reaches the app whole.
    pub async fn get_trimmed(&self, url: &str, trim: Option<&'static str>) -> Result<Response> {
        let (reply, rx) = oneshot::channel();
        self.post(Job::Get(Pending { url: url.into(), trim, reply }))?;
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => bail!("the browser closed during the request"),
            // The page never loaded because the site refused it: answer as a refusal, not a stuck browser.
            Err(_) => match self.page.load(Ordering::Relaxed) {
                PAGE_OK => bail!("tracker.gg didn't answer within {} s", self.timeout.as_secs()),
                0 => Ok(Response { status: 0, retry_after: None, body: "the tracker.gg page couldn't load".into() }),
                status => Ok(Response { status: 0, retry_after: None, body: format!("the tracker.gg page didn't load (HTTP {status})") }),
            },
        }
    }

    /// Loads the page again. Requests in flight are sent again from the new page.
    pub fn reload(&self) -> Result<()> {
        self.post(Job::Reload)
    }

    fn post(&self, job: Job) -> Result<()> {
        self.jobs.send(job).map_err(|_| anyhow!("the browser has stopped"))?;
        unsafe { PostThreadMessageW(self.thread, WM_APP, WPARAM(0), LPARAM(0)) }.context("couldn't reach the browser")
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

struct Pending {
    url: String,
    trim: Option<&'static str>,
    reply: oneshot::Sender<Response>,
}

#[derive(Default)]
struct State {
    /// The page has loaded, so requests can go out.
    ready: bool,
    loading: bool,
    waiting: Vec<Pending>,
    sent: HashMap<u64, Pending>,
    next_id: u64,
    /// Retry-After matters only on a 429 or 503, and tracker.gg doesn't expose it to the page, so it's
    /// read from the full response, whose event can come before or after the page's answer. Whichever
    /// comes first waits here for the other, by URL.
    headers_seen: HashMap<String, Option<String>>,
    awaiting_headers: HashMap<String, (oneshot::Sender<Response>, Response)>,
}

/// The browser thread's side: a hidden window holding the WebView. Everything here runs on that
/// thread, as WebView2 requires.
struct Window {
    hwnd: HWND,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    home: HSTRING,
    state: Rc<RefCell<State>>,
}

impl Window {
    fn open(data_dir: &HSTRING, home: HSTRING, page: Arc<AtomicI32>) -> Result<Self> {
        let hwnd = hidden_window()?;
        let environment = {
            let (tx, rx) = mpsc::channel();
            let data_dir = data_dir.clone();
            let options = CoreWebView2EnvironmentOptions::default();
            unsafe { options.set_additional_browser_arguments(ARGS.into()) };
            let options: ICoreWebView2EnvironmentOptions = options.into();
            CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| unsafe {
                    CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &data_dir, &options, &handler)
                        .map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |result, environment| {
                    result?;
                    let _ = tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                    Ok(())
                }),
            )
            .map_err(|e| anyhow!("{e}"))
            .context("couldn't start WebView2; is the Microsoft Edge WebView2 Runtime installed?")?;
            rx.recv()??
        };
        let controller = {
            let (tx, rx) = mpsc::channel();
            CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| unsafe {
                    environment.CreateCoreWebView2Controller(hwnd, &handler).map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |result, controller| {
                    result?;
                    let _ = tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                    Ok(())
                }),
            )
            .map_err(|e| anyhow!("{e}"))
            .context("couldn't create the WebView2 browser")?;
            rx.recv()??
        };
        let webview = unsafe {
            controller.SetBounds(RECT { left: 0, top: 0, right: 1280, bottom: 800 })?;
            let webview = controller.CoreWebView2()?;
            let settings = webview.Settings()?;
            settings.SetAreDevToolsEnabled(false)?;
            settings.SetAreDefaultScriptDialogsEnabled(false)?;
            webview
        };
        let window = Self { hwnd, controller, webview, home, state: Rc::default() };
        window.listen(page)?;
        Ok(window)
    }

    fn listen(&self, page: Arc<AtomicI32>) -> Result<()> {
        let mut token = 0i64;
        let state = self.state.clone();
        let starting = NavigationStartingEventHandler::create(Box::new(move |_, _| {
            let mut s = state.borrow_mut();
            s.ready = false;
            s.loading = true;
            // Requests sent from the old page will never answer; they go out again from the new one.
            // A late answer from the old page carries an id that's gone and is ignored.
            let sent: Vec<Pending> = s.sent.drain().map(|(_, p)| p).collect();
            s.waiting.extend(sent);
            s.headers_seen.clear();
            Ok(())
        }));
        let state = self.state.clone();
        let completed = NavigationCompletedEventHandler::create(Box::new(move |webview, args| {
            let status = args.as_ref().and_then(|a| a.cast::<ICoreWebView2NavigationCompletedEventArgs2>().ok()).map(|a| {
                let mut status = 0;
                let _ = unsafe { a.HttpStatusCode(&mut status) };
                status
            });
            let success = args.is_some_and(|a| {
                let mut ok = Default::default();
                unsafe { a.IsSuccess(&mut ok) }.is_ok() && ok.as_bool()
            });
            let ok = success && status.is_none_or(|s| s < 400);
            page.store(if ok { PAGE_OK } else { status.unwrap_or(0).max(0) }, Ordering::Relaxed);
            // After a refused load, waiting requests stay queued: a Cloudflare check page is refused,
            // then loads the real page once it passes. Otherwise they time out as refused.
            let waiting = {
                let mut s = state.borrow_mut();
                s.loading = false;
                s.ready = ok;
                if ok { std::mem::take(&mut s.waiting) } else { Vec::new() }
            };
            if let Some(webview) = webview {
                for pending in waiting {
                    send(&webview, &state, pending);
                }
            }
            Ok(())
        }));
        let state = self.state.clone();
        let response = WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let (url, status, retry_after) = unsafe {
                let (mut url, mut status, mut value) = (PWSTR::null(), 0, PWSTR::null());
                args.Request()?.Uri(&mut url)?;
                let response = args.Response()?;
                response.StatusCode(&mut status)?;
                let retry_after =
                    response.Headers()?.GetHeader(w!("retry-after"), &mut value).ok().map(|_| CoTaskMemPWSTR::from(value).to_string());
                (CoTaskMemPWSTR::from(url).to_string(), status, retry_after)
            };
            if status != 429 && status != 503 {
                return Ok(());
            }
            let mut s = state.borrow_mut();
            match s.awaiting_headers.remove(&url) {
                Some((reply, response)) => {
                    let _ = reply.send(Response { retry_after, ..response });
                }
                None if s.sent.values().any(|p| p.url == url) => {
                    s.headers_seen.insert(url, retry_after);
                }
                None => {}
            }
            Ok(())
        }));
        let state = self.state.clone();
        let message = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut json = PWSTR::null();
            unsafe { args.WebMessageAsJson(&mut json) }?;
            let json = CoTaskMemPWSTR::from(json).to_string();
            let Ok(v) = serde_json::from_str::<Value>(&json) else { return Ok(()) };
            let mut s = state.borrow_mut();
            let Some(Pending { url, reply, .. }) = v["id"].as_u64().and_then(|id| s.sent.remove(&id)) else { return Ok(()) };
            let response = Response {
                status: v["status"].as_u64().unwrap_or(0) as u16,
                retry_after: None,
                body: v["body"].as_str().unwrap_or("").to_string(),
            };
            // Every answer takes its URL's headers, so none are left for a later request.
            let seen = s.headers_seen.remove(&url);
            if response.status != 429 && response.status != 503 {
                let _ = reply.send(response);
                return Ok(());
            }
            match seen {
                Some(retry_after) => {
                    let _ = reply.send(Response { retry_after, ..response });
                }
                None => {
                    // Callers never fetch one URL twice at once; if one did, the earlier answer goes
                    // out without its header rather than being lost.
                    if let Some((earlier, earlier_response)) = s.awaiting_headers.insert(url, (reply, response)) {
                        let _ = earlier.send(earlier_response);
                    }
                }
            }
            Ok(())
        }));
        unsafe {
            self.webview.add_NavigationStarting(&starting, &mut token)?;
            self.webview.add_NavigationCompleted(&completed, &mut token)?;
            self.webview.cast::<ICoreWebView2_2>()?.add_WebResourceResponseReceived(&response, &mut token)?;
            self.webview.add_WebMessageReceived(&message, &mut token)?;
        }
        Ok(())
    }

    fn run(self, jobs: mpsc::Receiver<Job>) {
        let mut msg = MSG::default();
        // GetMessageW returns 0 for WM_QUIT and -1 on error.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
            if msg.hwnd.is_invalid() && msg.message == WM_APP {
                for job in jobs.try_iter() {
                    self.handle(job);
                }
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn handle(&self, job: Job) {
        let load = match job {
            Job::Get(pending) => {
                if self.state.borrow().ready {
                    send(&self.webview, &self.state, pending);
                    return;
                }
                let mut s = self.state.borrow_mut();
                s.waiting.push(pending);
                !std::mem::replace(&mut s.loading, true)
            }
            Job::Reload => {
                let mut s = self.state.borrow_mut();
                s.ready = false;
                !std::mem::replace(&mut s.loading, true)
            }
        };
        if load && unsafe { self.webview.Navigate(&self.home) }.is_err() {
            self.state.borrow_mut().loading = false;
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = self.controller.Close();
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Runs `fetch` on the page; its result comes back through `postMessage`.
fn send(webview: &ICoreWebView2, state: &Rc<RefCell<State>>, pending: Pending) {
    // The caller gave up waiting; don't send a request nobody reads.
    if pending.reply.is_closed() {
        return;
    }
    let id = {
        let mut s = state.borrow_mut();
        s.next_id += 1;
        s.next_id
    };
    let script = format!(
        r#"(async () => {{
  const post = (m) => window.chrome.webview.postMessage(Object.assign({{ id: {id} }}, m));
  try {{
    const r = await fetch({url}, {{ credentials: "include", headers: {{ accept: "application/json, text/plain, */*" }} }});
    const trim = {trim};
    let body = await r.text();
    // A body the trim can't read is passed on as the error text, not as a failed request, which
    // would look like a block.
    if (r.ok && trim) {{
      try {{ body = JSON.stringify(trim(JSON.parse(body))); }} catch (e) {{ body = "unreadable: " + String(e); }}
    }}
    post({{ status: r.status, body }});
  }} catch (e) {{
    post({{ status: 0, body: String(e) }});
  }}
}})();"#,
        url = Value::from(pending.url.as_str()),
        trim = pending.trim.unwrap_or("null"),
    );
    state.borrow_mut().sent.insert(id, pending);
    let done = ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
    if unsafe { webview.ExecuteScript(&HSTRING::from(script), &done) }.is_err() {
        state.borrow_mut().sent.remove(&id);
    }
}

fn hidden_window() -> Result<HWND> {
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class =
            WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: w!("teamdiff-browser"), ..Default::default() };
        // Fails harmlessly when an earlier browser already registered the class.
        RegisterClassW(&class);
        // Never shown: the page only needs a window to live in.
        Ok(CreateWindowExW(
            Default::default(),
            w!("teamdiff-browser"),
            w!("teamdiff"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280,
            800,
            None,
            None,
            Some(instance.into()),
            None,
        )?)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A finished match as tracker.gg's `/matches/{id}` returns it, cut to what the trim reads.
    const MATCH: &str = r#"{"data":{"segments":[
        {"type":"player-summary","attributes":{"platformUserIdentifier":"A#1"},"metadata":{"teamId":"Red","partyId":"1","agentName":"Jett"}},
        {"type":"player-summary","attributes":{"platformUserIdentifier":"B#2"},"metadata":{"teamId":"Red","partyId":"1"}},
        {"type":"player-summary","attributes":{"platformUserIdentifier":"C#3"},"metadata":{"teamId":"Blue","partyId":"1"}},
        {"type":"round-summary","attributes":{},"metadata":{}}
    ]}}"#;

    /// A local site on 127.0.0.1 whose API is called from `localhost`, a different origin, the way
    /// tracker.gg's page calls api.tracker.gg: CORS applies and Retry-After isn't exposed to the page.
    /// Returns (page origin, API origin).
    pub(crate) async fn site() -> (String, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let page = format!("http://127.0.0.1:{port}");
        let cors = format!("Access-Control-Allow-Origin: {page}\r\nAccess-Control-Allow-Credentials: true\r\n");
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let cors = cors.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.ends_with(b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let path = String::from_utf8_lossy(&request).split(' ').nth(1).unwrap_or("").to_string();
                    if path == "/slow" {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                    let (status, extra, body) = match path.as_str() {
                        "/" => ("200 OK", "Content-Type: text/html\r\n".to_string(), "<p>home</p>"),
                        "/data" | "/slow" => ("200 OK", format!("Content-Type: application/json\r\n{cors}"), r#"{"ok":1}"#),
                        p if p.starts_with("/busy") => ("503 Service Unavailable", format!("Retry-After: 7\r\n{cors}"), ""),
                        "/blocked" => ("403 Forbidden", "Content-Type: text/html\r\n".to_string(), "<p>blocked</p>"),
                        "/match" => ("200 OK", format!("Content-Type: application/json\r\n{cors}"), MATCH),
                        "/empty" => ("200 OK", format!("Content-Type: application/json\r\n{cors}"), "{}"),
                        "/html" => ("200 OK", format!("Content-Type: text/html\r\n{cors}"), "<p>not json</p>"),
                        _ => ("404 Not Found", cors, ""),
                    };
                    let response = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        (page, format!("http://localhost:{port}"))
    }

    /// A fresh profile folder, removed once the browser's processes let go of it. WebView2 can take
    /// longer than a request's timeout to start when several start at once, so tests that use a
    /// profile take turns.
    pub(crate) struct Profile(pub std::path::PathBuf, Option<std::sync::MutexGuard<'static, ()>>);

    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl Profile {
        pub(crate) fn new(name: &str) -> Self {
            let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
            Self(std::env::temp_dir().join(format!("teamdiff-{name}-{}", std::process::id())), Some(turn))
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            // The next test can start its browser while this one's files unlock.
            self.1.take();
            // WebView2's processes can hold the profile's files for a while after it closes.
            for _ in 0..300 {
                if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_reports_status_retry_after_and_body() {
        let (page, api) = site().await;
        let profile = Profile::new("browser-test");
        let b = Browser::start(&profile.0, &format!("{page}/"), Duration::from_secs(20)).unwrap();
        let ok = b.get(&format!("{api}/data")).await.unwrap();
        assert_eq!(ok, Response { status: 200, retry_after: None, body: r#"{"ok":1}"#.into() });
        // The full response and the page's answer arrive in either order.
        // Every other URL has the shape of the app's: an escaped `#` and an empty query.
        for i in 0..20 {
            let busy = b.get(&format!("{api}{}", if i % 2 == 0 { "/busy" } else { "/busy/a%23b?" })).await.unwrap();
            assert_eq!((busy.status, busy.retry_after.as_deref()), (503, Some("7")), "read from the full response, not the page");
        }
        assert_eq!(b.get(&format!("{api}/nope")).await.unwrap().status, 404);
        let trim = Some("(v) => ({ n: v.ok + 1 })");
        assert_eq!(b.get_trimmed(&format!("{api}/data"), trim).await.unwrap().body, r#"{"n":2}"#, "cut down in the page");
        let nope = b.get_trimmed(&format!("{api}/nope"), trim).await.unwrap();
        assert_eq!((nope.status, nope.body.as_str()), (404, ""), "an error body isn't trimmed");
        let html = b.get_trimmed(&format!("{api}/html"), trim).await.unwrap();
        assert_eq!(html.status, 200, "a body the trim can't read isn't a failed request: {html:?}");
        assert!(html.body.starts_with("unreadable"), "{html:?}");
        let blocked = b.get(&format!("{api}/blocked")).await.unwrap();
        assert_eq!(blocked.status, 0, "a block page without CORS headers can't be read: {blocked:?}");
        drop(b);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_match_trim_keeps_players_teams_and_parties() {
        let (page, api) = site().await;
        let profile = Profile::new("trim-test");
        let b = Browser::start(&profile.0, &format!("{page}/"), Duration::from_secs(20)).unwrap();
        let trim = Some(crate::tracker::MATCH_PLAYERS);
        let r = b.get_trimmed(&format!("{api}/match"), trim).await.unwrap();
        assert!(!r.body.contains("agentName") && !r.body.contains("round-summary"), "{}", r.body);
        let v: Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(crate::tracker::parse_match_parties(&v), [vec!["a#1".to_string(), "b#2".to_string()]]);
        let empty = b.get_trimmed(&format!("{api}/empty"), trim).await.unwrap();
        let v: Value = serde_json::from_str(&empty.body).unwrap();
        assert!(crate::tracker::parse_match_parties(&v).is_empty(), "an unexpected shape reads as no parties: {}", empty.body);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_sends_requests_in_flight_again() {
        let (page, api) = site().await;
        let profile = Profile::new("reload-test");
        let b = Arc::new(Browser::start(&profile.0, &format!("{page}/"), Duration::from_secs(20)).unwrap());
        assert_eq!(b.get(&format!("{api}/data")).await.unwrap().status, 200);
        let slow = tokio::spawn({
            let (b, url) = (b.clone(), format!("{api}/slow"));
            async move { b.get(&url).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        b.reload().unwrap();
        assert_eq!(slow.await.unwrap().unwrap().status, 200, "sent again from the reloaded page");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_page_answers_as_a_refusal() {
        let (page, api) = site().await;
        let profile = Profile::new("refused-test");
        let b = Browser::start(&profile.0, &format!("{page}/blocked"), Duration::from_secs(20)).unwrap();
        let r = b.get(&format!("{api}/data")).await.unwrap();
        assert_eq!(r.status, 0);
        assert!(r.body.contains("HTTP 403"), "{r:?}");
    }
}
