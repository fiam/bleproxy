"""End-to-end test: mock BlueZ -> bleproxy -> aioesphomeapi.

Drives a python-dbusmock BlueZ the way a real adapter would report
advertisements, and checks that Home Assistant's API client receives them
from bleproxy. Run through `cargo xtask e2e`.
"""

import asyncio
import signal
import subprocess
import sys

import dbus
from aioesphomeapi import APIClient

PORT = 6053
ADAPTER_ADDRESS = "00:01:02:03:04:05"  # what the mock assigns to hci0
DEVICE_ADDRESS = "A4:C1:38:00:11:22"
TIMEOUT = 10
NOBODY = 65534


def step(message):
    print(f"==> {message}", flush=True)


def mock_bluez():
    bus = dbus.SystemBus()
    mock = dbus.Interface(bus.get_object("org.bluez", "/"), "org.bluez.Mock")
    mock.AddAdapter("hci0", "e2e")
    return bus, mock


def adapter_calls(bus):
    adapter = bus.get_object("org.bluez", "/org/bluez/hci0")
    calls = dbus.Interface(adapter, "org.freedesktop.DBus.Mock").GetCalls()
    return {str(method): args for _, method, args in calls}


async def connect():
    client = APIClient("127.0.0.1", PORT, password=None)
    for _ in range(50):
        try:
            await client.connect(login=True)
            return client
        except Exception:  # noqa: BLE001 - still starting up
            await asyncio.sleep(0.2)
    raise SystemExit("could not connect to bleproxy")


async def wait_for(advertisements, predicate, what):
    async with asyncio.timeout(TIMEOUT):
        while True:
            for advertisement in advertisements:
                if predicate(advertisement):
                    return advertisement
            await asyncio.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


def monitor_properties(bus, pid, root):
    """Reads bleproxy's advertisement monitor, as BlueZ would."""
    names = dbus.Interface(
        bus.get_object("org.freedesktop.DBus", "/org/freedesktop/DBus"),
        "org.freedesktop.DBus",
    )
    for name in names.ListNames():
        if name.startswith(":") and names.GetConnectionUnixProcessID(name) == pid:
            manager = dbus.Interface(
                bus.get_object(name, root), "org.freedesktop.DBus.ObjectManager"
            )
            objects = manager.GetManagedObjects()
            (interfaces,) = objects.values()
            return interfaces["org.bluez.AdvertisementMonitor1"]
    raise AssertionError("bleproxy is not on the bus")


async def check_scanning_started(bus, mode, pid):
    async with asyncio.timeout(TIMEOUT):
        while not {"StartDiscovery", "RegisterMonitor"} & adapter_calls(bus).keys():
            await asyncio.sleep(0.05)
    calls = adapter_calls(bus)
    if mode == "active":
        discovery_filter = calls["SetDiscoveryFilter"][0]
        assert discovery_filter["Transport"] == "le", discovery_filter
        assert discovery_filter["DuplicateData"], discovery_filter
        assert "RegisterMonitor" not in calls, calls
    else:
        assert "StartDiscovery" not in calls, calls
        (root,) = calls["RegisterMonitor"]
        monitor = monitor_properties(bus, pid, root)
        assert monitor["Type"] == "or_patterns", monitor
        patterns = [(int(s), int(t), bytes(c)) for s, t, c in monitor["Patterns"]]
        assert (0, 0x01, b"\x06") in patterns, patterns


async def main(binary, mode):
    step(f"testing {mode} scanning")
    bus, mock = mock_bluez()
    step("starting bleproxy")
    # Unprivileged, as in deployments: the bus policy must let it in.
    proxy = subprocess.Popen(
        [
            binary,
            "--no-mdns",
            "--listen",
            f"127.0.0.1:{PORT}",
            "--scan-mode",
            mode,
            "--log-level",
            "debug",
        ],
        user=NOBODY,
        group=NOBODY,
        extra_groups=[],
    )
    try:
        client = await connect()
        step("checking device info")
        info = await client.device_info()
        assert info.bluetooth_mac_address == ADAPTER_ADDRESS, info
        assert info.mac_address == ADAPTER_ADDRESS, info
        assert info.name == "bleproxy-030405", info
        assert info.project_name == "fiam.bleproxy", info
        flags = info.bluetooth_proxy_feature_flags_compat(client.api_version)
        assert flags == 0b10_0001, flags

        received = []
        client.subscribe_bluetooth_le_raw_advertisements(
            lambda response: received.extend(response.advertisements)
        )

        step("checking that scanning was started")
        await check_scanning_started(bus, mode, proxy.pid)

        address = int(DEVICE_ADDRESS.replace(":", ""), 16)
        step("adding a device")
        path = mock.AddDevice("hci0", DEVICE_ADDRESS, "ATC_1122")
        first = await wait_for(
            received, lambda a: a.address == address, "the new device"
        )
        assert first.rssi == -79, first
        assert b"\x09ATC_1122" in first.data, first.data

        step("updating its advertisement")
        device = dbus.Interface(
            bus.get_object("org.bluez", path), "org.freedesktop.DBus.Mock"
        )
        # Emit the signal bluetoothd sends when an advertisement arrives.
        # (UpdateProperties can't wrap dictionaries in some dbusmock versions.)
        device.EmitSignal(
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            "sa{sv}as",
            [
                "org.bluez.Device1",
                {
                    "RSSI": dbus.Int16(-60, variant_level=1),
                    "ManufacturerData": dbus.Dictionary(
                        {dbus.UInt16(0x004C): dbus.Array([2, 0x15], signature="y", variant_level=1)},
                        signature="qv",
                        variant_level=1,
                    ),
                },
                dbus.Array([], signature="s"),
            ],
        )
        update = await wait_for(
            received,
            lambda a: a.address == address and a.rssi == -60,
            "the updated advertisement",
        )
        assert update.data.startswith(bytes([5, 0xFF, 0x4C, 0x00, 0x02, 0x15])), (
            update.data.hex()
        )
        assert update.address_type == 0, update

        await client.disconnect()
    finally:
        step("stopping bleproxy")
        proxy.send_signal(signal.SIGTERM)
        code = proxy.wait(TIMEOUT)
    assert code == 0, f"bleproxy exited with {code}"
    step(f"{mode} scanning passed")


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1], sys.argv[2]))
