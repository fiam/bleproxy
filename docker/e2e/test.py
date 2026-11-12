"""End-to-end test: mock BlueZ -> bleproxy -> aioesphomeapi.

Drives a python-dbusmock BlueZ the way a real adapter would report
advertisements and serve a device's GATT database, and checks that Home
Assistant's API client receives the advertisements and can use the device
through bleproxy. Run through `cargo xtask e2e`.
"""

import asyncio
import base64
import datetime
import os
import signal
import subprocess
import sys

import dbus
from aioesphomeapi import APIClient
from aioesphomeapi.core import (
    BluetoothGATTAPIError,
    InvalidEncryptionKeyAPIError,
    RequiresEncryptionAPIError,
)
from aioesphomeapi.model import (
    BluetoothScannerMode,
    BluetoothScannerState,
    EntityCategory,
    LogLevel,
    SensorInfo,
    SensorState,
    SensorStateClass,
    SwitchInfo,
    SwitchState,
    TextSensorInfo,
    TextSensorState,
)

PORT = 6053
ADAPTER_ADDRESS = "00:01:02:03:04:05"  # what the mock assigns to hci0
DEVICE_ADDRESS = "A4:C1:38:00:11:22"
SECOND_ADDRESS = "A4:C1:38:00:33:44"
BLOCKED_ADDRESS = "A4:C1:38:00:55:66"  # left out by the address filter
KNOWN_ADDRESS = "A4:C1:38:00:77:88"  # known to BlueZ before bleproxy starts
TIMEOUT = 10
MODES = {"active": BluetoothScannerMode.ACTIVE, "passive": BluetoothScannerMode.PASSIVE}
NOBODY = 65534
MOCK = "org.freedesktop.DBus.Mock"
GATT_SERVICE = "org.bluez.GattService1"
GATT_CHARACTERISTIC = "org.bluez.GattCharacteristic1"
GATT_DESCRIPTOR = "org.bluez.GattDescriptor1"


def uuid16(alias):
    return f"0000{alias:04x}-0000-1000-8000-00805f9b34fb"


def step(message):
    print(f"==> {message}", flush=True)


def mock_bluez():
    bus = dbus.SystemBus()
    mock = dbus.Interface(bus.get_object("org.bluez", "/"), "org.bluez.Mock")
    mock.AddAdapter("hci0", "e2e")
    return bus, mock


def adapter_calls(bus):
    return {str(method): args for method, args in calls(bus, "/org/bluez/hci0")}


def calls(bus, path):
    """The methods called on a mock object, in order, with their arguments."""
    mock = dbus.Interface(bus.get_object("org.bluez", path), MOCK)
    return [(str(method), args) for _, method, args in mock.GetCalls()]


async def wait_for_call(bus, path, method, count=1):
    async with asyncio.timeout(TIMEOUT):
        while True:
            made = [args for name, args in calls(bus, path) if name == method]
            if len(made) >= count:
                return made[count - 1]
            await asyncio.sleep(0.05)


