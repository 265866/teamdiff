//! Everything `teamdiff` says to the outside. So far, the self-updater: in a release build that
//! [`update::enabled`] says updates itself, the binary starts [`update::run`].

pub mod update;
