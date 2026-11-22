use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::Builder;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::oneshot;
use tokio::time;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::{Com, Threading};
use windows::Win32::UI::WindowsAndMessaging;
use windows::core::HSTRING;

use crate::response::Response;
use crate::window::{Job, Pending, Window};

/// The page the browser sits on: a tracker.gg page with no scripts or ads, so the browser stays
/// small. Requests sent from it carry tracker.gg's origin and cookies, as the site's own do.
pub const HOME: &str = "https://tracker.gg/robots.txt";

/// The page status while the last page load wasn't refused.
pub(crate) const PAGE_OK: i32 = -1;

/// A hidden `WebView2` on a thread of its own, sitting on a page and making requests from it.
///
/// Dropping it closes the browser.
#[derive(Debug)]
pub struct Browser {
    jobs: Sender<Job>,
    thread: u32,
    timeout: Duration,
    /// The HTTP status of the last page load if it was refused (0 without one), otherwise
    /// `PAGE_OK`.
    #[cfg_attr(
        dylint_lib = "trellis_lints",
        expect(
            shared_ownership,
            reason = "the browser thread's page-load handler writes the status that requests \
                      read on the caller's side, and each side lives on its own thread"
        )
    )]
    page: Arc<AtomicI32>,
}

impl Browser {
    /// Starts the browser on its own thread with its profile under `data_dir`. It opens `home` with
    /// the first request; a request gets `timeout` to answer, page load included.
    ///
    /// # Errors
    ///
    /// Fails when `data_dir` can't be created, the thread can't start, or `WebView2` doesn't start,
    /// as when the Microsoft Edge `WebView2` Runtime isn't installed.
    pub fn start(data_dir: &Path, home: &str, timeout: Duration) -> Result<Self> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("couldn't create {}", data_dir.display()))?;
        let data_dir = HSTRING::from(data_dir.as_os_str());
        let home = HSTRING::from(home);
        let page = Arc::new(AtomicI32::new(PAGE_OK));
        let (jobs, received) = mpsc::channel();
        let (started, started_rx) = mpsc::channel();
        let page_status = Arc::clone(&page);
        Builder::new()
            .name("tracker-browser".to_owned())
            .spawn(move || run_thread(&data_dir, home, page_status, received, &started))?;
        let thread = started_rx.recv().context("the browser thread stopped")??;
        Ok(Self {
            jobs,
            thread,
            timeout,
            page,
        })
    }

    /// GET `url` from the page. `trim`, a JavaScript function, cuts a successful JSON body down in
    /// the page, so a large response never reaches the app whole.
    ///
    /// # Errors
    ///
    /// Fails when the browser has stopped or can't be reached, when it closes during the request,
    /// or when the page loaded but no answer came within the timeout. A page the site refused
    /// isn't an error: the request answers with status 0 and the reason in its body.
    pub async fn get_trimmed(&self, url: &str, trim: Option<&'static str>) -> Result<Response> {
        let (reply, answer) = oneshot::channel();
        self.post(Job::Get(Pending {
            url: url.to_owned(),
            trim,
            reply,
        }))?;
        match time::timeout(self.timeout, answer).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => bail!("the browser closed during the request"),
            Err(_) => self.timed_out(),
        }
    }

    /// Loads the page again. Requests in flight are sent again from the new page.
    ///
    /// # Errors
    ///
    /// Fails when the browser has stopped or can't be reached.
    pub fn reload(&self) -> Result<()> {
        self.post(Job::Reload)
    }

    /// The answer to a request that timed out. When the page never loaded because the site
    /// refused it, that is a refusal, not a stuck browser.
    fn timed_out(&self) -> Result<Response> {
        match self.page.load(Ordering::Relaxed) {
            PAGE_OK => bail!(
                "tracker.gg didn't answer within {} s",
                self.timeout.as_secs()
            ),
            0 => Ok(Response {
                status: 0,
                retry_after: None,
                body: "the tracker.gg page couldn't load".into(),
            }),
            status => Ok(Response {
                status: 0,
                retry_after: None,
                body: format!("the tracker.gg page didn't load (HTTP {status})"),
            }),
        }
    }

    #[expect(
        unsafe_code,
        reason = "waking the browser thread takes PostThreadMessageW, which `windows` exposes only \
                  as an unsafe call"
    )]
    fn post(&self, job: Job) -> Result<()> {
        self.jobs
            .send(job)
            .map_err(|_| anyhow!("the browser has stopped"))?;
        // SAFETY: PostThreadMessageW takes only plain values, and WM_APP with zero parameters
        // carries no pointer for the browser thread to read.
        unsafe {
            WindowsAndMessaging::PostThreadMessageW(
                self.thread,
                WindowsAndMessaging::WM_APP,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .context("couldn't reach the browser")
    }
}

impl Drop for Browser {
    #[expect(
        unsafe_code,
        reason = "stopping the browser thread takes PostThreadMessageW, which `windows` exposes \
                  only as an unsafe call"
    )]
    fn drop(&mut self) {
        // SAFETY: PostThreadMessageW takes only plain values, and WM_QUIT carries no pointer.
        let quit = unsafe {
            WindowsAndMessaging::PostThreadMessageW(
                self.thread,
                WindowsAndMessaging::WM_QUIT,
                WPARAM(0),
                LPARAM(0),
            )
        };
        #[expect(
            clippy::let_underscore_must_use,
            let_underscore_drop,
            reason = "the post fails only when the browser thread has already stopped, which is \
                      the state dropping the browser asks for"
        )]
        let _ = quit;
    }
}

/// The browser thread: joins a single-threaded COM apartment, as `WebView2` requires, opens the
/// window and runs its message loop until the [`Browser`] drops. `started` gets the thread's ID
/// once the browser is open, or why it couldn't open.
#[expect(
    unsafe_code,
    reason = "joining and leaving a COM apartment and reading the thread's ID are Win32 calls \
              `windows` exposes only as unsafe"
)]
#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        shared_ownership,
        reason = "the page-load handler on this thread writes the status that `Browser` reads on \
                  the caller's thread"
    )
)]
fn run_thread(
    data_dir: &HSTRING,
    home: HSTRING,
    page: Arc<AtomicI32>,
    jobs: Receiver<Job>,
    started: &Sender<Result<u32>>,
) {
    // SAFETY: called once at the start of this thread, before any other COM call on it, and
    // balanced by the CoUninitialize below when it succeeds.
    let apartment = unsafe { Com::CoInitializeEx(None, Com::COINIT_APARTMENTTHREADED) };
    if let Err(e) = apartment.ok() {
        deliver(started, Err(e.into()));
        return;
    }
    match Window::open(data_dir, home, page) {
        Ok(window) => {
            // SAFETY: GetCurrentThreadId takes nothing and can't fail.
            let thread = unsafe { Threading::GetCurrentThreadId() };
            deliver(started, Ok(thread));
            window.run(&jobs);
            drop(jobs);
        }
        Err(e) => deliver(started, Err(e)),
    }
    // SAFETY: balances the successful CoInitializeEx above, after the window and every COM
    // object it held have been released.
    unsafe { Com::CoUninitialize() };
}

/// Sends `value` to a receiver that reads it only after this send, in the function that made the
/// channel, so the send can't fail.
pub(crate) fn deliver<T>(to: &Sender<T>, value: T) {
    #[expect(
        clippy::let_underscore_must_use,
        let_underscore_drop,
        reason = "each receiver is still waiting in the function that made the channel, so a \
                  failed send would mean nobody is left to tell"
    )]
    let _ = to.send(value);
}
