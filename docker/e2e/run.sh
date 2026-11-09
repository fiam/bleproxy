#!/bin/sh
# Builds bleproxy, starts a private D-Bus with a mock BlueZ on it, and runs
# the end-to-end test against them. Expects the source tree in /src.
set -eu

cargo build --locked --package bleproxy

# A private bus stands in for the system bus; bleproxy, dbus-send and
# dbusmock all honor DBUS_SYSTEM_BUS_ADDRESS.
DBUS_SYSTEM_BUS_ADDRESS="$(dbus-daemon --session --fork --print-address)"
export DBUS_SYSTEM_BUS_ADDRESS

python3 -m dbusmock --system --template bluez5 &
for _ in $(seq 50); do
    if dbus-send --system --dest=org.bluez --print-reply / \
        org.freedesktop.DBus.Introspectable.Introspect > /dev/null 2>&1; then
        break
    fi
    sleep 0.1
done

exec /opt/e2e/bin/python /src/docker/e2e/test.py "$CARGO_TARGET_DIR/debug/bleproxy"
