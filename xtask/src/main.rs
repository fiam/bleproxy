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
    Msrv,
    Ci,
    Docker(Platform),
    Esp32(Chip),
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
    (
        "msrv",
        "Check the host workspace with the minimum supported Rust",
    ),
    ("ci", "Run every check CI runs on the host"),
    (
        "docker <platform>",
        "Run the tests in a container: linux-amd64, linux-arm64 or windows",
    ),
    (
        "esp32 [chip]",
        "Lint and build the firmware: esp32c3 (default) or esp32s3",
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

/// A microcontroller the firmware can be built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chip {
    Esp32c3,
    Esp32s3,
}

impl Chip {
    const ALL: [Self; 2] = [Self::Esp32c3, Self::Esp32s3];

    /// Name of the chip, which is also the firmware's feature name.
    const fn name(self) -> &'static str {
        match self {
            Self::Esp32c3 => "esp32c3",
            Self::Esp32s3 => "esp32s3",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|chip| chip.name() == name)
    }

    const fn target(self) -> &'static str {
        match self {
            Self::Esp32c3 => "riscv32imc-unknown-none-elf",
            Self::Esp32s3 => "xtensa-esp32s3-none-elf",
        }
    }

    /// Whether the chip needs esp-rs's Xtensa toolchain instead of the
    /// pinned upstream one.
    const fn is_xtensa(self) -> bool {
        matches!(self, Self::Esp32s3)
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
        "msrv" => Task::Msrv,
        "ci" => Task::Ci,
        "docker" => return parse_docker(args),
        "esp32" => return parse_esp32(args),
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

fn parse_esp32(args: &[String]) -> Result<Task> {
    let names = Chip::ALL.map(Chip::name).join(", ");
    match args {
        [] => Ok(Task::Esp32(Chip::Esp32c3)),
        [chip] => Chip::from_name(chip)
            .map(Task::Esp32)
            .ok_or_else(|| format!("unknown chip `{chip}`, expected one of: {names}").into()),
        _ => Err(format!("`esp32` takes at most one chip: {names}").into()),
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
        Task::Msrv => msrv(),
        Task::Ci => ci(),
        Task::Docker(platform) => docker(platform),
        Task::Esp32(chip) => esp32(chip),
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
        Task::Msrv,
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
///
/// This resolves the full dependency graph on purpose: with `--no-deps`,
/// cargo does not check the lockfile against the manifests.
fn metadata() -> Result {
    status(
        cargo_command()
            .args(["metadata", "--format-version=1", "--locked"])
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

/// Lints and builds the firmware for `chip` in release mode.
fn esp32(chip: Chip) -> Result {
    let cargo = || {
        if chip.is_xtensa() {
            let mut command = command("rustup");
            command.args(["run", XTENSA_TOOLCHAIN, "cargo"]);
            command
        } else {
            cargo_command()
        }
    };
    if chip.is_xtensa() {
        require_xtensa_toolchain(chip)?;
    } else {
        // Installs the target into the pinned toolchain if it is missing.
        status(command("rustup").args(["target", "add", chip.target()]))?;
    }

    let mut args = vec![
        "--package",
        "bleproxy-esp32",
        "--target",
        chip.target(),
        "--features",
        chip.name(),
        "--release",
        "--locked",
    ];
    if chip.is_xtensa() {
        // The Xtensa toolchain ships no prebuilt standard library.
        args.push("-Zbuild-std=core,alloc");
    }
    status(
        cargo()
            .arg("clippy")
            .args(&args)
            .args(["--", "-D", "warnings"]),
    )?;
    status(cargo().arg("build").args(&args))
}

/// Name of the rustup toolchain that espup installs.
const XTENSA_TOOLCHAIN: &str = "esp";

/// Fails with setup instructions unless espup's toolchain and linker are
/// available.
fn require_xtensa_toolchain(chip: Chip) -> Result {
    let has_toolchain = command("rustup")
        .args(["run", XTENSA_TOOLCHAIN, "rustc", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let linker = format!("xtensa-{}-elf-gcc", chip.name());
    let has_linker = Command::new(&linker)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if has_toolchain && has_linker {
        return Ok(());
    }
    Err(format!(
        "{} needs the esp-rs Xtensa toolchain and the `{linker}` linker. \
         Install them with `cargo install --locked espup && espup install`, \
         then load the environment with `. ~/export-esp.sh` (or \
         `~/export-esp.ps1` on Windows) before running this task",
        chip.name()
    )
    .into())
}

/// Checks the host workspace with the `rust-version` declared in
/// `Cargo.toml`, installing that toolchain if needed.
fn msrv() -> Result {
    let version = rust_version()?;
    status(
        toolchain_cargo(&version)
            .args(["check", "--locked", "--all-targets"])
            // Keep the regular target directory's cache intact.
            .args(["--target-dir", "target/msrv"]),
    )
}

/// Returns the Rust version pinned in `rust-toolchain.toml`.
fn toolchain_channel() -> Result<String> {
    toml_string("rust-toolchain.toml", "channel")
}

/// Returns the minimum supported Rust version declared in `Cargo.toml`.
fn rust_version() -> Result<String> {
    toml_string("Cargo.toml", "rust-version")
}

/// Returns the first `key = "value"` string in a workspace file.
///
/// Good enough for the flat, well-known files it reads, without pulling a
/// TOML parser into the task runner.
fn toml_string(file: impl AsRef<Path>, key: &str) -> Result<String> {
    let path = workspace_root().join(file);
    let text = std::fs::read_to_string(&path)?;
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .ok_or_else(|| format!("no `{key}` in {}", path.display()).into())
}

/// Runs cargo with `args` from the workspace root.
fn cargo<I, S>(args: I) -> Result
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    status(cargo_command().args(args))
}

/// Returns a cargo command for the workspace's pinned toolchain.
fn cargo_command() -> Command {
    command(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// Returns a cargo command for `toolchain`, installing it if needed.
fn toolchain_cargo(toolchain: &str) -> Command {
    let mut command = command("rustup");
    command.args(["run", "--install", toolchain, "cargo"]);
    command
}

/// Returns a command for `program` that runs from the workspace root.
fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
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
    fn esp32_defaults_to_c3_and_accepts_known_chips() {
        assert_eq!(parse("esp32", &[]).unwrap(), Task::Esp32(Chip::Esp32c3));
        for chip in Chip::ALL {
            let task = parse("esp32", &[chip.name().into()]).unwrap();
            assert_eq!(task, Task::Esp32(chip));
        }
        assert!(parse("esp32", &["esp8266".into()]).is_err());
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
    fn release_image_uses_pinned_toolchain() {
        let path = workspace_root().join("Dockerfile");
        let dockerfile = std::fs::read_to_string(path).unwrap();
        let expected = format!("ARG RUST_VERSION={}", toolchain_channel().unwrap());
        assert!(
            dockerfile.lines().any(|line| line == expected),
            "Dockerfile must declare `{expected}`"
        );
    }

    fn assert_version(version: &str) {
        assert!(
            version.split('.').all(|part| part.parse::<u32>().is_ok()),
            "{version}"
        );
    }

    #[test]
    fn release_please_bumps_every_package_in_the_lockfile() {
        // release-please's TOML updater cannot match names by prefix, so
        // release-please-config.json lists each workspace package.
        let config =
            std::fs::read_to_string(workspace_root().join("release-please-config.json")).unwrap();
        for dir in ["apps", "crates"] {
            for entry in std::fs::read_dir(workspace_root().join(dir)).unwrap() {
                let manifest = entry.unwrap().path().join("Cargo.toml");
                let name = toml_string(&manifest, "name").unwrap();
                assert!(
                    config.contains(&format!("@.name.value == '{name}'")),
                    "release-please-config.json does not bump `{name}` in Cargo.lock"
                );
            }
        }
    }

    #[test]
    fn reads_pinned_toolchain() {
        assert_version(&toolchain_channel().unwrap());
    }

    #[test]
    fn reads_rust_version() {
        assert_version(&rust_version().unwrap());
    }
}
