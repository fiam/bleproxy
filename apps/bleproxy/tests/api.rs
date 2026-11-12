//! Drives the ESPHome API server over a real TCP connection, the way Home
//! Assistant does.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers should fail loudly"
)]

use std::sync::Arc;
use std::time::Duration;

use bleproxy::connections::{
    self, Characteristic, Command, ConnectionsDriver, ConnectionsHandle, Descriptor, Event, Service,
};
use bleproxy::filter::AddressFilter;
use bleproxy::scanner::{self, ScannerDriver, ScannerHandle};
use bleproxy::{Identity, server};
use bleproxy_core::Uuid;
use bleproxy_core::connection::ConnectionRequest;
use bleproxy_core::scanner::{ScanMode, ScannerState};
use bleproxy_core::{AddressType, AdvData, Advertisement, BdAddr};
use bleproxy_protocol::FrameHeader;
use bleproxy_protocol::wire::{Field, Fields, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot};
use tokio::time::timeout;

const MAC: BdAddr = BdAddr::new([0x02, 0x11, 0x22, 0xA1, 0xB2, 0xC3]);
const DEADLINE: Duration = Duration::from_secs(5);

struct Proxy {
    client: TcpStream,
    advertisements: broadcast::Sender<Advertisement>,
    inbox: Vec<u8>,
    _shutdown: oneshot::Sender<()>,
}

impl Proxy {
    async fn start() -> Self {
        Self::start_with(None, None).await
    }

    /// Starts a proxy with a switchable scanner, returning the driver the
    /// test plays the scanner with.
    async fn start_with_scanner(configured: ScanMode) -> (Self, ScannerDriver) {
        let (handle, driver) = scanner::channel(configured);
        (Self::start_with(Some(handle), None).await, driver)
    }

    /// Starts a proxy that offers connections, returning the driver the
    /// test plays the connection manager with.
    async fn start_with_connections(limit: u32) -> (Self, ConnectionsDriver) {
        let (handle, driver) = connections::channel(limit, AddressFilter::default());
        (Self::start_with(None, Some(handle)).await, driver)
    }

    async fn start_with(
        scanner: Option<ScannerHandle>,
        connections: Option<ConnectionsHandle>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local address");
        let mut identity = Identity::new(None, None, MAC, MAC).expect("identity");
        if let Some(scanner) = &scanner {
            identity = identity.with_scanner(scanner.configured());
        }
        if connections.is_some() {
            identity = identity.with_connections();
        }
        let (advertisements, _) = broadcast::channel(64);
        let (shutdown, stopped) = oneshot::channel::<()>();
        let context = server::Context {
            identity: Arc::new(identity),
            advertisements: advertisements.clone(),
            scanner,
            diagnostics: Arc::default(),
            encryption: None,
            connections,
        };
        tokio::spawn(server::serve(listener, context, async move {
            let _ = stopped.await;
        }));
        let client = TcpStream::connect(address).await.expect("connect");
        Self {
            client,
            advertisements,
            inbox: Vec::new(),
            _shutdown: shutdown,
        }
    }

    async fn send(&mut self, message_type: u8, payload: &[u8]) {
        let len = u8::try_from(payload.len()).expect("short test payload");
        let mut frame = vec![0x00, len, message_type];
        frame.extend_from_slice(payload);
        self.client.write_all(&frame).await.expect("write");
    }

    /// Reads the next frame, returning its type and payload.
    async fn receive(&mut self) -> (u16, Vec<u8>) {
        timeout(DEADLINE, async {
            loop {
                if let Some((header, header_len)) =
                    FrameHeader::decode(&self.inbox).expect("valid frame")
                {
                    let end = header_len + header.payload_len as usize;
                    if self.inbox.len() >= end {
                        let payload = self.inbox[header_len..end].to_vec();
                        self.inbox.drain(..end);
                        return (header.message_type, payload);
                    }
                }
                let read = self.client.read_buf(&mut self.inbox).await.expect("read");
                assert_ne!(read, 0, "server closed the connection");
            }
        })
        .await
        .expect("timed out waiting for a frame")
    }

    async fn hello(&mut self) {
        // HelloRequest { client_info: "test", api_version: 1.14 }
        self.send(
            1,
            &[0x0A, 0x04, b't', b'e', b's', b't', 0x10, 0x01, 0x18, 0x0E],
        )
        .await;
        let (message_type, _) = self.receive().await;
        assert_eq!(message_type, 2);
    }
}

fn fields(payload: &[u8]) -> Vec<Field<'_>> {
    Fields::new(payload)
        .collect::<Result<_, _>>()
        .expect("valid message")
}