def add_gatt_database(bus, device):
    """Exports a battery service the way BlueZ does once it has resolved a
    device's services: object paths end in the attribute handles."""
    root = dbus.Interface(bus.get_object("org.bluez", "/"), MOCK)
    service = f"{device}/service0028"
    level = f"{service}/char0029"
    secret = f"{service}/char002d"
    root.AddObject(service, GATT_SERVICE, {
        "UUID": uuid16(0x180F),
        "Device": dbus.ObjectPath(device),
        "Primary": True,
    }, [])
    # Like BlueZ, reads update Value, which looks just like a notification.
    root.AddObject(level, GATT_CHARACTERISTIC, {
        "UUID": uuid16(0x2A19),
        "Service": dbus.ObjectPath(service),
        "Flags": dbus.Array(["read", "write", "write-without-response", "notify"], signature="s"),
        "MTU": dbus.UInt16(247),
        "Value": dbus.Array([], signature="y"),
    }, [
        ("ReadValue", "a{sv}", "ay",
         "self.Set('org.bluez.GattCharacteristic1', 'Value', dbus.Array([0x64], signature='y'))\n"
         "ret = dbus.Array([0x64], signature='y')"),
        ("WriteValue", "aya{sv}", "", ""),
        ("StartNotify", "", "", ""),
        ("StopNotify", "", "", ""),
    ])
    for handle, uuid, value in [(0x2B, 0x2902, b"\x00\x00"), (0x2C, 0x2901, b"Battery")]:
        root.AddObject(f"{level}/desc{handle:04x}", GATT_DESCRIPTOR, {
            "UUID": uuid16(uuid),
            "Characteristic": dbus.ObjectPath(level),
            "Value": dbus.Array(value, signature="y"),
        }, [
            ("ReadValue", "a{sv}", "ay", f"ret = dbus.Array({list(value)!r}, signature='y')"),
            ("WriteValue", "aya{sv}", "", ""),
        ])
    root.AddObject(secret, GATT_CHARACTERISTIC, {
        "UUID": uuid16(0x2A00),
        "Service": dbus.ObjectPath(service),
        "Flags": dbus.Array(["read"], signature="s"),
        "Value": dbus.Array([], signature="y"),
    }, [
        ("ReadValue", "a{sv}", "ay",
         "raise dbus.exceptions.DBusException('Read not permitted', "
         "name='org.bluez.Error.NotPermitted')"),
    ])
    return level


async def resolve_services_on_connect(bus, device, count):
    """Plays BlueZ: once bleproxy connects, report the services resolved."""
    await wait_for_call(bus, device, "Connect", count)
    mock = dbus.Interface(bus.get_object("org.bluez", device), MOCK)
    mock.UpdateProperties("org.bluez.Device1", {"ServicesResolved": True})


async def check_connections(bus, client, device, flags):
    address = int(DEVICE_ADDRESS.replace(":", ""), 16)
    level = add_gatt_database(bus, device)

    slots = []
    client.subscribe_bluetooth_connections_free(lambda *update: slots.append(update))
    await wait_for(slots, lambda s: s == (3, 3, []), "the free slots")

    step("connecting to the device")
    states = []
    resolving = asyncio.create_task(resolve_services_on_connect(bus, device, 1))
    await client.bluetooth_device_connect(
        address,
        lambda *state: states.append(state),
        feature_flags=flags,
        address_type=0,
    )
    await resolving
    assert states == [(True, 247, 0)], states
    await wait_for(slots, lambda s: s == (2, 3, [address]), "the slot to be taken")

    step("listing its services")
    services = (await client.bluetooth_gatt_get_services(address)).services
    assert [(s.uuid, s.handle) for s in services] == [(uuid16(0x180F), 0x28)], services
    characteristics = services[0].characteristics
    # Characteristics go by their value handle, one past the declaration's.
    assert [(c.uuid, c.handle, c.properties) for c in characteristics] == [
        (uuid16(0x2A19), 0x2A, 0x1E),
        (uuid16(0x2A00), 0x2E, 0x02),
    ], characteristics
    descriptors = characteristics[0].descriptors
    assert [(d.uuid, d.handle) for d in descriptors] == [
        (uuid16(0x2902), 0x2B),
        (uuid16(0x2901), 0x2C),
    ], descriptors

    step("reading and writing")
    assert await client.bluetooth_gatt_read(address, 0x2A) == b"\x64"
    assert await client.bluetooth_gatt_read_descriptor(address, 0x2C) == b"Battery"
    await client.bluetooth_gatt_write(address, 0x2A, b"\x01\x02", response=True)
    await client.bluetooth_gatt_write(address, 0x2A, b"\x03", response=False)
    data, options = await wait_for_call(bus, level, "WriteValue", 2)
    assert bytes(data) == b"\x03" and options["type"] == "command", (data, options)
    data, options = (await wait_for_call(bus, level, "WriteValue", 1))
    assert bytes(data) == b"\x01\x02" and options["type"] == "request", (data, options)
    try:
        await client.bluetooth_gatt_read(address, 0x2E)
        raise AssertionError("reading a protected characteristic should fail")
    except BluetoothGATTAPIError as err:
        assert err.error.error == 2, err  # read not permitted

    step("receiving notifications")
    notifications = []
    await client.bluetooth_gatt_start_notify(
        address, 0x2A, lambda handle, data: notifications.append(bytes(data))
    )
    await wait_for_call(bus, level, "StartNotify")
    # Clients configure the CCCD after asking for notifications; BlueZ does
    # that itself and refuses the write, so bleproxy must not pass it on.
    await client.bluetooth_gatt_write_descriptor(address, 0x2B, b"\x01\x00")
    descriptor_writes = calls(bus, f"{level}/desc002b")
    assert not descriptor_writes, descriptor_writes
    # A read changes Value too, but must not pass for a notification.
    assert await client.bluetooth_gatt_read(address, 0x2A) == b"\x64"
    characteristic = dbus.Interface(bus.get_object("org.bluez", level), MOCK)
    characteristic.EmitSignal(
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        "sa{sv}as",
        [
            GATT_CHARACTERISTIC,
            {"Value": dbus.Array([0x63], signature="y", variant_level=1)},
            dbus.Array([], signature="s"),
        ],
    )
    await wait_for(notifications, lambda n: True, "a notification")
    assert notifications == [b"\x63"], notifications

    step("disconnecting")
    await client.bluetooth_device_disconnect(address)
    await wait_for(slots, lambda s: s == (3, 3, []), "the slot to be freed")

    step("releasing devices of clients that go away")
    resolving = asyncio.create_task(resolve_services_on_connect(bus, device, 2))
    await client.bluetooth_device_connect(
        address, lambda *state: None, feature_flags=flags, address_type=0
    )
    await resolving
    await client.disconnect()
    await wait_for_call(bus, device, "Disconnect", 2)


