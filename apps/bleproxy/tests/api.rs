//! Drives the ESPHome API server over a real TCP connection, the way Home
//! Assistant does.

#![allow(clippy::expect_used, reason = "test helpers should fail loudly")]

use std::sync::Arc;
use std::time::Duration;

use bleproxy::scanner::{self, ScannerDriver, ScannerHandle};
use bleproxy::{Identity, server};
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
        Self::start_with(None).await
    }

    /// Starts a proxy with a switchable scanner, returning the driver the
    /// test plays the scanner with.
    async fn start_with_scanner(configured: ScanMode) -> (Self, ScannerDriver) {
        let (handle, driver) = scanner::channel(configured);
        (Self::start_with(Some(handle)).await, driver)
    }

    async fn start_with(scanner: Option<ScannerHandle>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local address");
        let mut identity = Identity::new(None, None, MAC, MAC).expect("identity");
        if let Some(scanner) = &scanner {
            identity = identity.with_scanner(scanner.configured());
        }
        let (advertisements, _) = broadcast::channel(64);
        let (shutdown, stopped) = oneshot::channel::<()>();
        let context = server::Context {
            identity: Arc::new(identity),
            advertisements: advertisements.clone(),
            scanner,
            encryption: None,
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

    proxy.send(11, &[]).await;
    assert_eq!(proxy.receive().await.0, 19, "ListEntitiesDoneResponse");
    proxy.send(7, &[]).await;
    assert_eq!(proxy.receive().await.0, 8, "PingResponse");
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
    let requested = timeout(DEADLINE, driver.mode_change(ScanMode::Active))
        .await
        .expect("mode request");
    assert_eq!(requested, ScanMode::Passive);

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