fn string_field<'a>(fields: &[Field<'a>], number: u32) -> &'a str {
    fields
        .iter()
        .find(|field| field.number == number)
        .map(|field| field.as_str().expect("string"))
        .unwrap_or_default()
}

#[tokio::test]
async fn handshake_and_device_info() {
    let mut proxy = Proxy::start().await;
    proxy.send(1, &[0x10, 0x01, 0x18, 0x0E]).await;
    let (message_type, payload) = proxy.receive().await;
    assert_eq!(message_type, 2, "HelloResponse");
    let hello = fields(&payload);
    assert_eq!(string_field(&hello, 4), "bleproxy-a1b2c3");

    proxy.send(9, &[]).await;
    let (message_type, payload) = proxy.receive().await;
    assert_eq!(message_type, 10, "DeviceInfoResponse");
    let info = fields(&payload);
    assert_eq!(string_field(&info, 3), "02:11:22:A1:B2:C3");
    assert_eq!(string_field(&info, 8), "fiam.bleproxy");
    let flags = info
        .iter()
        .find(|field| field.number == 15)
        .map(|field| field.as_u32().expect("flags"));
    assert_eq!(flags, Some(0b10_0001));

    proxy.send(7, &[]).await;
    assert_eq!(proxy.receive().await.0, 8, "PingResponse");
}

#[tokio::test]
async fn switches_scanning_off_and_on() {
    let (mut proxy, mut driver) = Proxy::start_with_scanner(ScanMode::Active).await;
    proxy.hello().await;

    // With a scanner, the BLE Scanning switch comes first.
    proxy.send(11, &[]).await;
    let (message_type, payload) = proxy.receive().await;
    assert_eq!(message_type, 17, "ListEntitiesSwitchResponse");
    assert_eq!(string_field(&fields(&payload), 1), "ble_scanning");
    while proxy.receive().await.0 != 19 {}

    // It starts on.
    proxy.send(20, &[]).await;
    let (message_type, payload) = proxy.receive().await;
    assert_eq!(message_type, 26, "SwitchStateResponse");
    assert_eq!(payload, hex("0d040000001001"));
    let on = driver.wanted();
    assert!(on.enabled);

    // SwitchCommandRequest { key: 4, state: false } turns the scanner off,
    // and the new state is pushed.
    proxy.send(33, &[0x0D, 0x04, 0x00, 0x00, 0x00]).await;
    let off = timeout(DEADLINE, driver.change(on)).await.expect("switch");
    assert!(!off.enabled);
    loop {
        let (message_type, payload) = proxy.receive().await;
        if message_type == 26 {
            assert_eq!(payload, hex("0d04000000"), "off");
            break;
        }
    }

    // Commands for other keys are ignored.
    proxy
        .send(33, &[0x0D, 0x09, 0x00, 0x00, 0x00, 0x10, 0x01])
        .await;
    proxy.send(7, &[]).await;
    while proxy.receive().await.0 != 8 {}
    assert!(!driver.wanted().enabled);
}

#[tokio::test]
async fn reports_diagnostic_sensors() {
    let mut proxy = Proxy::start().await;
    proxy.hello().await;

    proxy.send(11, &[]).await;
    let mut listed = Vec::new();
    loop {
        let (message_type, payload) = proxy.receive().await;
        if message_type == 19 {
            break;
        }
        let listing = fields(&payload);
        // Text sensors (18) and sensors (16) number the field differently.
        let category = if message_type == 18 { 7 } else { 13 };
        let category = listing
            .iter()
            .find(|field| field.number == category)
            .and_then(|field| field.as_u32().ok());
        assert_eq!(category, Some(2), "diagnostic entity category");
        listed.push((message_type, string_field(&listing, 1).to_owned()));
    }
    let listed: Vec<(u16, &str)> = listed.iter().map(|(t, id)| (*t, id.as_str())).collect();
    assert_eq!(
        listed,
        [
            (18, "uptime"),
            (16, "advertisements"),
            (16, "connected_clients")
        ]
    );

    // Subscribing sends the current states.
    proxy.send(20, &[]).await;
    let (message_type, payload) = proxy.receive().await;
    assert_eq!(message_type, 27, "TextSensorStateResponse");
    assert!(string_field(&fields(&payload), 2).ends_with('s'));
    let mut sensors = Vec::new();
    for _ in 0..2 {
        let (message_type, payload) = proxy.receive().await;
        assert_eq!(message_type, 25, "SensorStateResponse");
        let state = fields(&payload)
            .iter()
            .find(|field| field.number == 2)
            .map(|field| match field.value {
                Value::Fixed32(bits) => f32::from_bits(bits),
                ref other => panic!("unexpected state {other:?}"),
            })
            .unwrap_or_default();
        sensors.push(state);
    }
    // Nothing was published, and this test is the one client.
    assert_eq!(sensors, [0.0, 1.0]);
}

