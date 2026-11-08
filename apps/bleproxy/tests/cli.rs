//! End-to-end tests that run the compiled `bleproxy` binary.

#![allow(clippy::expect_used, reason = "test helpers should fail loudly")]

use std::process::{Command, Output};

fn bleproxy(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bleproxy"))
        .args(args)
        .output()
        .expect("failed to run bleproxy")
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("stdout is not UTF-8")
}

#[test]
fn prints_name_and_version() {
    let expected = format!("bleproxy {}", env!("CARGO_PKG_VERSION"));
    for args in [&[][..], &["--version"]] {
        let output = bleproxy(args);
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert_eq!(stdout(&output).trim_end(), expected, "{args:?}");
    }
}

#[test]
fn rejects_unknown_arguments() {
    let output = bleproxy(&["--no-such-flag"]);
    assert!(!output.status.success());
}
