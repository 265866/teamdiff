//! Helpers for tests that drive a [`Browser`](crate::Browser): a local [`Site`] shaped like
//! tracker.gg's, and a throwaway [`Profile`] folder, so no test loads tracker.gg or touches the
//! app's own profile.
//!
//! They are public because `cfg(test)` code doesn't cross crates, and the tests of the crates
//! that use the browser need them too. Nothing outside tests uses them.

mod profile;
mod site;

pub use profile::Profile;
pub use site::Site;