#[tokio::test]
async fn forwards_advertisements_to_subscribers() {
    let mut proxy = Proxy::start().await;
    proxy.hello().await;
    // SubscribeBluetoothLEAdvertisementsRequest { flags: RAW }, followed by
    // a ping so we know the subscription has been processed.
    proxy.send(66, &[0x08, 0x01]).await;
    proxy.send(7, &[]).await;
    assert_eq!(proxy.receive().await.0, 8);

    let advertisement = Advertisement {
        address: BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]),
        address_type: AddressType::Random,
        rssi: -70,
        data: AdvData::new(&[0x02, 0x01, 0x06]).expect("data"),
    };
    proxy
        .advertisements
        .send(advertisement)
        .expect("subscriber exists");

    let (message_type, payload) = proxy.receive().await;
    assert_eq!(message_type, 93, "BluetoothLERawAdvertisementsResponse");
    let batch = fields(&payload);
    assert_eq!(batch.len(), 1);
    let Value::Len(entry) = batch[0].value else {
        panic!("expected an embedded message");
    };
    let entry = fields(entry);
    assert_eq!(entry[0].as_u64(), Ok(0xA4C1_3800_1122));
    assert_eq!(entry[1].as_u32(), Ok(139), "zigzag(-70)");
    assert_eq!(entry[2].as_u32(), Ok(1));
    assert_eq!(entry[3].as_bytes(), Ok(&[0x02, 0x01, 0x06][..]));
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn uint_fields(payload: &[u8]) -> Vec<(u32, u32)> {
    fields(payload)
        .iter()
        .map(|field| (field.number, field.as_u32().expect("uint")))
        .collect()
}

#[tokio::test]
async fn reports_scanner_state_and_switches_modes() {
    let (mut proxy, mut driver) = Proxy::start_with_scanner(ScanMode::Active).await;
    proxy.hello().await;

    proxy.send(9, &[]).await;
    let (_, info) = proxy.receive().await;
    let flags = fields(&info)
        .iter()
        .find(|field| field.number == 15)
        .map(|field| field.as_u32().expect("flags"));
    assert_eq!(flags, Some(0b110_0001), "STATE_AND_MODE is offered");

    // Subscribing reports the current state: idle, active, configured active.
    proxy.send(66, &[0x08, 0x01]).await;
    let (message_type, state) = proxy.receive().await;
    assert_eq!(message_type, 126, "BluetoothScannerStateResponse");
    assert_eq!(uint_fields(&state), [(2, 1), (3, 1)]);

    // BluetoothScannerSetModeRequest { mode: PASSIVE } reaches the scanner.
    proxy.send(127, &[]).await;
    let initial = driver.wanted();
    let requested = timeout(DEADLINE, driver.change(initial))
        .await
        .expect("mode request");
    assert_eq!(requested.mode, ScanMode::Passive);

    // Status changes are pushed to the subscriber.
    driver.report(ScannerState::Running, ScanMode::Passive);
    let (message_type, state) = proxy.receive().await;
    assert_eq!(message_type, 126);
    assert_eq!(uint_fields(&state), [(1, 2), (3, 1)], "running, passive");
}

#[tokio::test]
async fn closes_connections_that_skip_the_handshake() {
    let mut proxy = Proxy::start().await;
    proxy.send(9, &[]).await;
    let mut buf = [0; 16];
    let read = timeout(DEADLINE, proxy.client.read(&mut buf))
        .await
        .expect("server should close the connection")
        .expect("read");
    assert_eq!(read, 0);
}

#[tokio::test]
async fn acknowledges_disconnects() {
    let mut proxy = Proxy::start().await;
    proxy.hello().await;
    proxy.send(5, &[]).await;
    assert_eq!(proxy.receive().await.0, 6, "DisconnectResponse");
}

/// The device the connection tests talk to, and its address on the wire.
const DEVICE: BdAddr = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);
const DEVICE_HEX: &str = "08a2a280c0939829";

/// Waits for the next request the proxy passes to the connection manager.
async fn next_request(
    driver: &mut ConnectionsDriver,
) -> (connections::EventSender, connections::Request) {
    match timeout(DEADLINE, driver.next()).await.expect("a command") {
        Some(Command::Request {
            events, request, ..
        }) => (events, request),
        other => panic!("expected a request, got {other:?}"),
    }
}

