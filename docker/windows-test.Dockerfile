# Runs the test suite for x86_64-pc-windows-gnu under Wine.
# Used by `cargo xtask docker windows`; build with --platform linux/amd64.
ARG RUST_VERSION
FROM rust:${RUST_VERSION}-slim-trixie

RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 wine wine64 \
    && rm -rf /var/lib/apt/lists/*

# Components requested by rust-toolchain.toml, plus the Windows target.
RUN rustup component add clippy rustfmt \
    && rustup target add x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine \
    WINEDEBUG=-all \
    XDG_RUNTIME_DIR=/tmp

# Create the Wine prefix once at build time instead of on every test run.
# Disabling mscoree/mshtml skips the Mono and Gecko installers, which
# otherwise wait for a prompt that never comes in a headless container.
RUN WINEDLLOVERRIDES="mscoree,mshtml=" wineboot --init && wineserver --wait
