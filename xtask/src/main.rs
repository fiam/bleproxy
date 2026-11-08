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
    Docker(Platform),
    Help,
}

/// Usage and description of every task, in the order `help` lists them.
const TASKS: &[(&str, &str)] = &[
    ("build", "Build the host workspace"),
    ("check", "Type-check the host workspace"),
    ("fmt", "Format all code"),
    ("fmt-check", "Verify formatting"),
    ("lint", "Run Clippy, denying warnings"),
    ("test", "Run tests (nextest if installed, plus doctests)"),
    ("docs", "Build documentation, denying warnings"),
    ("metadata", "Validate the workspace manifests and lockfile"),
    (
        "deny",
        "Check advisories, licenses, bans and sources (cargo-deny)",
    ),
    ("deps", "Find unused dependencies (cargo-machete)"),
    (
        "coverage",
        "Write an lcov coverage report to lcov.info (cargo-llvm-cov)",
    ),
    ("ci", "Run every check CI runs on the host"),
    (
        "docker <platform>",
        "Run the tests in a container: linux-amd64, linux-arm64 or windows",
    ),
    ("help", "Show this message"),
];

/// A platform whose test suite runs in a Docker container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    LinuxAmd64,
    LinuxArm64,
    /// `x86_64-pc-windows-gnu`, built with MinGW and run under Wine.
    Windows,
}

impl Platform {
    const ALL: [Self; 3] = [Self::LinuxAmd64, Self::LinuxArm64, Self::Windows];

    const fn name(self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux-amd64",
            Self::LinuxArm64 => "linux-arm64",
            Self::Windows => "windows",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.name() == name)
    }

    /// Container platform. Non-native ones run under QEMU emulation.
    const fn docker_platform(self) -> &'static str {
        match self {
            Self::LinuxAmd64 | Self::Windows => "linux/amd64",
            Self::LinuxArm64 => "linux/arm64",
        }
    }

    const fn dockerfile(self) -> &'static str {
        match self {
            Self::LinuxAmd64 | Self::LinuxArm64 => "docker/linux-test.Dockerfile",
            Self::Windows => "docker/windows-test.Dockerfile",
        }
    }

    const fn test_args(self) -> &'static [&'static str] {
        match self {
            Self::LinuxAmd64 | Self::LinuxArm64 => &["test", "--locked"],
            Self::Windows => &["test", "--locked", "--target", "x86_64-pc-windows-gnu"],
        }
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
        "build" => Task::Build,
        "check" => Task::Check,
        "fmt" => Task::Fmt,
        "fmt-check" => Task::FmtCheck,
        "lint" => Task::Lint,
        "test" => Task::Test,
        "docs" => Task::Docs,
        "metadata" => Task::Metadata,
        "deny" => Task::Deny,
        "deps" => Task::Deps,
        "coverage" => Task::Coverage,
        "ci" => Task::Ci,
        "docker" => return parse_docker(args),
        "help" | "-h" | "--help" => Task::Help,
        _ => return Err(format!("unknown task `{name}`, see `cargo xtask help`").into()),
    };
    if !args.is_empty() {
        return Err(format!("`{name}` takes no arguments, got {args:?}").into());
    }
    Ok(task)
}

fn parse_docker(args: &[String]) -> Result<Task> {
    let names = Platform::ALL.map(Platform::name).join(", ");
    match args {
        [platform] => Platform::from_name(platform)
            .map(Task::Docker)
            .ok_or_else(|| {
                format!("unknown platform `{platform}`, expected one of: {names}").into()
            }),
        _ => Err(format!("`docker` takes one platform: {names}").into()),
    }
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
        Task::Docker(platform) => docker(platform),
        Task::Help => {
            help();
            Ok(())
        }
    }
}

fn help() {
    println!("Usage: cargo xtask <task>\n\nTasks:");
    for (usage, about) in TASKS {
        println!("  {usage:<17}  {about}");
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

/// Builds the test image for `platform` and runs the test suite in it.
///
/// The source tree is mounted read-only; build output and the crate
/// registry live in named volumes so repeated runs are incremental.
fn docker(platform: Platform) -> Result {
    let root = workspace_root().canonicalize()?;
    let name = platform.name();
    let image = format!("bleproxy-test-{name}");
    let rust_version = toolchain_channel()?;
    status(
        Command::new("docker")
            .current_dir(&root)
            .args(["build", "--platform", platform.docker_platform()])
            .args(["--build-arg", &format!("RUST_VERSION={rust_version}")])
            .args(["--tag", &image, "--file", platform.dockerfile(), "docker"]),
    )?;
    status(
        Command::new("docker")
            .args(["run", "--rm", "--platform", platform.docker_platform()])
            .args(["--volume", &format!("{}:/src:ro", root.display())])
            .args(["--workdir", "/src"])
            .args(["--volume", &format!("bleproxy-target-{name}:/target")])
            .args(["--env", "CARGO_TARGET_DIR=/target"])
            .args([
                "--volume",
                "bleproxy-cargo-registry:/usr/local/cargo/registry",
            ])
            .args([&image, "cargo"])
            .args(platform.test_args()),
    )
}

/// Returns the Rust version pinned in `rust-toolchain.toml`.
fn toolchain_channel() -> Result<String> {
    let path = workspace_root().join("rust-toolchain.toml");
    let text = std::fs::read_to_string(&path)?;
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "channel")
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .ok_or_else(|| format!("no toolchain channel in {}", path.display()).into())
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
    fn every_listed_task_parses() {
        for (usage, _) in TASKS {
            let mut words = usage.split_whitespace();
            let name = words.next().unwrap();
            if words.next() == Some("<platform>") {
                for platform in Platform::ALL {
                    let task = parse(name, &[platform.name().into()]).unwrap();
                    assert_eq!(task, Task::Docker(platform));
                }
            } else {
                parse(name, &[]).unwrap();
            }
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

    #[test]
    fn docker_requires_a_known_platform() {
        assert!(parse("docker", &[]).is_err());
        assert!(parse("docker", &["solaris".into()]).is_err());
        assert!(parse("docker", &["windows".into(), "extra".into()]).is_err());
    }

    #[test]
    fn reads_pinned_toolchain() {
        let channel = toolchain_channel().unwrap();
        assert!(
            channel.split('.').all(|part| part.parse::<u32>().is_ok()),
            "{channel}"
        );
    }
}