#[tokio::test]
async fn connects_devices_and_reports_free_slots() {
    let (mut proxy, mut driver) = Proxy::start_with_connections(3).await;
    proxy.hello().await;

    proxy.send(9, &[]).await;
    let (_, info) = proxy.receive().await;
    let flags = fields(&info)
        .iter()
        .find(|field| field.number == 15)
        .map(|field| field.as_u32().expect("flags"));
    assert_eq!(
        flags,
        Some(0b10_0111),
        "ACTIVE_CONNECTIONS and REMOTE_CACHING are offered"
    );

    // Subscribing to free slots reports them right away...
    proxy.send(80, &[]).await;
    let (message_type, free) = proxy.receive().await;
    assert_eq!(message_type, 81, "BluetoothConnectionsFreeResponse");
    assert_eq!(uint_fields(&free), [(1, 3), (2, 3)]);

    // ...and connection requests reach the connection manager.
    // BluetoothDeviceRequest { CONNECT_V3_WITH_CACHE, address type random }
    proxy
        .send(68, &hex(&format!("{DEVICE_HEX}100418012001")))
        .await;
    let (events, request) = next_request(&mut driver).await;
    assert_eq!(
        request,
        ConnectionRequest::Connect {
            address: DEVICE,
            address_type: Some(AddressType::Random),
        }
    );

    // Slot changes are pushed to the subscriber.
    driver.report(vec![DEVICE]);
    let (message_type, free) = proxy.receive().await;
    assert_eq!(message_type, 81);
    assert_eq!(free, hex("080210031a07a2a280c0939829"));

    events.send(Event::Connection {
        address: DEVICE,
        connected: true,
        mtu: 247,
        error: 0,
    });
    let (message_type, connected) = proxy.receive().await;
    assert_eq!(message_type, 69, "BluetoothDeviceConnectionResponse");
    assert_eq!(connected, hex(&format!("{DEVICE_HEX}100118f701")));

    // Closing the API connection releases the client's devices.
    drop(proxy);
    let command = timeout(DEADLINE, driver.next()).await.expect("a command");
    assert!(matches!(command, Some(Command::Release(_))), "{command:?}");
}

#[tokio::test]
async fn proxies_gatt_operations() {
    let (mut proxy, mut driver) = Proxy::start_with_connections(3).await;
    proxy.hello().await;

    // Services come one per message, then a done message.
    proxy.send(70, &hex(DEVICE_HEX)).await;
    let (events, request) = next_request(&mut driver).await;
    assert_eq!(request, ConnectionRequest::GetServices { address: DEVICE });
    events.send(Event::Services {
        address: DEVICE,
        services: vec![Service {
            uuid: Uuid::from_u16(0x180F),
            handle: 0x28,
            characteristics: vec![Characteristic {
                uuid: Uuid::from_u16(0x2A19),
                handle: 0x2A,
                properties: 0x12,
                descriptors: vec![Descriptor {
                    uuid: Uuid::from_u16(0x2902),
                    handle: 0x2B,
                }],
            }],
        }],
    });
    let (message_type, services) = proxy.receive().await;
    assert_eq!(message_type, 71, "BluetoothGATTGetServicesResponse");
    assert_eq!(
        services,
        hex(concat!(
            "08a2a280c093982912450a1180a08080f08106fbe9ecfc85908080800110281a",
            "2e0a1180a0808090c30afbe9ecfc859080808001102a181222150a1180a08080",
            "a0a00afbe9ecfc859080808001102b",
        ))
    );
    let (message_type, done) = proxy.receive().await;
    assert_eq!(message_type, 72, "BluetoothGATTGetServicesDoneResponse");
    assert_eq!(done, hex(DEVICE_HEX));

    // Written values are passed on; results come back as responses.
    proxy
        .send(77, &hex(&format!("{DEVICE_HEX}102b1a020100")))
        .await;
    let (events, request) = next_request(&mut driver).await;
    assert_eq!(
        request,
        ConnectionRequest::WriteDescriptor {
            address: DEVICE,
            handle: 0x2B,
            data: vec![0x01, 0x00],
        }
    );
    events.send(Event::Written {
        address: DEVICE,
        handle: 0x2B,
    });
    assert_eq!(proxy.receive().await.0, 83, "BluetoothGATTWriteResponse");

    events.send(Event::Notification {
        address: DEVICE,
        handle: 0x2A,
        data: vec![0x63],
    });
    let (message_type, notification) = proxy.receive().await;
    assert_eq!(message_type, 79, "BluetoothGATTNotifyDataResponse");
    assert_eq!(notification, hex(&format!("{DEVICE_HEX}102a1a0163")));

    events.send(Event::Error {
        address: DEVICE,
        handle: 0x2A,
        error: -1,
    });
    let (message_type, error) = proxy.receive().await;
    assert_eq!(message_type, 82, "BluetoothGATTErrorResponse");
    assert_eq!(
        error,
        hex(&format!("{DEVICE_HEX}102a18ffffffffffffffffff01"))
    );
}
