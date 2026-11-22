//! Tests for the command line of `teamdiff`.

use std::process::{Command, Output};

use teamdiff_core::command_line::HELP;

/// Runs the built `teamdiff` with `arguments`.
fn run(arguments: &[&str]) -> Output {
    let binary = env!("CARGO_BIN_EXE_teamdiff");
    Command::new(binary)
        .args(arguments)
        .output()
        .expect("the binary should start")
}

#[test]
fn version_prints_the_version_line_and_succeeds() {
    let output = run(&["--version"]);
    assert!(
        output.status.success(),
        "`--version` should exit 0: {output:?}"
    );
    let name = "teamdiff";
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{name} {}\n", env!("CARGO_PKG_VERSION")),
        "`--version` should print the name and the version"
    );
}

#[test]
fn an_unknown_argument_fails() {
    let output = run(&["--unknown"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "an argument the program doesn't know should fail: {output:?}"
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    let help = HELP;
    assert_eq!(
        stderr,
        format!("unknown demo screen \"nowhere\"\n\n{help}\n"),
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
        let help = HELP;
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{help}\n"),
            "`{flag}` prints the help"
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
}
