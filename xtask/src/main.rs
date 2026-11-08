//! Development task runner for the bleproxy workspace.
//!
//! Invoked as `cargo xtask <task>` through the alias in `.cargo/config.toml`.
//! Tasks shell out to cargo and to optional cargo plugins; none of them are
//! needed for a plain `cargo build`. Run `cargo xtask help` for the list.

use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A task that `cargo xtask` can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Task {
    Build,
    Check,
    Fmt,
    FmtCheck,
    Lint,
    Test,
    Docs,
    Metadata,
    Deny,
    Deps,
    Coverage,
    Ci,
    Help,
}

/// Every task with its name and description, in the order `help` lists them.
const TASKS: &[(Task, &str, &str)] = &[
    (Task::Build, "build", "Build the host workspace"),
    (Task::Check, "check", "Type-check the host workspace"),
    (Task::Fmt, "fmt", "Format all code"),
    (Task::FmtCheck, "fmt-check", "Verify formatting"),
    (Task::Lint, "lint", "Run Clippy, denying warnings"),
    (
        Task::Test,
        "test",
        "Run tests (nextest if installed, plus doctests)",
    ),
    (Task::Docs, "docs", "Build documentation, denying warnings"),
    (
        Task::Metadata,
        "metadata",
        "Validate the workspace manifests and lockfile",
    ),
    (
        Task::Deny,
        "deny",
        "Check advisories, licenses, bans and sources (cargo-deny)",
    ),
    (
        Task::Deps,
        "deps",
        "Find unused dependencies (cargo-machete)",
    ),
    (
        Task::Coverage,
        "coverage",
        "Write an lcov coverage report to lcov.info (cargo-llvm-cov)",
    ),
    (Task::Ci, "ci", "Run every check CI runs on the host"),
    (Task::Help, "help", "Show this message"),
];