async def connect(key=None):
    for _ in range(50):
        client = APIClient("127.0.0.1", PORT, password=None, noise_psk=key)
        try:
            await client.connect(login=True)
            return client
        except (InvalidEncryptionKeyAPIError, RequiresEncryptionAPIError):
            raise
        except Exception:  # noqa: BLE001 - still starting up
            await asyncio.sleep(0.2)
    raise SystemExit("could not connect to bleproxy")


async def check_rejections(key):
    """Clients with the wrong key, or none, are told what is wrong."""
    wrong = base64.b64encode(os.urandom(32)).decode()
    for client_key, expected in [
        (wrong, InvalidEncryptionKeyAPIError),
        (None, RequiresEncryptionAPIError),
    ]:
        try:
            client = await connect(client_key)
        except expected:
            continue
        await client.disconnect()
        raise AssertionError(f"expected {expected.__name__} with key {client_key!r}")


async def wait_for(advertisements, predicate, what):
    async with asyncio.timeout(TIMEOUT):
        while True:
            for advertisement in advertisements:
                if predicate(advertisement):
                    return advertisement
            await asyncio.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


async def wait_for_new(items, seen, predicate, what):
    """Like wait_for, but only for items that arrive after the first `seen`."""
    async with asyncio.timeout(TIMEOUT):
        while True:
            for item in items[seen:]:
                if predicate(item):
                    return item
            await asyncio.sleep(0.05)


def monitor_properties(bus, pid, root):
    """Reads bleproxy's advertisement monitor, as BlueZ would."""
    names = dbus.Interface(
        bus.get_object("org.freedesktop.DBus", "/org/freedesktop/DBus"),
        "org.freedesktop.DBus",
    )
    for name in names.ListNames():
        if name.startswith(":") and names.GetConnectionUnixProcessID(name) == pid:
            # bleproxy has several bus connections; only the scanner's
            # serves the monitor, and the others don't answer at all.
            manager = dbus.Interface(
                bus.get_object(name, root, introspect=False),
                "org.freedesktop.DBus.ObjectManager",
            )
            try:
                objects = manager.GetManagedObjects(timeout=2)
            except dbus.exceptions.DBusException:
                continue
            (interfaces,) = objects.values()
            return interfaces["org.bluez.AdvertisementMonitor1"]
    raise AssertionError("bleproxy's advertisement monitor is not on the bus")


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


def device_sensor_ids(addresses):
    """The object IDs of the allowed devices' signal and last seen sensors."""
    ids = set()
    for address in addresses:
        suffix = address.replace(":", "").lower()
        ids |= {f"rssi_{suffix}", f"last_seen_{suffix}"}
    return ids


