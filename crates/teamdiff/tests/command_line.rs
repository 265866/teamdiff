//! Tests for the command line of `teamdiff`.

use std::process::{Command, Output};

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