impl Task {
    fn from_name(name: &str) -> Option<Self> {
        TASKS
            .iter()
            .find(|(_, task_name, _)| *task_name == name)
            .map(|(task, _, _)| *task)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.split_first() {
        None => run(Task::Help),
        Some((name, rest)) => parse(name, rest).and_then(run),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn parse(name: &str, args: &[String]) -> Result<Task> {
    let task = match name {
        "-h" | "--help" => Task::Help,
        name => Task::from_name(name)
            .ok_or_else(|| format!("unknown task `{name}`, see `cargo xtask help`"))?,
    };
    if !args.is_empty() {
        return Err(format!("`{name}` takes no arguments, got {args:?}").into());
    }
    Ok(task)
}

fn run(task: Task) -> Result {
    match task {
        Task::Build => cargo(["build", "--all-targets", "--locked"]),
        Task::Check => cargo(["check", "--all-targets", "--locked"]),
        Task::Fmt => cargo(["fmt", "--all"]),
        Task::FmtCheck => cargo(["fmt", "--all", "--", "--check"]),
        Task::Lint => cargo([
            "clippy",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        Task::Test => test(),
        Task::Docs => docs(),
        Task::Metadata => metadata(),
        Task::Deny => {
            require("deny", "cargo-deny")?;
            cargo(["deny", "check"])
        }
        Task::Deps => {
            require("machete", "cargo-machete")?;
            cargo(["machete"])
        }
        Task::Coverage => coverage(),
        Task::Ci => ci(),
        Task::Help => {
            help();
            Ok(())
        }
    }
}

fn help() {
    println!("Usage: cargo xtask <task>\n\nTasks:");
    for (_, name, about) in TASKS {
        println!("  {name:<10}  {about}");
    }
}

fn ci() -> Result {
    for task in [
        Task::FmtCheck,
        Task::Metadata,
        Task::Lint,
        Task::Build,
        Task::Test,
        Task::Docs,
        Task::Deny,
        Task::Deps,
    ] {
        eprintln!("==> {task:?}");
        run(task)?;
    }
    eprintln!("==> all checks passed");
    Ok(())
}

fn test() -> Result {
    if has_subcommand("nextest") {
        cargo(["nextest", "run", "--locked"])?;
        // nextest does not run doctests.
        cargo(["test", "--doc", "--locked"])
    } else {
        eprintln!("note: cargo-nextest is not installed, falling back to `cargo test`");
        cargo(["test", "--locked"])
    }
}

fn docs() -> Result {
    status(
        cargo_command()
            .args(["doc", "--no-deps", "--locked"])
            .env("RUSTDOCFLAGS", "-D warnings"),
    )
}

/// Fails if any manifest is invalid or `Cargo.lock` is out of date.
fn metadata() -> Result {
    status(
        cargo_command()
            .args(["metadata", "--format-version=1", "--no-deps", "--locked"])
            .stdout(Stdio::null()),
    )
}

fn coverage() -> Result {
    require("llvm-cov", "cargo-llvm-cov")?;
    let runner: &[&str] = if has_subcommand("nextest") {
        &["llvm-cov", "nextest"]
    } else {
        &["llvm-cov"]
    };
    status(
        cargo_command()
            .args(runner)
            .args(["--locked", "--no-report"]),
    )?;
    // The task runner is development tooling, not shipped code.
    let report = ["llvm-cov", "report", "--ignore-filename-regex", "xtask/"];
    status(cargo_command().args(report).arg("--summary-only"))?;
    status(
        cargo_command()
            .args(report)
            .args(["--lcov", "--output-path", "lcov.info"]),
    )?;
    eprintln!("wrote lcov.info");
    Ok(())
}

/// Runs cargo with `args` from the workspace root.
fn cargo<I, S>(args: I) -> Result
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    status(cargo_command().args(args))
}

fn cargo_command() -> Command {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command.current_dir(workspace_root());
    // `cargo run` describes the xtask package itself through these
    // variables. Child commands must not inherit them: some plugins, such as
    // cargo-machete, use them to detect how they were invoked.
    for (key, _) in env::vars_os() {
        let key_str = key.to_string_lossy();
        if PACKAGE_ENV_PREFIXES
            .iter()
            .any(|prefix| key_str.starts_with(prefix))
        {
            command.env_remove(&key);
        }
    }
    command
}

/// Prefixes of the environment variables cargo sets when running a package.
const PACKAGE_ENV_PREFIXES: &[&str] = &[
    "CARGO_BIN_",
    "CARGO_CRATE_",
    "CARGO_MANIFEST_",
    "CARGO_PKG_",
    "CARGO_PRIMARY_PACKAGE",
];

fn workspace_root() -> &'static Path {
    // xtask lives one level below the workspace root.
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

/// Returns whether `cargo <name>` is available, built in or installed.
fn has_subcommand(name: &str) -> bool {
    // `cargo --list` is more reliable than probing `cargo <name> --version`,
    // which not every plugin implements.
    cargo_command()
        .arg("--list")
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .any(|command| command == name)
        })
}

fn require(subcommand: &str, package: &str) -> Result {
    if has_subcommand(subcommand) {
        Ok(())
    } else {
        Err(format!(
            "`cargo {subcommand}` is not available; install it with \
             `cargo install --locked {package}`"
        )
        .into())
    }
}

/// Runs `command`, echoing it first, and fails if it does not succeed.
fn status(command: &mut Command) -> Result {
    let line = describe(command);
    eprintln!("$ {line}");
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{line}` failed ({status})").into())
    }
}

fn describe(command: &Command) -> String {
    let program = Path::new(command.get_program())
        .file_stem()
        .unwrap_or_else(|| command.get_program());
    let mut line = program.to_string_lossy().into_owned();
    for arg in command.get_args() {
        line.push(' ');
        line.push_str(&arg.to_string_lossy());
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_task_has_a_unique_name() {
        for (task, name, _) in TASKS {
            assert_eq!(parse(name, &[]).unwrap(), *task, "{name}");
        }
    }

    #[test]
    fn rejects_unknown_tasks() {
        let err = parse("no-such-task", &[]).unwrap_err();
        assert!(err.to_string().contains("unknown task"));
    }

    #[test]
    fn rejects_unexpected_arguments() {
        let err = parse("build", &["--release".into()]).unwrap_err();
        assert!(err.to_string().contains("takes no arguments"));
    }
}