async def check_entities(client, allowed=()):
    """The proxy's own entities, listed and reported like ESPHome's.

    Returns the scanning switch and the list states keep arriving in.
    """
    step("checking the diagnostic sensors and the scanning switch")
    entities, services = await client.list_entities_services()
    assert not services, services
    by_id = {entity.object_id: entity for entity in entities}
    sensors = {"uptime", "advertisements", "connected_clients"}
    devices = device_sensor_ids(allowed)
    assert set(by_id) == sensors | devices | {"ble_scanning"}, by_id
    assert isinstance(by_id["uptime"], TextSensorInfo), by_id
    assert isinstance(by_id["advertisements"], SensorInfo), by_id
    assert by_id["advertisements"].state_class == SensorStateClass.TOTAL_INCREASING, by_id
    assert by_id["connected_clients"].state_class == SensorStateClass.MEASUREMENT, by_id
    assert all(by_id[s].entity_category == EntityCategory.DIAGNOSTIC for s in sensors), by_id
    switch = by_id["ble_scanning"]
    assert isinstance(switch, SwitchInfo), switch
    assert switch.entity_category == EntityCategory.CONFIG, switch

    states = []
    client.subscribe_states(states.append)
    async with asyncio.timeout(TIMEOUT):
        while len(states) < len(entities):
            await asyncio.sleep(0.05)
    by_key = {state.key: state for state in states}
    uptime = by_key[by_id["uptime"].key]
    assert isinstance(uptime, TextSensorState) and uptime.state.endswith("s"), uptime
    clients = by_key[by_id["connected_clients"].key]
    assert isinstance(clients, SensorState) and clients.state == 1, clients
    scanning = by_key[switch.key]
    assert isinstance(scanning, SwitchState) and scanning.state, scanning
    return switch, states


async def check_scanning_switch(client, bus, mode, switch, states, scanner_states):
    """Turning the BLE Scanning switch off stops the scan; on restarts it."""
    configured = MODES[mode]
    start = "StartDiscovery" if mode == "active" else "RegisterMonitor"
    stop = "StopDiscovery" if mode == "active" else "UnregisterMonitor"

    step("switching scanning off")
    seen_scanner, seen_states = len(scanner_states), len(states)
    client.switch_command(switch.key, False)
    await wait_for_new(
        scanner_states,
        seen_scanner,
        lambda s: s.state == BluetoothScannerState.STOPPED,
        "the stopped scanner",
    )
    await wait_for_call(bus, "/org/bluez/hci0", stop)
    await wait_for_new(
        states, seen_states, lambda s: s.key == switch.key and not s.state, "switch off"
    )

    step("switching scanning back on")
    seen_scanner, seen_states = len(scanner_states), len(states)
    client.switch_command(switch.key, True)
    await wait_for_call(bus, "/org/bluez/hci0", start, count=2)
    await wait_for_new(
        scanner_states,
        seen_scanner,
        lambda s: s.state == BluetoothScannerState.RUNNING and s.mode == configured,
        "the scanner running again",
    )
    await wait_for_new(
        states, seen_states, lambda s: s.key == switch.key and s.state, "switch on"
    )


async def check_device_sensors(key):
    """Allowed devices are named by label, by BlueZ's name, or by address,
    and report their signal and when they were last heard."""
    step("checking the allowed devices' sensors")
    client = await connect(key)
    entities, _ = await client.list_entities_services()
    by_name = {entity.name: entity for entity in entities}
    names = {
        "Thermo": DEVICE_ADDRESS,
        SECOND_ADDRESS: SECOND_ADDRESS,
        "ATC_7788": KNOWN_ADDRESS,
    }
    for name, address in names.items():
        rssi, last_seen = by_name[f"{name} RSSI"], by_name[f"{name} Last Seen"]
        suffix = address.replace(":", "").lower()
        assert rssi.object_id == f"rssi_{suffix}", rssi
        assert isinstance(rssi, SensorInfo) and rssi.device_class == "signal_strength", rssi
        assert rssi.unit_of_measurement == "dBm", rssi
        assert rssi.entity_category == EntityCategory.DIAGNOSTIC, rssi
        assert isinstance(last_seen, TextSensorInfo), last_seen
        assert last_seen.device_class == "timestamp", last_seen

    states = []
    client.subscribe_states(states.append)
    async with asyncio.timeout(TIMEOUT):
        while len(states) < len(entities):
            await asyncio.sleep(0.05)
    by_key = {state.key: state for state in states}
    # The device's last advertisement had an RSSI of -60.
    thermo = by_key[by_name["Thermo RSSI"].key]
    assert not thermo.missing_state and thermo.state == -60, thermo
    seen = by_key[by_name["Thermo Last Seen"].key]
    assert not seen.missing_state, seen
    heard = datetime.datetime.fromisoformat(seen.state)
    now = datetime.datetime.now(datetime.UTC)
    assert datetime.timedelta(0) <= now - heard < datetime.timedelta(minutes=1), seen
    await client.disconnect()


