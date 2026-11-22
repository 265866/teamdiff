use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver};

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::sync::oneshot::Sender;
use webview2_com::Microsoft::Web::WebView2::Win32 as WebView2;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2_2, ICoreWebView2Controller, ICoreWebView2EnvironmentOptions,
    ICoreWebView2NavigationCompletedEventArgs2,
};
use webview2_com::{
    CoTaskMemPWSTR, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ExecuteScriptCompletedHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    WebMessageReceivedEventHandler, WebResourceResponseReceivedEventHandler,
};
use windows::Win32::Foundation::{E_POINTER, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader;
use windows::Win32::UI::WindowsAndMessaging;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, MSG, WINDOW_EX_STYLE, WM_APP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{BOOL, Error, HSTRING, PCWSTR, PWSTR, w};

use crate::browser::{self, PAGE_OK};
use crate::response::Response;

/// The hidden window never draws, so the GPU process (about 40 MB) isn't needed.
const ARGS: &str = "--disable-gpu";

/// The hidden window's class name.
const CLASS: PCWSTR = w!("teamdiff-browser");
/// The width of the hidden window and its `WebView`, in pixels.
const WIDTH: i32 = 1280;
/// The height of the hidden window and its `WebView`, in pixels.
const HEIGHT: i32 = 800;

/// What the [`Browser`](crate::Browser) asks of its thread.
pub(crate) enum Job {
    Get(Pending),
    Reload,
}

/// A request waiting for its answer.
pub(crate) struct Pending {
    pub(crate) url: String,
    pub(crate) trim: Option<&'static str>,
    pub(crate) reply: Sender<Response>,
}

/// A 429 or 503 answer held until its `Retry-After` header arrives.
struct HeldAnswer {
    reply: Sender<Response>,
    response: Response,
}

#[derive(Default)]
struct State {
    /// The page has loaded, so requests can go out.
    ready: bool,
    loading: bool,
    waiting: Vec<Pending>,
    sent: HashMap<u64, Pending>,
    next_id: u64,
    /// Retry-After matters only on a 429 or 503, and tracker.gg doesn't expose it to the page, so
    /// it's read from the full response, whose event can come before or after the page's answer.
    /// Whichever comes first waits here for the other, by URL.
    headers_seen: HashMap<String, Option<String>>,
    awaiting_headers: HashMap<String, HeldAnswer>,
}

impl State {
    /// A new page load began. Requests sent from the old page will never answer, so they go out
    /// again from the new one; a late answer from the old page carries an ID that's gone and is
    /// ignored.
    fn navigation_started(&mut self) {
        self.ready = false;
        self.loading = true;
        self.waiting
            .extend(self.sent.drain().map(|(_, pending)| pending));
        self.headers_seen.clear();
    }

    /// The page load finished, `ok` if the page wasn't refused, and returns the requests to send
    /// now. After a refused load, waiting requests stay queued: a Cloudflare check page is
    /// refused, then loads the real page once it passes. Otherwise they time out as refused.
    fn navigation_completed(&mut self, ok: bool) -> Vec<Pending> {
        self.loading = false;
        self.ready = ok;
        if ok {
            mem::take(&mut self.waiting)
        } else {
            Vec::new()
        }
    }

    /// The full response of a 429 or 503 for `url` arrived with `retry_after`. It completes the
    /// page's answer if that came first, or waits for it if a request for `url` is out.
    fn retry_after_received(&mut self, url: String, retry_after: Option<String>) {
        match self.awaiting_headers.remove(&url) {
            Some(held) => answer(
                held.reply,
                Response {
                    retry_after,
                    ..held.response
                },
            ),
            None if self.sent.values().any(|pending| pending.url == url) => {
                self.headers_seen.insert(url, retry_after);
            }
            None => {}
        }
    }

    /// The page posted a request's answer. Every answer takes its URL's headers, so none are left
    /// for a later request. A 429 or 503 without them yet waits for them. Callers never fetch one
    /// URL twice at once; if one did, the earlier answer goes out without its header rather than
    /// being lost.
    fn answer_received(&mut self, message: &Value) {
        let Some(Pending { url, reply, .. }) =
            message["id"].as_u64().and_then(|id| self.sent.remove(&id))
        else {
            return;
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the page posts a fetch status, which is 0 or an HTTP status below 600, so \
                      it always fits"
        )]
        let status = message["status"].as_u64().unwrap_or(0) as u16;
        let response = Response {
            status,
            retry_after: None,
            body: message["body"].as_str().unwrap_or("").to_owned(),
        };
        let seen = self.headers_seen.remove(&url);
        if response.status != 429 && response.status != 503 {
            answer(reply, response);
            return;
        }
        match seen {
            Some(retry_after) => answer(
                reply,
                Response {
                    retry_after,
                    ..response
                },
            ),
            None => {
                if let Some(earlier) = self
                    .awaiting_headers
                    .insert(url, HeldAnswer { reply, response })
                {
                    answer(earlier.reply, earlier.response);
                }
            }
        }
    }
}

