//! The logic behind `teamdiff`, apart from its binary so the binary's command-line and
//! runtime dependencies stay out of it.

mod cast;
pub mod content;
pub mod goldstars;
pub mod local_files;
pub mod presence;
pub mod stats;
mod version_line;

pub use version_line::version_line;