async def check_logs(key):
    """Subscribed clients get the configuration, then the proxy's logs."""
    step("streaming logs")
    client = await connect(key)
    lines = []
    client.subscribe_logs(
        lambda response: lines.append((response.level, response.message.decode())),
        log_level=LogLevel.LOG_LEVEL_INFO,
        dump_config=True,
    )
    await wait_for(
        lines,
        lambda line: line == (LogLevel.LOG_LEVEL_CONFIG, "[C][bleproxy]: Encryption: "
        + ("on" if key else "off")),
        "the configuration",
    )
    seen = len(lines)
    other = await connect(key)
    await wait_for_new(
        lines,
        seen,
        lambda line: line[0] == LogLevel.LOG_LEVEL_INFO
        and line[1].startswith("[I][server]: connection{peer=127.0.0.1:")
        and line[1].endswith("client connected"),
        "the other client's connection",
    )
    # bleproxy logs at debug here, but this client asked for info.
    assert all(level <= LogLevel.LOG_LEVEL_INFO for level, _ in lines[seen:]), lines
    await other.disconnect()
    await client.disconnect()


def start_proxy(binary, mode, env):
    step("starting bleproxy")
    # Unprivileged, as in deployments: the bus policy must let it in.
    return subprocess.Popen(
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
        env=env,
    )


def stop_proxy(proxy):
    step("stopping bleproxy")
    proxy.send_signal(signal.SIGTERM)
    code = proxy.wait(TIMEOUT)
    assert code == 0, f"bleproxy exited with {code}"


async def check_passive_fallback(binary):
    """Without advertisement monitors, passive scanning falls back to active."""
    step("testing passive scanning without advertisement monitor support")
    bus, mock = mock_bluez()
    # Like BlueZ releases that only offer monitors with --experimental.
    adapter = dbus.Interface(bus.get_object("org.bluez", "/org/bluez/hci0"), MOCK)
    adapter.AddMethod(
        "org.bluez.AdvertisementMonitorManager1",
        "RegisterMonitor",
        "o",
        "",
        "raise dbus.exceptions.DBusException('no monitors', "
        "name='org.freedesktop.DBus.Error.UnknownMethod')",
    )
    proxy = start_proxy(binary, "passive", dict(os.environ))
    try:
        client = await connect(None)
        states = []
        client.subscribe_bluetooth_scanner_state(states.append)
        received = []
        client.subscribe_bluetooth_le_raw_advertisements(
            lambda response: received.extend(response.advertisements)
        )
        step("checking that it scans actively and says so")
        await wait_for(
            states,
            lambda s: s.state == BluetoothScannerState.RUNNING
            and s.mode == BluetoothScannerMode.ACTIVE
            and s.configured_mode == BluetoothScannerMode.PASSIVE,
            "the scanner running in active mode",
        )
        calls = adapter_calls(bus)
        assert "RegisterMonitor" in calls and "StartDiscovery" in calls, calls
        address = int(DEVICE_ADDRESS.replace(":", ""), 16)
        mock.AddDevice("hci0", DEVICE_ADDRESS, "ATC_1122")
        await wait_for(received, lambda a: a.address == address, "the device")
        await client.disconnect()
    finally:
        stop_proxy(proxy)
    step("passive fallback passed")


