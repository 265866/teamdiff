//! A hidden `WebView2`, the Edge engine built into Windows, that sits on a tracker.gg page and calls
//! tracker.gg's API from it the way the site's own scripts do. Cloudflare sees a real browser: a
//! page load lets it run its checks, and the cookies they earn are kept in the app's own profile.

mod browser;
mod response;
pub mod test_support;
mod window;

pub use browser::{Browser, HOME};
pub use response::Response;
