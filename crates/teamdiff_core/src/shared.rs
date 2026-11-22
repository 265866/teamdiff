//! The model as the controller, which keeps it current, and the UI, which draws it, share it.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::model::Model;

/// The model, shared between the controller's task and the terminal loop.
#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        shared_ownership,
        reason = "teamdiff_live's controller runs in a spawned tokio task, which must own what it \
                  uses, while teamdiff_ui's terminal loop on the main thread reads the same model"
    )
)]
pub type Shared = Arc<Mutex<Model>>;

/// Locks the shared model.
///
/// # Panics
///
/// When a thread panicked while it held the lock. A release build aborts the process on a panic,
/// so there the lock is never poisoned and this never panics.
pub fn lock(shared: &Shared) -> MutexGuard<'_, Model> {
    shared
        .lock()
        .expect("a thread panicked while it held the model's lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;
    use crate::model::Phase;

    #[test]
    fn a_change_made_through_one_handle_shows_through_the_other() {
        let controller: Shared = Arc::new(Mutex::new(Model::new(Content::default())));
        let ui = Arc::clone(&controller);
        lock(&controller).phase = Phase::Menus;
        assert_eq!(lock(&ui).phase, Phase::Menus);
    }

    #[test]
    #[should_panic(expected = "a thread panicked while it held the model's lock")]
    fn a_poisoned_lock_panics_rather_than_handing_out_the_model() {
        use std::thread;

        let shared: Shared = Arc::new(Mutex::new(Model::new(Content::default())));
        let holder = Arc::clone(&shared);
        let poisoned = thread::spawn(move || {
            let _guard = lock(&holder);
            panic!("the holder panics with the lock held");
        })
        .join();
        assert!(poisoned.is_err(), "the holder's panic poisons the lock");
        drop(lock(&shared));
    }
}
