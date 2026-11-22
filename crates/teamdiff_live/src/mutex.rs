//! Locking the small mutexes the clients keep their state in.

use std::sync::{Mutex, MutexGuard};

/// Locks `mutex`.
///
/// # Panics
///
/// When a thread panicked while it held the lock. A release build aborts the process on a panic,
/// so there no lock is ever poisoned and this never panics, as with the model's lock in
/// `teamdiff_core::shared`.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("a thread panicked while it held a client's lock")
}
