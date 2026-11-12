#!/bin/sh
# Builds bleproxy, starts a system bus with a mock BlueZ on it, and runs the
# end-to-end test against them. Expects the source tree in /src.
set -eu

cargo build --locked --package bleproxy

# A real system bus with BlueZ's access rules, so bleproxy faces the same
# policy as on a host. The mock BlueZ runs as root, like bluetoothd.
mkdir -p /usr/share/dbus-1/system.d /run/dbus
cp /src/docker/e2e/bluez-policy.conf /usr/share/dbus-1/system.d/bluez.conf
dbus-uuidgen --ensure
dbus-daemon --system --fork

# Each run gets a fresh mock BlueZ: both scan modes, then active scanning
# over an encrypted API with an address filter. Every run also connects to
# a device. The last run checks that passive scanning falls back to active
# without BlueZ's advertisement monitors.
for run in active passive "active encrypted filtered" "passive nomonitor"; do
    python3 -m dbusmock --system --template bluez5 &
    mock=$!
    for _ in $(seq 50); do
        if dbus-send --system --dest=org.bluez --print-reply / \
            org.freedesktop.DBus.Introspectable.Introspect > /dev/null 2>&1; then
            break
        fi
        sleep 0.1
    done
    # shellcheck disable=SC2086 # the run's words are separate arguments
    /opt/e2e/bin/python /src/docker/e2e/test.py "$CARGO_TARGET_DIR/debug/bleproxy" $run
    kill "$mock"
    wait "$mock" || true
done
