//! `teamdiff`: A terminal companion for VALORANT on Windows: your lobby's ranks, records and recent form, and the post-game results.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::{env, io};

use teamdiff_core::command_line::{self, Args, Command, HELP, Mode};
use teamdiff_core::content::Content;
use teamdiff_core::demo;
use teamdiff_core::model::{Model, Updated};
use teamdiff_core::shared::{self, Shared};
use teamdiff_live::{controller, update};
use tokio::runtime::Builder;

/// The dependencies' license notices, which `build.rs` embeds.
const LICENSES: &str = include_str!(concat!(env!("OUT_DIR"), "/THIRD-PARTY-LICENSES.txt"));

fn main() -> anyhow::Result<ExitCode> {
    use std::io::Write as _;

    let Args { mode, update } = match command_line::parse_args(env::args().skip(1)) {
        Ok(Command::Run(args)) => args,
        Ok(Command::Help) => {
            writeln!(io::stdout().lock(), "{HELP}")?;
            return Ok(ExitCode::SUCCESS);
        }
        Ok(Command::Licenses) => {
            #[expect(
                clippy::let_underscore_must_use,
                let_underscore_drop,
                reason = "a pager that quits early closes the pipe, and then nothing is left to \
                          print the notices to"
            )]
            let _ = io::stdout().lock().write_all(LICENSES.as_bytes());
            return Ok(ExitCode::SUCCESS);
        }
        Ok(Command::Version) => {
            let line = teamdiff_core::version_line();
            writeln!(io::stdout().lock(), "{line}")?;
            return Ok(ExitCode::SUCCESS);
        }
        Err(e) => {
            writeln!(io::stderr().lock(), "{e}\n\n{HELP}")?;
            return Ok(ExitCode::from(2));
        }
    };
    let shared: Shared = Arc::new(Mutex::new(match &mode {
        Mode::Demo { screen } => demo::model(demo::SCREENS[*screen]),
        Mode::Live { .. } => Model::new(Content::default()),
    }));
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    if let Mode::Live { dump } = &mode {
        if update::enabled() {
            let updated = update::updated().map(|version| Updated {
                version,
                started: Instant::now(),
            });
            shared::lock(&shared).updated = updated;
            if let Ok(exe) = env::current_exe() {
                update::remove_replaced(&exe);
            }
        }
        runtime.spawn(controller::run(Arc::clone(&shared), dump.clone()));
        if update && update::enabled() {
            runtime.spawn(update::run(Arc::clone(&shared)));
        }
    }
    let demo_screen = match mode {
        Mode::Demo { screen } => Some(screen),
        Mode::Live { .. } => None,
    };
    teamdiff_ui::run(&shared, demo_screen)?;
    Ok(ExitCode::SUCCESS)
}
