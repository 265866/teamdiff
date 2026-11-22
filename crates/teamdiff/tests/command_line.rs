//! Tests for the command line of `teamdiff`.

use std::process::{Command, Output};

/// teamdiff 0.2.1's help after `teamdiff <version>`, word for word. A copy rather than the
/// constant the binary prints, so a change to the help fails here.
const HELP_AFTER_VERSION: &str = ": a terminal tracker for your VALORANT matches

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
Riot Games does not endorse or sponsor this project.";

/// Runs the built `teamdiff` with `arguments`.
fn run(arguments: &[&str]) -> Output {
    let binary = env!("CARGO_BIN_EXE_teamdiff");
    Command::new(binary)
        .args(arguments)
        .output()
        .expect("the binary should start")
}

/// The help as this build prints it.
fn help() -> String {
    format!("teamdiff {}{HELP_AFTER_VERSION}", env!("CARGO_PKG_VERSION"))
}

#[test]
fn version_prints_the_version_line_and_succeeds() {
    for flag in ["--version", "-V"] {
        let output = run(&[flag]);
        assert!(
            output.status.success(),
            "`{flag}` should exit 0: {output:?}"
        );
        let name = "teamdiff";
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{name} {}\n", env!("CARGO_PKG_VERSION")),
            "`{flag}` should print the name and the version"
        );
        assert!(
            output.stderr.is_empty(),
            "nothing goes to stderr: {output:?}"
        );
    }
}

#[test]
fn an_unknown_argument_fails_naming_it() {
    let output = run(&["--unknown"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "an argument the program doesn't know should fail: {output:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("unknown argument \"--unknown\"\n\n{}\n", help()),
        "the problem, a blank line, then the help"
    );
}

#[test]
fn a_refused_command_line_names_the_problem_then_the_help() {
    let output = run(&["--demo", "nowhere"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "nothing goes to stdout: {output:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("unknown demo screen \"nowhere\"\n\n{}\n", help()),
        "the problem, a blank line, then the help, as teamdiff 0.2.1 printed it"
    );
}

#[test]
fn help_prints_the_help_and_succeeds() {
    for flag in ["--help", "-h"] {
        let output = run(&[flag]);
        assert!(
            output.status.success(),
            "`{flag}` should exit 0: {output:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}\n", help()),
            "`{flag}` prints teamdiff 0.2.1's help"
        );
        assert!(
            output.stderr.is_empty(),
            "nothing goes to stderr: {output:?}"
        );
    }
}

#[test]
fn licenses_prints_the_embedded_notices_and_succeeds() {
    let output = run(&["--licenses"]);
    assert!(
        output.status.success(),
        "`--licenses` should exit 0: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .starts_with("This build of teamdiff didn't include its dependencies' license notices")
            || stdout.starts_with("teamdiff third-party licenses\n"),
        "a test build embeds the note, a release build the notices: {stdout}"
    );
    assert!(
        output.stderr.is_empty(),
        "nothing goes to stderr: {output:?}"
    );
}
