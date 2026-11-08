# Runs the test suite on Linux. Used by `cargo xtask docker linux-<arch>`.
ARG RUST_VERSION
FROM rust:${RUST_VERSION}-slim-trixie

# Components requested by rust-toolchain.toml; installing them here keeps
# rustup from downloading them on every run.
RUN rustup component add clippy rustfmt
