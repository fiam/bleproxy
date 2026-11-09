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

python3 -m dbusmock --system --template bluez5 &
for _ in $(seq 50); do
    if dbus-send --system --dest=org.bluez --print-reply / \
        org.freedesktop.DBus.Introspectable.Introspect > /dev/null 2>&1; then
        break
    fi
    sleep 0.1
done

exec /opt/e2e/bin/python /src/docker/e2e/test.py "$CARGO_TARGET_DIR/debug/bleproxy"
