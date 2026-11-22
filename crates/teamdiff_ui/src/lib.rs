//! The terminal UI of `teamdiff`: every screen drawn with ratatui from the model in
//! `teamdiff_core`, the accolade icons, and the loop that reads keys. It makes no requests.

mod cast;
#[cfg(test)]
mod demo_screens;
mod draw;
mod icons;
#[cfg(test)]
mod rendering;
mod terminal;

pub use terminal::run;
