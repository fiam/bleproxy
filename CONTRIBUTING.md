# Contributing to bleproxy

Thanks for helping! This document describes how changes get from your editor
to a release. The [README](README.md#development) covers tool installation.

## The short version

```sh
cargo xtask fmt     # format
cargo xtask ci      # everything CI checks on the host
```

If `cargo xtask ci` passes, CI almost certainly will too. Changes to
platform-specific code should also run the relevant
`cargo xtask docker <platform>` and `cargo xtask esp32 [chip]` tasks.

## Code style

- **Formatting** is `rustfmt` with the settings in [`rustfmt.toml`](rustfmt.toml).
  Don't fight it.
- **Lints** are configured once in `[workspace.lints]` in the root
  [`Cargo.toml`](Cargo.toml), and every crate inherits them with
  `[lints] workspace = true`. CI denies all warnings.
  - Fix the code rather than silencing a lint. If an exception is genuinely
    right, scope it as tightly as possible and say why:
    `#[allow(clippy::lint_name, reason = "...")]`.
  - Workspace-wide exceptions need a comment in `Cargo.toml` explaining them.
- **Errors** are typed. Library code returns `Result` with an error enum
  (`thiserror` works in `no_std`). `unwrap`, `expect` and `panic!` are flagged
  outside tests. Use them only when the invariant is local and obvious, with an
  `#[allow(..., reason = "...")]`.
- **Unsafe** code is forbidden in the shared crates and denied elsewhere. If
  firmware ever needs it, keep it minimal and document the safety argument.
- **Documentation**: every public item gets a doc comment (`missing_docs` is
  on). Prefer examples that run as doctests.
- **Keep the layers apart.**
  - `bleproxy-core` and `bleproxy-protocol` are `no_std` and must not depend on
    an OS, a runtime or a radio.
  - Platform code lives in `apps/*`, behind traits defined by the shared
    crates. Don't put `#[cfg(target_os = ...)]` in shared code.
- **No placeholders.** Don't add stub implementations, `todo!()` or speculative
  abstractions. Add code together with its first real use and its tests.

## Testing

- Unit tests live next to the code (`#[cfg(test)] mod tests`). End-to-end tests
  for binaries live in `apps/<app>/tests/`.
- Tests must be deterministic: no network, no real Bluetooth adapters, no
  sleeps or timing assumptions.
- `cargo xtask test` uses nextest when available and always runs doctests.
- Cross-platform behavior: `cargo xtask docker linux-amd64`, `linux-arm64` and
  `windows` (MinGW + Wine) run the full suite in containers.
- Firmware has no host test harness. Logic worth testing belongs in the shared
  crates, where it is tested on the host.

## Commits

- **Small and logical.** Each commit does one thing and builds and passes
  `cargo xtask ci` on its own. Split refactors from behavior changes.
- **[Conventional Commits][conventional]**, because release-please reads them
  to pick the next version and write the changelog:

  | Type | Use for | Release effect |
  | --- | --- | --- |
  | `feat` | user-visible functionality | minor bump |
  | `fix` | bug fixes | patch bump |
  | `perf` | performance improvements | patch bump |
  | `refactor` | restructuring without behavior change | none |
  | `docs` | documentation only | none |
  | `test` | tests only | none |
  | `build` | build system, dependencies, tooling (`xtask`, Docker) | none |
  | `ci` | GitHub workflows and automation | none |
  | `chore` | anything else | none |

  Use a scope when it helps (`feat(core): ...`, `fix(protocol): ...`,
  `build(xtask): ...`). Mark breaking changes with `!` (`feat(protocol)!: ...`)
  or a `BREAKING CHANGE:` footer.
- **Write a body.** The subject says *what*; the body explains *why*, and
  anything a reviewer or future reader would otherwise have to rediscover.
  Wrap it at 72 columns.

## Pull requests

- PRs are **squash-merged**, and the PR title becomes the commit on `main`, so
  the title must be a valid conventional commit. A check enforces this.
- Keep PRs focused. Unrelated cleanups go in their own PR.
- Describe what changed, why, and how you tested it, including which platforms
  or chips you ran it on.
- CI must be green. Don't merge with failing or skipped checks.

## Dependencies

- Add a dependency only when it clearly pays for itself. Prefer crates that
  support `no_std` for anything the shared crates use.
- Versions shared between crates go in `[workspace.dependencies]`, and crates
  use `dep.workspace = true`. Never use wildcard versions.
- Commit `Cargo.lock` changes together with the manifest change that caused
  them.
- `cargo xtask deny` must pass:
  - **licenses**: new licenses must be permissive and compatible with
    MIT OR Apache-2.0. Add them to `deny.toml` deliberately.
  - **advisories**: never ignore a vulnerability. An `ignore` entry needs a
    reason and should be revisited.
  - **bans**: the host dependency graph must stay free of duplicate versions.
  - **sources**: crates.io only.
- `cargo xtask deps` must pass (no unused dependencies).
- Dependabot opens weekly update PRs. esp-rs and Embassy crates are grouped
  because they have to be upgraded together.

## Adding a crate

1. Create it under `crates/` (shared) or `apps/` (binaries). Inherit the
   workspace package metadata and lints, as the existing crates do.
2. Add its name to the `Cargo.lock` filter in
   [`release-please-config.json`](release-please-config.json). An xtask test
   fails until you do.
3. If it is a host crate, make sure `default-members` in `Cargo.toml` picks it
   up.

## Releases

Maintainers merge the release PR that release-please keeps open. Everything
else (tagging, GitHub Release, binaries, container images, provenance) is
automated. See [Releases](README.md#releases).

## License

By contributing, you agree that your contributions are dual licensed under
MIT OR Apache-2.0, without any additional terms or conditions.

[conventional]: https://www.conventionalcommits.org
