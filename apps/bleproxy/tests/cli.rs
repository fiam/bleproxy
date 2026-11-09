//! End-to-end tests that run the compiled `bleproxy` binary.

#![allow(clippy::expect_used, reason = "test helpers should fail loudly")]

use std::process::{Command, Output};

fn bleproxy(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bleproxy"))
        .args(args)
        .output()
        .expect("failed to run bleproxy")
}

#[test]
fn prints_name_and_version() {
    let output = bleproxy(&["--version"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout is not UTF-8");
    assert_eq!(
        stdout.trim_end(),
        format!("bleproxy {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn rejects_unknown_arguments() {
    assert!(!bleproxy(&["--no-such-flag"]).status.success());
}

#[test]
fn rejects_invalid_device_names_before_starting() {
    let output = bleproxy(&[
        "--name",
        "Not_Valid",
        "--mac",
        "02:00:00:00:00:01",
        "--listen",
        "127.0.0.1:0",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid device name"), "{stderr}");
}
