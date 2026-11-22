//! The logic behind `teamdiff`, apart from its binary so the binary's command-line and
//! runtime dependencies stay out of it.

mod cast;
pub mod command_line;
pub mod content;
pub mod demo;
pub mod goldstars;
pub mod local_files;
pub mod model;
pub mod net_stats;
pub mod notices;
pub mod presence;
pub mod shared;
pub mod stats;
pub mod tracker;
mod version_line;

pub use version_line::version_line;
