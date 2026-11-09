# End-to-end test environment: a mock BlueZ (python-dbusmock) on a private
# D-Bus, and Home Assistant's API client (aioesphomeapi).
# Used by `cargo xtask e2e`.
ARG RUST_VERSION
FROM rust:${RUST_VERSION}-slim-trixie

RUN apt-get update \
    && apt-get install -y --no-install-recommends dbus python3-dbusmock python3-venv \
    && rm -rf /var/lib/apt/lists/*

# The virtualenv sees the distribution's dbus bindings and adds the client.
RUN python3 -m venv --system-site-packages /opt/e2e \
    && /opt/e2e/bin/pip install --no-cache-dir aioesphomeapi==46.28.0

RUN rustup component add clippy rustfmt