/// Sends a request's answer. A caller that gave up waiting has dropped its receiver, and the
/// answer is dropped with it.
fn answer(reply: Sender<Response>, response: Response) {
    #[expect(
        clippy::let_underscore_must_use,
        let_underscore_drop,
        reason = "the send fails only when the caller timed out and dropped its receiver, so the \
                  answer has nobody left to go to"
    )]
    let _ = reply.send(response);
}

/// The browser thread's side: a hidden window holding the `WebView`. Everything here runs on that
/// thread, as `WebView2` requires.
pub(crate) struct Window {
    hwnd: HWND,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    home: HSTRING,
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "each WebView2 event handler is a boxed `'static` closure that reads and \
                      changes the state the window's own job handling uses too"
        )
    )]
    state: Rc<RefCell<State>>,
}

impl Window {
    /// Opens the hidden window and its `WebView`, with the profile in `data_dir`. `page` gets the
    /// status of each page load.
    #[expect(
        unsafe_code,
        reason = "creating the WebView2 environment and controller and setting them up are COM \
                  calls the bindings expose only as unsafe"
    )]
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the page-load handler writes the status that `Browser` reads on the \
                      caller's thread"
        )
    )]
    pub(crate) fn open(data_dir: &HSTRING, home: HSTRING, page: Arc<AtomicI32>) -> Result<Self> {
        use anyhow::Context as _;

        let hwnd = hidden_window()?;
        let environment = {
            let (tx, rx) = mpsc::channel();
            let data_dir = data_dir.clone();
            let options = CoreWebView2EnvironmentOptions::default();
            // SAFETY: `options` is new and only this thread holds it, so nothing reads its
            // arguments while they are written.
            unsafe { options.set_additional_browser_arguments(ARGS.into()) };
            let options: ICoreWebView2EnvironmentOptions = options.into();
            CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    // SAFETY: a null browser folder picks the installed runtime, and `data_dir`,
                    // `options` and `handler` are live values that outlive the call.
                    unsafe {
                        WebView2::CreateCoreWebView2EnvironmentWithOptions(
                            PCWSTR::null(),
                            &data_dir,
                            &options,
                            &handler,
                        )
                    }
                    .map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |result, environment| {
                    result?;
                    browser::deliver(&tx, environment.ok_or_else(|| Error::from(E_POINTER)));
                    Ok(())
                }),
            )
            .map_err(|e| anyhow!("{e}"))
            .context(
                "couldn't start WebView2; is the Microsoft Edge WebView2 Runtime installed?",
            )?;
            rx.recv()??
        };
        let controller = {
            let (tx, rx) = mpsc::channel();
            CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    // SAFETY: `hwnd` is the window this thread just created, and `handler` is a
                    // live value that outlives the call.
                    unsafe { environment.CreateCoreWebView2Controller(hwnd, &handler) }
                        .map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |result, controller| {
                    result?;
                    browser::deliver(&tx, controller.ok_or_else(|| Error::from(E_POINTER)));
                    Ok(())
                }),
            )
            .map_err(|e| anyhow!("{e}"))
            .context("couldn't create the WebView2 browser")?;
            rx.recv()??
        };
        let bounds = RECT {
            left: 0,
            top: 0,
            right: WIDTH,
            bottom: HEIGHT,
        };
        // SAFETY: `controller` is live and belongs to this thread, and SetBounds takes the RECT
        // by value.
        unsafe { controller.SetBounds(bounds) }?;
        // SAFETY: `controller` is live and belongs to this thread.
        let webview = unsafe { controller.CoreWebView2() }?;
        // SAFETY: `webview` is live and belongs to this thread.
        let settings = unsafe { webview.Settings() }?;
        // SAFETY: `settings` is live and belongs to this thread, and the call takes a plain value.
        unsafe { settings.SetAreDevToolsEnabled(false) }?;
        // SAFETY: `settings` is live and belongs to this thread, and the call takes a plain value.
        unsafe { settings.SetAreDefaultScriptDialogsEnabled(false) }?;
        let window = Self {
            hwnd,
            controller,
            webview,
            home,
            state: Rc::default(),
        };
        window.listen(page)?;
        Ok(window)
    }

    /// Adds the `WebView`'s event handlers: page loads, full responses for their `Retry-After`, and
    /// the answers the page posts.
    #[expect(
        unsafe_code,
        reason = "reading WebView2 event arguments and adding event handlers are COM calls the \
                  bindings expose only as unsafe"
    )]
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the page-load handler writes the status that `Browser` reads on the \
                      caller's thread"
        )
    )]
    fn listen(&self, page: Arc<AtomicI32>) -> Result<()> {
        use windows::core::Interface as _;

        let state = Rc::clone(&self.state);
        let starting = NavigationStartingEventHandler::create(Box::new(move |_, _| {
            state.borrow_mut().navigation_started();
            Ok(())
        }));
        let state = Rc::clone(&self.state);
        let completed = NavigationCompletedEventHandler::create(Box::new(move |webview, args| {
            let status = args
                .as_ref()
                .and_then(|a| a.cast::<ICoreWebView2NavigationCompletedEventArgs2>().ok())
                .and_then(|a| {
                    let mut status = 0;
                    // SAFETY: `a` is the event's live argument object, and `status` is a
                    // writable i32.
                    unsafe { a.HttpStatusCode(&raw mut status) }
                        .ok()
                        .map(|()| status)
                });
            let success = args.is_some_and(|a| {
                let mut ok = BOOL::default();
                // SAFETY: `a` is the event's live argument object, and `ok` is a writable BOOL.
                unsafe { a.IsSuccess(&raw mut ok) }.is_ok() && ok.as_bool()
            });
            let ok = success && status.is_none_or(|s| s < 400);
            page.store(
                if ok {
                    PAGE_OK
                } else {
                    status.unwrap_or(0).max(0)
                },
                Ordering::Relaxed,
            );
            let waiting = state.borrow_mut().navigation_completed(ok);
            if let Some(webview) = webview {
                for pending in waiting {
                    send(&webview, &state, pending);
                }
            }
            Ok(())
        }));
        let state = Rc::clone(&self.state);
        let response = WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            // SAFETY: `args` is the event's live argument object.
            let request = unsafe { args.Request() }?;
            let mut url = PWSTR::null();
            // SAFETY: `request` is live, and `url` is a writable PWSTR for a string the caller
            // frees, which `CoTaskMemPWSTR` does.
            unsafe { request.Uri(&raw mut url) }?;
            let url = CoTaskMemPWSTR::from(url).to_string();
            // SAFETY: `args` is the event's live argument object.
            let response = unsafe { args.Response() }?;
            let mut status = 0;
            // SAFETY: `response` is live, and `status` is a writable i32.
            unsafe { response.StatusCode(&raw mut status) }?;
            // SAFETY: `response` is live.
            let headers = unsafe { response.Headers() }?;
            let mut value = PWSTR::null();
            // SAFETY: `headers` is live, the name is a static string, and `value` is a writable
            // PWSTR for a string the caller frees, which `CoTaskMemPWSTR` does.
            let retry_after = unsafe { headers.GetHeader(w!("retry-after"), &raw mut value) }
                .ok()
                .map(|()| CoTaskMemPWSTR::from(value).to_string());
            if status != 429 && status != 503 {
                return Ok(());
            }
            state.borrow_mut().retry_after_received(url, retry_after);
            Ok(())
        }));
        let state = Rc::clone(&self.state);
        let message = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut json = PWSTR::null();
            // SAFETY: `args` is the event's live argument object, and `json` is a writable PWSTR
            // for a string the caller frees, which `CoTaskMemPWSTR` does.
            unsafe { args.WebMessageAsJson(&raw mut json) }?;
            let json = CoTaskMemPWSTR::from(json).to_string();
            let Ok(message) = serde_json::from_str::<Value>(&json) else {
                return Ok(());
            };
            state.borrow_mut().answer_received(&message);
            Ok(())
        }));
        let mut token: i64 = 0;
        // SAFETY: the WebView is live and belongs to this thread, the handler is a live COM
        // object it keeps a reference to, and `token` is a writable i64.
        unsafe {
            self.webview
                .add_NavigationStarting(&starting, &raw mut token)
        }?;
        // SAFETY: as for the handler above.
        unsafe {
            self.webview
                .add_NavigationCompleted(&completed, &raw mut token)
        }?;
        let webview = self.webview.cast::<ICoreWebView2_2>()?;
        // SAFETY: as for the handlers above.
        unsafe { webview.add_WebResourceResponseReceived(&response, &raw mut token) }?;
        // SAFETY: as for the handlers above.
        unsafe {
            self.webview
                .add_WebMessageReceived(&message, &raw mut token)
        }?;
        Ok(())
    }

    /// Runs the thread's message loop until `WM_QUIT`, handling `jobs` each time the
    /// [`Browser`](crate::Browser) posts `WM_APP`. `GetMessageW` returns 0 for `WM_QUIT` and -1 on an
    /// error, and both end the loop.
    #[expect(
        unsafe_code,
        reason = "the Win32 message loop's calls are ones `windows` exposes only as unsafe"
    )]
    pub(crate) fn run(self, jobs: &Receiver<Job>) {
        let mut msg = MSG::default();
        loop {
            // SAFETY: `msg` is a writable MSG, and no window filter reads every message of this
            // thread.
            let got = unsafe { WindowsAndMessaging::GetMessageW(&raw mut msg, None, 0, 0) };
            if got.0 <= 0 {
                break;
            }
            if msg.hwnd.is_invalid() && msg.message == WM_APP {
                for job in jobs.try_iter() {
                    self.handle(job);
                }
                continue;
            }
            // SAFETY: `msg` is the message GetMessageW just filled in.
            let translated = unsafe { WindowsAndMessaging::TranslateMessage(&raw const msg) };
            #[expect(
                clippy::let_underscore_must_use,
                reason = "TranslateMessage only says whether the message was a key press it \
                          translated, which the loop has no use for"
            )]
            let _ = translated;
            // SAFETY: `msg` is the message GetMessageW just filled in.
            unsafe { WindowsAndMessaging::DispatchMessageW(&raw const msg) };
        }
    }

    /// Sends a request now if the page has loaded, or queues it and loads the page if no load is
    /// under way. A reload loads the page unless a load is already under way.
    #[expect(
        unsafe_code,
        reason = "loading the page is a COM call the bindings expose only as unsafe"
    )]
    fn handle(&self, job: Job) {
        let load = match job {
            Job::Get(pending) => {
                if self.state.borrow().ready {
                    send(&self.webview, &self.state, pending);
                    return;
                }
                let mut state = self.state.borrow_mut();
                state.waiting.push(pending);
                !mem::replace(&mut state.loading, true)
            }
            Job::Reload => {
                let mut state = self.state.borrow_mut();
                state.ready = false;
                !mem::replace(&mut state.loading, true)
            }
        };
        if !load {
            return;
        }
        // SAFETY: the WebView is live and belongs to this thread, and `home` is a live string.
        let navigated = unsafe { self.webview.Navigate(&self.home) };
        if navigated.is_err() {
            self.state.borrow_mut().loading = false;
        }
    }
}

