//! The command line: its help text, and what its arguments ask for.

use std::path::PathBuf;

use crate::demo;

/// The text `--help` prints, and an error prints after its message.
pub const HELP: &str = concat!(
    "teamdiff ",
    env!("CARGO_PKG_VERSION"),
    ": a terminal tracker for your VALORANT matches

USAGE:
    teamdiff [--dump <dir>] [--no-update]
    teamdiff --demo [screen]
    teamdiff --licenses
    teamdiff --version

OPTIONS:
    --dump <dir>   Save raw API responses under <dir> (for checking field names)
    --no-update    Don't install new releases of teamdiff by itself
    --demo         Show sample data instead of connecting; Tab cycles screens
                   (waiting, menus, agent-select, loading, in-game, victory, defeat)
    --licenses     Print the licenses of the open-source code teamdiff is built with
    -V, --version  Print teamdiff's version
    -h, --help     Show this help

KEYS:
    up/down  select a player     enter  details     ?  accolade legend
    esc      back                q      quit

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' \"Legal Jibber Jabber\" policy using assets owned by Riot Games.
Riot Games does not endorse or sponsor this project."
);

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the binary does something different for each command by matching every one, so a               new command has to fail its build until the binary handles it"
)]
pub enum Command {
    /// Print [`HELP`].
    Help,
    /// Print the licenses of the open-source code teamdiff is built with.
    Licenses,
    /// Print the version line.
    Version,
    /// Run teamdiff.
    Run(Args),
}

/// How to run teamdiff.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "the binary takes the arguments apart into the mode and the update setting it starts               with"
)]
pub struct Args {
    /// Live or demo.
    pub mode: Mode,
    /// Whether to install new releases; a demo never does.
    pub update: bool,
}

/// Where teamdiff's data comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the binary starts the controller or the demo by matching every mode, so a new mode               has to fail its build until the binary starts it"
)]
pub enum Mode {
    /// The running game, with raw API responses saved under `dump` when it is set.
    Live {
        /// The directory `--dump` names.
        dump: Option<PathBuf>,
    },
    /// Sample data, starting at the screen with this index in [`demo::SCREENS`].
    Demo {
        /// The index of the first screen in [`demo::SCREENS`].
        screen: usize,
    },
}

/// Reads the arguments after the program's name. The first of `--help`, `--licenses` and
/// `--version` wins over everything after it. A flag right after `--dump` is a flag, not a
/// directory to fill with live responses, and the screen after `--demo` is optional, so a flag
/// after it is a flag, not a screen.
///
/// # Errors
///
/// Fails, with a message for the user, on an argument it doesn't know, on `--dump` without a
/// directory, on a demo screen that doesn't exist, and on `--dump` together with `--demo`.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    let mut mode = Mode::Live { dump: None };
    let mut update = true;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--licenses" => return Ok(Command::Licenses),
            "-V" | "--version" => return Ok(Command::Version),
            "--dump" => {
                let dir = args
                    .next_if(|a| !a.starts_with('-'))
                    .ok_or("--dump needs a directory")?;
                if matches!(mode, Mode::Demo { .. }) {
                    return Err("--demo makes no requests, so --dump has nothing to save".into());
                }
                mode = Mode::Live {
                    dump: Some(dir.into()),
                };
            }
            "--demo" => {
                if matches!(mode, Mode::Live { dump: Some(_) }) {
                    return Err("--demo makes no requests, so --dump has nothing to save".into());
                }
                let screen = match args.next_if(|a| !a.starts_with('-')) {
                    Some(name) => demo::SCREENS
                        .iter()
                        .position(|s| *s == name)
                        .ok_or_else(|| format!("unknown demo screen {name:?}"))?,
                    None => 0,
                };
                mode = Mode::Demo { screen };
            }
            "--no-update" => update = false,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Command::Run(Args { mode, update }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|a| (*a).to_string()))
    }

    fn updating(mode: Mode) -> Command {
        Command::Run(Args { mode, update: true })
    }

    fn not_updating(mode: Mode) -> Command {
        Command::Run(Args {
            mode,
            update: false,
        })
    }

    #[test]
    fn no_arguments_run_live_with_updates() {
        assert_eq!(parse(&[]), Ok(updating(Mode::Live { dump: None })));
        assert_eq!(
            parse(&["--no-update"]),
            Ok(not_updating(Mode::Live { dump: None }))
        );
    }

    #[test]
    fn dump_takes_a_directory_that_is_no_flag() {
        assert_eq!(
            parse(&["--dump", "out", "--no-update"]),
            Ok(not_updating(Mode::Live {
                dump: Some("out".into())
            }))
        );
        assert_eq!(
            parse(&["--dump", "--no-update"]),
            Err("--dump needs a directory".into())
        );
        assert_eq!(parse(&["--dump"]), Err("--dump needs a directory".into()));
    }

    #[test]
    fn demo_takes_an_optional_screen() {
        assert_eq!(parse(&["--demo"]), Ok(updating(Mode::Demo { screen: 0 })));
        assert_eq!(
            parse(&["--demo", "victory"]),
            Ok(updating(Mode::Demo { screen: 5 }))
        );
        assert_eq!(
            parse(&["--demo", "--no-update"]),
            Ok(not_updating(Mode::Demo { screen: 0 })),
            "a flag after --demo is a flag, not a screen"
        );
        assert_eq!(
            parse(&["--demo", "lobby"]),
            Err(r#"unknown demo screen "lobby""#.into())
        );
    }

    #[test]
    fn demo_and_dump_exclude_each_other_in_either_order() {
        let error: Result<Command, String> =
            Err("--demo makes no requests, so --dump has nothing to save".into());
        assert_eq!(parse(&["--demo", "--dump", "out"]), error);
        assert_eq!(parse(&["--dump", "out", "--demo"]), error);
        assert_eq!(
            parse(&["--demo", "--dump"]),
            Err("--dump needs a directory".into()),
            "the missing directory is found first"
        );
    }

    #[test]
    fn the_first_of_help_licenses_and_version_wins() {
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--help", "--bogus"]), Ok(Command::Help));
        assert_eq!(parse(&["--licenses", "-V"]), Ok(Command::Licenses));
        assert_eq!(parse(&["-V", "--help"]), Ok(Command::Version));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(
            parse(&["--bogus", "--help"]),
            Err(r#"unknown argument "--bogus""#.into()),
            "an argument before --help is still read"
        );
    }

    #[test]
    fn help_starts_with_the_version_line() {
        assert!(
            HELP.starts_with(&format!("{}: a terminal tracker", crate::version_line())),
            "{HELP}"
        );
    }
}
