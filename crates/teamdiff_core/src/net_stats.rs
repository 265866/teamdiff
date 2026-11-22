//! Counters of the requests teamdiff makes, for the status corner.

use std::sync::Mutex;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

/// Request counters shown in the status corner. The Riot and tracker.gg clients count into the
/// same counters.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct NetStats {
    /// Requests sent so far.
    pub started: AtomicU32,
    /// Requests answered or failed so far.
    pub finished: AtomicU32,
    /// When the latest wait a server asked for ends.
    pub rate_limited_until: Mutex<Option<Instant>>,
}