impl Drop for Window {
    #[expect(
        unsafe_code,
        reason = "closing the WebView and destroying its window are calls the bindings expose \
                  only as unsafe"
    )]
    fn drop(&mut self) {
        // SAFETY: the controller is live and belongs to this thread, and is closed only here.
        let closed = unsafe { self.controller.Close() };
        #[expect(
            clippy::let_underscore_must_use,
            let_underscore_drop,
            reason = "the browser is shutting down, and a controller that fails to close has \
                      nothing left to retry or report to"
        )]
        let _ = closed;
        // SAFETY: `hwnd` is this thread's window, and is destroyed only here.
        let destroyed = unsafe { WindowsAndMessaging::DestroyWindow(self.hwnd) };
        #[expect(
            clippy::let_underscore_must_use,
            let_underscore_drop,
            reason = "the browser is shutting down, and a window that fails to go away has \
                      nothing left to retry or report to"
        )]
        let _ = destroyed;
    }
}

/// Runs `fetch` on the page; its result comes back through `postMessage`. A request whose caller
/// gave up waiting isn't sent, since nobody would read its answer.
#[expect(
    unsafe_code,
    reason = "running a script on the page is a COM call the bindings expose only as unsafe"
)]
fn send(webview: &ICoreWebView2, state: &RefCell<State>, pending: Pending) {
    if pending.reply.is_closed() {
        return;
    }
    let id = {
        let mut state = state.borrow_mut();
        state.next_id += 1;
        state.next_id
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
    // SAFETY: the WebView is live and belongs to this thread, and the script and handler are live
    // values that outlive the call.
    let executed = unsafe { webview.ExecuteScript(&HSTRING::from(script), &done) };
    if executed.is_err() {
        state.borrow_mut().sent.remove(&id);
    }
}

/// Creates the window the `WebView` lives in. It is never shown: the page only needs a window to
/// live in. The class is registered each time, which fails harmlessly when an earlier browser
/// already registered it.
#[expect(
    unsafe_code,
    reason = "registering a window class and creating a window are Win32 calls `windows` exposes \
              only as unsafe"
)]
fn hidden_window() -> Result<HWND> {
    // SAFETY: no module name asks for this executable's handle, which needs no cleanup.
    let instance = unsafe { LibraryLoader::GetModuleHandleW(None) }?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(default_window_proc),
        hInstance: instance.into(),
        lpszClassName: CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is a filled-in WNDCLASSW whose procedure and names live for the whole
    // program.
    unsafe { WindowsAndMessaging::RegisterClassW(&raw const class) };
    // SAFETY: the names are static strings and every other argument is a plain value or none. If
    // registering the class failed, the call fails and returns an error rather than a window.
    let hwnd = unsafe {
        WindowsAndMessaging::CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("teamdiff"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }?;
    Ok(hwnd)
}

/// The hidden window's procedure, which leaves every message to Windows.
#[expect(
    unsafe_code,
    reason = "a window procedure is an unsafe callback, since Windows passes it pointers inside \
              `l` that only some messages make valid"
)]
unsafe extern "system" fn default_window_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    // SAFETY: the arguments are the ones Windows passed this procedure, unchanged.
    unsafe { WindowsAndMessaging::DefWindowProcW(hwnd, msg, w, l) }
}