async def main(binary, mode, encrypted, filtered):
    step(
        f"testing {mode} scanning"
        + (" with encryption" if encrypted else "")
        + (" and an address filter" if filtered else "")
    )
    bus, mock = mock_bluez()
    key = base64.b64encode(os.urandom(32)).decode() if encrypted else None
    env = dict(os.environ)
    if key:
        env["BLEPROXY_ENCRYPTION_KEY"] = key
    if filtered:
        # BlueZ already knows one device's name; one gets a label, one
        # neither.
        mock.AddDevice("hci0", KNOWN_ADDRESS, "ATC_7788")
        env["BLEPROXY_ALLOW_ADDRESS"] = (
            f"{DEVICE_ADDRESS}=Thermo,{SECOND_ADDRESS},{KNOWN_ADDRESS}"
        )
    proxy = start_proxy(binary, mode, env)
    try:
        if encrypted:
            step("checking that wrong keys and plaintext clients are rejected")
            await check_rejections(key)
        client = await connect(key)
        step("checking device info")
        info = await client.device_info()
        assert info.bluetooth_mac_address == ADAPTER_ADDRESS, info
        assert info.mac_address == ADAPTER_ADDRESS, info
        assert info.name == "bleproxy-030405", info
        assert info.project_name == "fiam.bleproxy", info
        assert info.api_encryption_supported == encrypted, info
        flags = info.bluetooth_proxy_feature_flags_compat(client.api_version)
        # Passive scan, active connections, remote caching, raw
        # advertisements, and state and mode.
        assert flags == 0b110_0111, flags
        allowed = (DEVICE_ADDRESS, SECOND_ADDRESS, KNOWN_ADDRESS) if filtered else ()
        switch, entity_states = await check_entities(client, allowed)
        await check_logs(key)

        states = []
        client.subscribe_bluetooth_scanner_state(states.append)
        received = []
        client.subscribe_bluetooth_le_raw_advertisements(
            lambda response: received.extend(response.advertisements)
        )

        configured = MODES[mode]
        step("checking the reported scanner state")
        await wait_for(
            states,
            lambda s: s.state == BluetoothScannerState.RUNNING
            and s.mode == configured
            and s.configured_mode == configured,
            "the running scanner",
        )

        step("checking that scanning was started")
        await check_scanning_started(bus, mode, proxy.pid)
        await check_scanning_switch(client, bus, mode, switch, entity_states, states)

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

        other = "passive" if mode == "active" else "active"
        step(f"switching to {other} scanning")
        client.bluetooth_scanner_set_mode(MODES[other])
        await wait_for(
            states,
            lambda s: s.state == BluetoothScannerState.RUNNING
            and s.mode == MODES[other]
            and s.configured_mode == configured,
            f"the scanner running in {other} mode",
        )
        calls = adapter_calls(bus)
        if other == "passive":
            assert "StopDiscovery" in calls and "RegisterMonitor" in calls, calls
        else:
            assert "UnregisterMonitor" in calls and "StartDiscovery" in calls, calls

        step("receiving advertisements after the switch")
        if filtered:
            # BlueZ signals arrive in order, so once the allowed device that
            # appeared later is through, the filtered out one was dropped.
            mock.AddDevice("hci0", BLOCKED_ADDRESS, "ATC_5566")
        second = int(SECOND_ADDRESS.replace(":", ""), 16)
        mock.AddDevice("hci0", SECOND_ADDRESS, "ATC_3344")
        await wait_for(received, lambda a: a.address == second, "the second device")
        if filtered:
            blocked = int(BLOCKED_ADDRESS.replace(":", ""), 16)
            assert all(a.address != blocked for a in received), "forwarded a filtered out device"
            await check_device_sensors(key)

        await check_connections(bus, client, path, flags)
    finally:
        stop_proxy(proxy)
    step(f"{mode} scanning passed")


if __name__ == "__main__":
    flags = sys.argv[3:]
    if "nomonitor" in flags:
        asyncio.run(check_passive_fallback(sys.argv[1]))
    else:
        asyncio.run(main(sys.argv[1], sys.argv[2], "encrypted" in flags, "filtered" in flags))
