//! Locking the small mutexes the clients keep their state in.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`, handing out its value as a panicking holder left it, as teamdiff 0.2.1 did.
#[expect(
    clippy::disallowed_methods,
    reason = "release builds abort on a panic, so only a debug build or a test sees a poisoned \
              lock; each holder of these makes one change that can't stop halfway, such as \
              setting a deadline or adding an id to a set, so what a panic left is still valid"
)]
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
