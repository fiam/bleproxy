# AGENTS.md

Guidance for coding agents working in this repository. Humans should start
with [README.md](README.md) and [CONTRIBUTING.md](CONTRIBUTING.md); this file
condenses what matters when making changes.

## Project

bleproxy is a Bluetooth Low Energy proxy for Home Assistant that speaks the
ESPHome Native API. One Rust workspace targets desktop/server hosts (Linux,
Windows, macOS) and microcontrollers (ESP32-C3, ESP32-S3).

| Path | What | Constraints |
| --- | --- | --- |
| `crates/bleproxy-core` | BLE domain types, proxy logic | `no_std`, `forbid(unsafe_code)`, no I/O |
| `crates/bleproxy-protocol` | ESPHome wire format | `no_std`, `forbid(unsafe_code)`, no I/O, no allocation in codecs |
| `apps/bleproxy` | host app on Tokio: API server, transports (plain, Noise), mDNS, scanner and connection control, BlueZ (Linux) | platform code lives here; lib + thin bin |
| `apps/bleproxy-esp32` | firmware (esp-hal, esp-rtos, Embassy) | not in `default-members`; built via `cargo xtask esp32` |
| `xtask` | task runner (std only) | no dependencies |
| `docker/` | test images for `cargo xtask docker` | |

## Commands

Always use `cargo xtask`, never ad-hoc command lines, so results match CI.

```sh
cargo xtask fmt                 # format (run before committing)
cargo xtask ci                  # all host checks: fmt, metadata, clippy, msrv, build, test, docs, deny, deps
cargo xtask test                # just the tests
cargo xtask docker windows      # tests for Windows under Wine (also linux-amd64, linux-arm64)
cargo xtask esp32 [esp32s3]     # firmware lint and build; S3 needs `. ~/export-esp.sh`
cargo xtask e2e                 # mock BlueZ + bleproxy + aioesphomeapi: scanning, encryption, GATT
```

**Definition of done:** `cargo xtask ci` passes. If you touched platform
code, the relevant `docker`, `e2e` or `esp32` task passes too. Linux-only
code (`apps/bleproxy/src/bluez.rs` and `bluez/`) can be linted from other hosts with
`cargo clippy --target aarch64-unknown-linux-gnu --all-targets`.

## Rules

- **Layering.**
  - Shared crates must not depend on an OS, an async runtime or a radio. They
    are sans-I/O: types, codecs and state machines (like `Session`) that the
    apps feed. Add a trait to a shared crate only once two platforms need it.
  - Don't add `#[cfg(target_os = ...)]` to shared crates.
- **Errors.**
  - Return typed errors (`thiserror`, `default-features = false`).
  - Don't use `unwrap`/`expect`/`panic!` outside tests; Clippy flags them.
- **Lints.**
  - Don't silence lints to get green. If an exception is truly right, use a
    narrowly scoped `#[allow(lint, reason = "...")]`.
  - Never add workspace-wide `allow`s without a comment in `Cargo.toml`.
- **Docs.** Every public item needs a doc comment (`missing_docs` is on, and
  CI denies warnings).
- **Tests.** Write deterministic tests: no network, real adapters or sleeps.
  New logic in the shared crates comes with unit tests.
- **No placeholders.** Don't add `todo!()`, stub backends or speculative
  traits. Add code with its first real use.
- **Dependencies.**
  - Put shared versions in `[workspace.dependencies]`, and don't use wildcard
    versions.
  - Commit `Cargo.lock` with the change.
  - `cargo xtask deny` and `cargo xtask deps` must pass.
  - Don't widen `deny.toml` (licenses, ignores, skips) without a stated reason.
- **Toolchain and MSRV.**
  - Development uses the version pinned in `rust-toolchain.toml`.
  - `rust-version` in `Cargo.toml` (1.95) is the supported minimum, checked by
    `cargo xtask msrv`. Don't use newer standard library APIs.
  - Keep `rust-version` below the esp-rs Xtensa toolchain's version (currently
    1.97), or the ESP32-S3 build breaks.
- **New crates.** Add the package name to the `Cargo.lock` filter in
  `release-please-config.json` (an xtask test enforces this). Host crates must
  be covered by `default-members`.

## Commits and PRs

- Commit in small, logical steps. Each commit passes `cargo xtask ci`.
- Use Conventional Commits (`feat`, `fix`, `perf`, `refactor`, `docs`, `test`,
  `build`, `ci`, `chore`), with an optional scope (`core`, `protocol`,
  `bleproxy`, `esp32`, `xtask`, `docker`).
  - Subjects are imperative and lowercase, with no trailing period.
  - Include a body explaining *why*.
- release-please derives versions and `CHANGELOG.md` from these commits. Never
  edit versions or `CHANGELOG.md` by hand.
- PRs are squash-merged, so the PR title must itself be a valid conventional
  commit.

## Gotchas

- `cargo clippy --workspace` (or any `--workspace` command) includes the
  firmware and fails on the host. Use the xtask tasks, which pick the right
  members.
- The ESP32-S3 build uses the `esp` toolchain via `rustup run esp` with
  `-Zbuild-std=core,alloc`. `cargo xtask esp32 esp32s3` handles that, but the
  shell still needs espup's environment (`. ~/export-esp.sh`).
- The release `Dockerfile` default `RUST_VERSION` must match
  `rust-toolchain.toml`. An xtask test checks it.
- GitHub Actions are pinned to commit SHAs with a version comment. Keep both
  in sync when updating by hand (Dependabot normally does it).
- The system bus only accepts users it can resolve, which rules out
  `DynamicUser=` in the systemd unit and distroless's `nonroot` user in the
  image (the image runs as `nobody`). The image must also set
  `DBUS_SYSTEM_BUS_ADDRESS`, since distroless lacks `/var/run`. The e2e test
  runs a real system bus with BlueZ's policy to catch regressions here.
- BlueZ differs from ESPHome in ways clients notice: it refuses writes to
  the client characteristic configuration descriptor, and a read changes a
  characteristic's `Value` just like a notification does. `bluez/gatt.rs`
  documents how the proxy hides this; the e2e mock reproduces both.
- Wire compatibility is checked against aioesphomeapi, Home Assistant's
  client: protobuf test vectors come from its generated classes, and the e2e
  test drives the real binary with it. Encryption mismatches must produce the
  same replies as ESPHome so clients can explain them. Never log keys.
