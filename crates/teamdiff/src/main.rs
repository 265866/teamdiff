//! `teamdiff`: A terminal companion for VALORANT on Windows: your lobby's ranks, records and recent form, and the post-game results.

use std::ffi::OsString;
use std::process::ExitCode;
use std::{env, io};

fn main() -> io::Result<ExitCode> {
    use std::io::Write as _;

    let arguments: Vec<OsString> = env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [] => run(),
        [flag] if flag == "--version" => {
            let line = teamdiff_core::version_line();
            writeln!(io::stdout().lock(), "{line}")?;
            Ok(ExitCode::SUCCESS)
        }
        _ => {
            let name = env!("CARGO_BIN_NAME");
            writeln!(io::stderr().lock(), "usage: {name} [--version]")?;
            Ok(ExitCode::from(2))
        }
    }
}

/// Runs `teamdiff`, which so far only prints its version line.
fn run() -> io::Result<ExitCode> {
    use std::io::Write as _;

    let line = teamdiff_core::version_line();
    writeln!(io::stdout().lock(), "{line}")?;
    Ok(ExitCode::SUCCESS)
}
