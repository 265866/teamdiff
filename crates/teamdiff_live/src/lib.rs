//! Everything `teamdiff` says to the outside: the Riot Client's local API and Riot's game servers,
//! tracker.gg through `teamdiff_browser`, the controller that keeps the shared model current, and
//! the self-updater.
//!
//! The binary starts [`controller::run`] for a live session, and, in a release build that
//! [`update::enabled`] says updates itself, [`update::run`] beside it.

pub mod controller;
mod mutex;
mod riot;
mod tracker;
pub mod update;
