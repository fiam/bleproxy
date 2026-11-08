# Release image for the bleproxy host binary.
#
#   docker build -t bleproxy .
#   docker run --rm bleproxy --version
#
# Multi-arch images (linux/amd64, linux/arm64) are published by the release
# workflow, which builds each architecture natively.

# Keep in sync with rust-toolchain.toml (checked by the xtask tests).
ARG RUST_VERSION=1.99.1

FROM rust:${RUST_VERSION}-slim-trixie AS builder
ARG RUST_VERSION
# Use the image's toolchain directly instead of letting rust-toolchain.toml
# pull in development components such as clippy and rustfmt.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION}
WORKDIR /src
COPY . .
RUN cargo build --release --locked --package bleproxy \
    && cp target/release/bleproxy /usr/local/bin/bleproxy

FROM gcr.io/distroless/cc-debian13:nonroot
LABEL org.opencontainers.image.title="bleproxy" \
      org.opencontainers.image.description="Bluetooth LE proxy for Home Assistant, speaking the ESPHome Native API." \
      org.opencontainers.image.source="https://github.com/fiam/bleproxy" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --from=builder /usr/local/bin/bleproxy /usr/local/bin/bleproxy
ENTRYPOINT ["/usr/local/bin/bleproxy"]
