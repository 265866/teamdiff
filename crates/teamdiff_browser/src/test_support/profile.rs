use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, io, process, thread};

/// The lock file every test process shares to take turns, in the temp folder.
const TURN_FILE: &str = "teamdiff-browser-tests.lock";

/// A fresh profile folder in the temp folder, removed once the browser's processes let go of it.
///
/// `WebView2` can take longer than a request's timeout to start when several start at once, so
/// tests that hold a profile take turns. The turn is a lock on a file in the temp folder rather
/// than in the process, since nextest runs each test in a process of its own.
#[derive(Debug)]
pub struct Profile {
    dir: PathBuf,
    /// The open lock file while this profile holds the turn.
    turn: Option<File>,
}

impl Profile {
    /// Waits for the turn, then names a folder for `name` and this process in the temp folder.
    /// [`Browser::start`](crate::Browser::start) creates it.
    ///
    /// # Errors
    ///
    /// Fails when the lock file can't be opened or locked.
    pub fn try_new(name: &str) -> io::Result<Self> {
        let turn = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(env::temp_dir().join(TURN_FILE))?;
        turn.lock()?;
        let dir = env::temp_dir().join(format!("teamdiff-{name}-{}", process::id()));
        Ok(Self {
            dir,
            turn: Some(turn),
        })
    }

    /// The profile folder, to pass to [`Browser::start`](crate::Browser::start).
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Profile {
    /// Passes the turn on first, so the next test can start its browser while this one's files
    /// unlock. `WebView2`'s processes can hold the profile's files for a while after the browser
    /// closes, so removing the folder is retried for up to 30 s.
    fn drop(&mut self) {
        drop(self.turn.take());
        for _ in 0..300 {
            if fs::remove_dir_all(&self.dir).is_ok() || !self.dir.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
