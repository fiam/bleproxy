//! The ESPHome Native API server: accepts connections and runs a
//! [`Session`] for each.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use bleproxy_core::Advertisement;
use bleproxy_core::connection::ConnectionRequest;
use bleproxy_core::scanner::ScannerStatus;
use bleproxy_core::session::{Outcome, Reply, Session};
use bleproxy_protocol::wire::{DecodeError, EncodeError};
use bleproxy_protocol::{
    BluetoothConnectionsFreeResponse, BluetoothDeviceConnectionResponse,
    BluetoothGattCharacteristic, BluetoothGattDescriptor, BluetoothGattErrorResponse,
    BluetoothGattGetServicesDoneResponse, BluetoothGattGetServicesResponse,
    BluetoothGattNotifyDataResponse, BluetoothGattNotifyResponse, BluetoothGattReadResponse,
    BluetoothGattService, BluetoothGattWriteResponse, BluetoothLeRawAdvertisementsResponse,
    ClientMessage, PingRequest, RawAdvertisement, SubscribeLogsResponse, log_level,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{Instrument, debug, info, info_span, warn};

use crate::Identity;
use crate::connections::{Client, ConnectionsHandle, Event, Service, Slots};
use crate::diagnostics::{self, Diagnostics, STATE_INTERVAL};
use crate::encryption::EncryptionKey;
use crate::logs::{self, LogLine, LogSubscriber, Logs};
use crate::scanner::{SCANNING_SWITCH, ScannerHandle};
use crate::transport::{Message, Transport, TransportError};

/// Most advertisements sent in one message, as ESPHome does.
const BATCH_SIZE: usize = 16;

/// How long advertisements may wait to fill a batch.
const BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// After hearing nothing from a client for this long, ping it, as ESPHome
/// does. Clients only ping when they receive nothing themselves, so one
/// kept busy with advertisements or scanner updates can stay quiet.
const KEEPALIVE: Duration = Duration::from_secs(60);

/// Connections that stay silent this long, ping included, are closed.
/// ESPHome waits two and a half keepalive periods.
const IDLE_TIMEOUT: Duration = Duration::from_secs(150);

/// Initial size of the buffer message payloads are encoded into. A full
/// advertisement batch is under 1.5 KiB; the buffer grows for larger
/// messages, such as services with many characteristics.
const PAYLOAD_BUFFER_LEN: usize = 4096;

/// Errors that end a client connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// Reading from or writing to the socket failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The client violated the framing or the encryption handshake failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The client sent an invalid message.
    #[error("invalid message: {0}")]
    Decode(#[from] DecodeError),
    /// A reply did not fit the payload buffer.
    #[error("reply too large: {0}")]
    Encode(#[from] EncodeError),
}

/// What every connection shares.
#[derive(Debug, Clone)]
pub struct Context {
    /// How the proxy identifies itself.
    pub identity: Arc<Identity>,
    /// Advertisements to forward to subscribed clients.
    pub advertisements: broadcast::Sender<Advertisement>,
    /// The scanner, if clients may follow its state and switch its mode.
    pub scanner: Option<ScannerHandle>,
    /// What the proxy's diagnostic sensors report.
    pub diagnostics: Arc<Diagnostics>,
    /// Key that encrypts every connection, or `None` for plaintext.
    pub encryption: Option<EncryptionKey>,
    /// The connection manager, if clients may connect to devices.
    pub connections: Option<ConnectionsHandle>,
    /// The proxy's logs, for clients that subscribe to them.
    pub logs: Arc<Logs>,
    /// Lines describing the proxy's configuration, logged to clients that
    /// ask for it.
    pub config: Arc<[String]>,
}

/// Accepts API connections on `listener` until `shutdown` completes.
///
/// Each connection gets its own task. Advertisements published on the
/// context's channel are forwarded to every client that subscribed to them.
///
/// # Errors
///
/// Returns an error if accepting connections fails.
pub async fn serve(
    listener: TcpListener,
    context: Context,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => accepted?,
            () = &mut shutdown => return Ok(()),
        };
        let context = context.clone();
        tokio::spawn(
            async move {
                info!("client connected");
                match handle_connection(stream, &context).await {
                    Ok(()) => info!("client disconnected"),
                    Err(err) => warn!(%err, "client connection failed"),
                }
            }
            .instrument(info_span!("connection", %peer)),
        );
    }
}

/// Runs one API session over `stream`.
///
/// # Errors
///
/// Returns an error if the socket fails or the client violates the
/// protocol.
pub async fn handle_connection(
    stream: TcpStream,
    context: &Context,
) -> Result<(), ConnectionError> {
    stream.set_nodelay(true)?;
    let _connected = stream
        .peer_addr()
        .ok()
        .map(|peer| context.diagnostics.connect(peer.ip()));
    let (mut reader, writer) = stream.into_split();
    let identity = &context.identity;
    let transport = context
        .encryption
        .as_ref()
        .map_or(Transport::Plaintext, |key| {
            Transport::noise(key, identity.name(), &identity.mac_hex())
        });
    let mut out = Outbound::new(writer, transport);
    let device = identity.device_identity();
    let mut session = Session::new(&device);
    let mut scanner_status = context.scanner.as_ref().map(ScannerHandle::status);
    let mut scanning = context.scanner.as_ref().map(ScannerHandle::wanted);
    let mut connections = context.connections.as_ref().map(ConnectionsHandle::client);
    let mut slots = None;
    let mut log_lines = None;

    let mut inbox = Vec::with_capacity(1024);
    let mut subscription: Option<broadcast::Receiver<Advertisement>> = None;
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut flush = tokio::time::interval(BATCH_INTERVAL);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut refresh = tokio::time::interval_at(Instant::now() + STATE_INTERVAL, STATE_INTERVAL);
    refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut keepalive = Keepalive::new(Instant::now());
    let idle = tokio::time::sleep_until(keepalive.deadline());
    tokio::pin!(idle);

    loop {
        tokio::select! {
            read = reader.read_buf(&mut inbox) => {
                if read? == 0 {
                    return Ok(());
                }
                keepalive.heard(Instant::now());
                idle.as_mut().reset(keepalive.deadline());
                let client = connections.as_ref();
                if handle_messages(&mut inbox, &mut out, &mut session, context, client, &mut keepalive, &mut log_lines).await? {
                    out.writer.shutdown().await?;
                    return Ok(());
                }
                // Follow subscription changes made by the messages above.
                follow_advertisements(&session, context, &mut subscription, &mut batch, scanner_status.as_mut(), &mut out).await?;
                follow_slots(&session, context, &mut slots, &mut out).await?;
            }
            event = next_event(&mut connections) => out.event(&event).await?,
            Some(current) = changed(&mut slots) => out.slots(&current).await?,
            received = recv(&mut subscription) => match received {
                Ok(advertisement) => {
                    batch.push(advertisement);
                    if batch.len() >= BATCH_SIZE {
                        out.advertisements(&mut batch).await?;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    warn!(missed, "client is too slow, dropped advertisements");
                }
                Err(broadcast::error::RecvError::Closed) => subscription = None,
            },
            received = recv_log(&mut log_lines) => match received {
                Ok(line) => out.log(&line).await?,
                // Not logged: the warning would go to this client too.
                Err(broadcast::error::RecvError::Lagged(missed)) => out.logs_dropped(missed).await?,
                Err(broadcast::error::RecvError::Closed) => log_lines = None,
            },
            Some(status) = changed(&mut scanner_status), if subscription.is_some() => {
                if let Some(reply) = session.scanner_state(status) {
                    out.reply(&reply).await?;
                }
            }
            _ = flush.tick(), if !batch.is_empty() => {
                out.advertisements(&mut batch).await?;
            }
            _ = refresh.tick(), if session.wants_states() => out.states(context).await?,
            Some(wanted) = changed(&mut scanning), if session.wants_states() => {
                out.message(&wanted.switch_state()).await?;
            }
            () = &mut idle => match keepalive.expired() {
                Idle::Ping => {
                    debug!("pinging quiet client");
                    out.message(&PingRequest).await?;
                    idle.as_mut().reset(keepalive.deadline());
                }
                Idle::Close => {
                    info!("closing idle connection");
                    return Ok(());
                }
            },
        }
    }
}

/// Starts or stops forwarding advertisements as the client asked. Like
/// ESPHome, reports the scanner's state to new subscribers, who then
/// follow its changes.
async fn follow_advertisements<W: AsyncWriteExt + Unpin + Send>(
    session: &Session<'_>,
    context: &Context,
    subscription: &mut Option<broadcast::Receiver<Advertisement>>,
    batch: &mut Vec<Advertisement>,
    scanner_status: Option<&mut watch::Receiver<ScannerStatus>>,
    out: &mut Outbound<W>,
) -> Result<(), ConnectionError> {
    match (&subscription, session.wants_raw_advertisements()) {
        (None, true) => {
            *subscription = Some(context.advertisements.subscribe());
            if let Some(status) = scanner_status {
                let current = *status.borrow_and_update();
                if let Some(reply) = session.scanner_state(current) {
                    out.reply(&reply).await?;
                }
            }
        }
        (Some(_), false) => {
            *subscription = None;
            batch.clear();
        }
        _ => {}
    }
    Ok(())
}

/// Reports the free connection slots to new subscribers, who then follow
/// them.
async fn follow_slots<W: AsyncWriteExt + Unpin + Send>(
    session: &Session<'_>,
    context: &Context,
    slots: &mut Option<watch::Receiver<Slots>>,
    out: &mut Outbound<W>,
) -> Result<(), ConnectionError> {
    if slots.is_none()
        && session.wants_connections_free()
        && let Some(handle) = &context.connections
    {
        let mut receiver = handle.slots();
        let current = receiver.borrow_and_update().clone();
        out.slots(&current).await?;
        *slots = Some(receiver);
    }
    Ok(())
}

/// Handles the complete messages in `inbox`, and returns whether the
/// session asked to close the connection.
async fn handle_messages<W: AsyncWriteExt + Unpin + Send>(
    inbox: &mut Vec<u8>,
    out: &mut Outbound<W>,
    session: &mut Session<'_>,
    context: &Context,
    client: Option<&Client>,
    keepalive: &mut Keepalive,
    log_lines: &mut Option<LogSubscriber>,
) -> Result<bool, ConnectionError> {
    while let Some(Message {
        message_type,
        payload,
    }) = out.next_message(inbox).await?
    {
        let message = ClientMessage::decode(message_type, &payload)?;
        keepalive.established();
        log_message(&message);
        let subscribed = session.wants_states();
        let outcome = session.handle(&message);
        if outcome.list_entities {
            out.entities(context).await?;
        }
        if let Some(reply) = outcome.reply {
            out.reply(&reply).await?;
        }
        // Like ESPHome, send the current states to new subscribers.
        if !subscribed && session.wants_states() {
            out.states(context).await?;
        }
        if let Some(subscription) = outcome.logs {
            // A new subscription replaces the old one.
            *log_lines = (subscription.level > log_level::NONE)
                .then(|| context.logs.subscribe(subscription.level));
            if subscription.dump_config {
                out.config(context).await?;
            }
        }
        forward(&outcome, context, client);
        if outcome.close {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Decides when to ping a quiet client and when to give up on it.
#[derive(Debug)]
struct Keepalive {
    /// When the client last sent anything.
    heard: Instant,
    /// Whether it was pinged since.
    pinged: bool,
    /// Whether a message got through, which means any encryption handshake
    /// is done and the client can be pinged.
    established: bool,
}

/// What to do with a client that has been quiet until the deadline.
#[derive(Debug, PartialEq, Eq)]
enum Idle {
    /// Ping it, and wait for an answer until the next deadline.
    Ping,
    /// Close the connection.
    Close,
}

impl Keepalive {
    const fn new(now: Instant) -> Self {
        Self {
            heard: now,
            pinged: false,
            established: false,
        }
    }

    /// Records that the client sent something at `now`.
    const fn heard(&mut self, now: Instant) {
        self.heard = now;
        self.pinged = false;
    }

    /// Records that a message got through.
    const fn established(&mut self) {
        self.established = true;
    }

    /// When the client will have been quiet for too long.
    fn deadline(&self) -> Instant {
        self.heard + if self.pinged { IDLE_TIMEOUT } else { KEEPALIVE }
    }

    /// Decides what to do once the deadline passes.
    const fn expired(&mut self) -> Idle {
        if self.pinged || !self.established {
            return Idle::Close;
        }
        self.pinged = true;
        Idle::Ping
    }
}

/// Passes the scan mode, switch and connection requests in `outcome` on
/// to the scanner and the connection manager.
fn forward(outcome: &Outcome<'_, '_>, context: &Context, client: Option<&Client>) {
    if let (Some(mode), Some(scanner)) = (outcome.scan_mode, &context.scanner) {
        info!(?mode, "client requested a scan mode");
        scanner.request(mode);
    }
    match (outcome.switch, &context.scanner) {
        (Some(command), Some(scanner)) if command.key == SCANNING_SWITCH.key => {
            info!(on = command.state, "client switched scanning");
            scanner.enable(command.state);
        }
        (Some(command), _) => debug!(key = command.key, "command for an unknown switch"),
        (None, _) => {}
    }
    if let (Some(request), Some(client)) = (outcome.connection, client) {
        log_request(&request);
        client.request(request.map_data(<[u8]>::to_vec));
    }
}

/// Waits for a watched value to change, and returns it. Never completes
/// without a receiver, or once the sender is gone.
async fn changed<T: Clone + Send + Sync>(receiver: &mut Option<watch::Receiver<T>>) -> Option<T> {
    if let Some(receiver) = receiver
        && receiver.changed().await.is_ok()
    {
        return Some(receiver.borrow_and_update().clone());
    }
    std::future::pending().await
}

/// Waits for the next connection event, or forever without connections.
async fn next_event(client: &mut Option<Client>) -> Event {
    match client {
        Some(client) => client.event().await,
        None => std::future::pending().await,
    }
}

/// Receives the next log line, or waits forever without a subscription.
async fn recv_log(
    subscriber: &mut Option<LogSubscriber>,
) -> Result<LogLine, broadcast::error::RecvError> {
    match subscriber {
        Some(subscriber) => subscriber.recv().await,
        None => std::future::pending().await,
    }
}

/// Receives from `subscription`, or waits forever without one.
async fn recv(
    subscription: &mut Option<broadcast::Receiver<Advertisement>>,
) -> Result<Advertisement, broadcast::error::RecvError> {
    match subscription {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

/// The sending side of a connection: frames messages through the
/// connection's transport and writes them to the socket.
struct Outbound<W> {
    writer: W,
    transport: Transport,
    /// Scratch space for encoding payloads.
    payload: Vec<u8>,
    /// Bytes waiting to be written.
    wire: Vec<u8>,
}

impl<W: AsyncWriteExt + Unpin> Outbound<W> {
    fn new(writer: W, transport: Transport) -> Self {
        Self {
            writer,
            transport,
            payload: vec![0; PAYLOAD_BUFFER_LEN],
            wire: Vec::with_capacity(PAYLOAD_BUFFER_LEN),
        }
    }

    /// Takes the next message out of `inbox`. Whatever the transport has
    /// to say to the client (handshake replies, or why it gives up) is
    /// sent first, even if reading fails.
    async fn next_message(
        &mut self,
        inbox: &mut Vec<u8>,
    ) -> Result<Option<Message>, ConnectionError> {
        let result = self.transport.read(inbox, &mut self.wire);
        self.flush().await?;
        Ok(result?)
    }

    async fn reply(&mut self, reply: &Reply<'_>) -> Result<(), ConnectionError> {
        let len = reply.encode_payload(&mut self.payload)?;
        self.send(reply.message_type(), len).await
    }

    /// Describes the diagnostic sensors, including the allowed devices'
    /// ones, and the scanning switch if there is a scanner.
    async fn entities(&mut self, context: &Context) -> Result<(), ConnectionError> {
        if context.scanner.is_some() {
            self.message(&SCANNING_SWITCH).await?;
        }
        for listing in &diagnostics::TEXT_SENSORS {
            self.message(listing).await?;
        }
        for listing in &diagnostics::SENSORS {
            self.message(listing).await?;
        }
        for (rssi, last_seen) in context.diagnostics.device_entities() {
            self.message(&rssi).await?;
            self.message(&last_seen).await?;
        }
        Ok(())
    }

    /// Sends the states of the entities [`Self::entities`] describes.
    async fn states(&mut self, context: &Context) -> Result<(), ConnectionError> {
        if let Some(scanner) = &context.scanner {
            let wanted = *scanner.wanted().borrow();
            self.message(&wanted.switch_state()).await?;
        }
        let uptime = context.diagnostics.uptime();
        for state in Diagnostics::text_states(&uptime) {
            self.message(&state).await?;
        }
        for state in context.diagnostics.sensor_states() {
            self.message(&state).await?;
        }
        for device in context.diagnostics.device_states() {
            self.message(&device.rssi).await?;
            self.message(&device.last_seen()).await?;
        }
        Ok(())
    }

    async fn log(&mut self, line: &LogLine) -> Result<(), ConnectionError> {
        self.message(&SubscribeLogsResponse {
            level: line.level,
            message: line.text.as_bytes(),
        })
        .await
    }

    /// Tells a client that fell behind how many lines it missed. Not
    /// logged, since the warning would go to the client too.
    async fn logs_dropped(&mut self, missed: u64) -> Result<(), ConnectionError> {
        let text = format!("{missed} log lines dropped");
        self.log(&LogLine {
            level: log_level::WARN,
            text: logs::format_line(log_level::WARN, "server", &text),
        })
        .await
    }

    /// Logs the proxy's configuration, as ESPHome devices do on request.
    async fn config(&mut self, context: &Context) -> Result<(), ConnectionError> {
        for line in context.config.iter() {
            self.log(&LogLine {
                level: log_level::CONFIG,
                text: logs::format_line(log_level::CONFIG, "bleproxy", line),
            })
            .await?;
        }
        Ok(())
    }

    async fn advertisements(
        &mut self,
        batch: &mut Vec<Advertisement>,
    ) -> Result<(), ConnectionError> {
        let raw: Vec<RawAdvertisement<'_>> = batch.iter().map(Advertisement::as_raw).collect();
        self.message(&BluetoothLeRawAdvertisementsResponse {
            advertisements: &raw,
        })
        .await?;
        debug!(count = batch.len(), "forwarded advertisements");
        batch.clear();
        Ok(())
    }

    async fn slots(&mut self, slots: &Slots) -> Result<(), ConnectionError> {
        let allocated: Vec<u64> = slots.allocated.iter().map(|a| a.to_u64()).collect();
        self.message(&BluetoothConnectionsFreeResponse {
            free: slots.free(),
            limit: slots.limit,
            allocated: &allocated,
        })
        .await
    }

    async fn event(&mut self, event: &Event) -> Result<(), ConnectionError> {
        match event {
            Event::Connection {
                address,
                connected,
                mtu,
                error,
            } => {
                info!(%address, connected, mtu, error, "device connection changed");
                self.message(&BluetoothDeviceConnectionResponse {
                    address: address.to_u64(),
                    connected: *connected,
                    mtu: (*mtu).into(),
                    error: *error,
                })
                .await
            }
            Event::Services { address, services } => {
                let address = address.to_u64();
                // One service per message keeps messages small.
                for service in services {
                    self.service(address, service).await?;
                }
                self.message(&BluetoothGattGetServicesDoneResponse { address })
                    .await
            }
            Event::Read {
                address,
                handle,
                data,
            } => {
                self.message(&BluetoothGattReadResponse {
                    address: address.to_u64(),
                    handle: *handle,
                    data,
                })
                .await
            }
            Event::Written { address, handle } => {
                self.message(&BluetoothGattWriteResponse {
                    address: address.to_u64(),
                    handle: *handle,
                })
                .await
            }
            Event::Notifying { address, handle } => {
                self.message(&BluetoothGattNotifyResponse {
                    address: address.to_u64(),
                    handle: *handle,
                })
                .await
            }
            Event::Notification {
                address,
                handle,
                data,
            } => {
                self.message(&BluetoothGattNotifyDataResponse {
                    address: address.to_u64(),
                    handle: *handle,
                    data,
                })
                .await
            }
            Event::Error {
                address,
                handle,
                error,
            } => {
                debug!(%address, handle, error, "GATT operation failed");
                self.message(&BluetoothGattErrorResponse {
                    address: address.to_u64(),
                    handle: *handle,
                    error: *error,
                })
                .await
            }
        }
    }

    async fn service(&mut self, address: u64, service: &Service) -> Result<(), ConnectionError> {
        let descriptors: Vec<Vec<BluetoothGattDescriptor>> = service
            .characteristics
            .iter()
            .map(|characteristic| {
                characteristic
                    .descriptors
                    .iter()
                    .map(|descriptor| BluetoothGattDescriptor {
                        uuid: descriptor.uuid.as_u128(),
                        handle: descriptor.handle.into(),
                    })
                    .collect()
            })
            .collect();
        let characteristics: Vec<BluetoothGattCharacteristic<'_>> = service
            .characteristics
            .iter()
            .zip(&descriptors)
            .map(
                |(characteristic, descriptors)| BluetoothGattCharacteristic {
                    uuid: characteristic.uuid.as_u128(),
                    handle: characteristic.handle.into(),
                    properties: characteristic.properties.into(),
                    descriptors,
                },
            )
            .collect();
        self.message(&BluetoothGattGetServicesResponse {
            address,
            services: &[BluetoothGattService {
                uuid: service.uuid.as_u128(),
                handle: service.handle.into(),
                characteristics: &characteristics,
            }],
        })
        .await
    }

    /// Encodes and sends `message`, growing the payload buffer if needed.
    async fn message<M: bleproxy_protocol::Message + Sync>(
        &mut self,
        message: &M,
    ) -> Result<(), ConnectionError> {
        let len = message.encoded_len();
        if self.payload.len() < len {
            self.payload.resize(len, 0);
        }
        let len = message.encode_to_slice(&mut self.payload)?;
        self.send(M::TYPE, len).await
    }

    /// Sends the first `len` bytes of the payload buffer as a message.
    async fn send(&mut self, message_type: u16, len: usize) -> Result<(), ConnectionError> {
        let payload = self.payload.get(..len).unwrap_or_default();
        self.transport
            .write(message_type, payload, &mut self.wire)?;
        self.flush().await
    }

    async fn flush(&mut self) -> Result<(), ConnectionError> {
        if !self.wire.is_empty() {
            self.writer.write_all(&self.wire).await?;
            self.wire.clear();
        }
        Ok(())
    }
}

fn log_request(request: &ConnectionRequest<&[u8]>) {
    match request {
        ConnectionRequest::Connect {
            address,
            address_type,
        } => info!(%address, ?address_type, "client asked to connect"),
        ConnectionRequest::Disconnect { address } => {
            info!(%address, "client asked to disconnect");
        }
        other => debug!(?other, "connection request"),
    }
}

fn log_message(message: &ClientMessage<'_>) {
    match message {
        ClientMessage::Hello(hello) => info!(
            client = hello.client_info,
            api_version = %format_args!("{}.{}", hello.api_version_major, hello.api_version_minor),
            "hello"
        ),
        ClientMessage::SubscribeBluetoothLeAdvertisements { flags } => {
            info!(flags, "client subscribed to advertisements");
        }
        ClientMessage::UnsubscribeBluetoothLeAdvertisements => {
            info!("client unsubscribed from advertisements");
        }
        ClientMessage::SubscribeLogs { level, dump_config } => {
            info!(level, dump_config, "client subscribed to logs");
        }
        ClientMessage::Unknown(message_type) => debug!(message_type, "ignoring unknown message"),
        other => debug!(?other, "received"),
    }
}

#[cfg(test)]
mod tests {
    use bleproxy_protocol::wire::Encode as _;

    use super::*;

    #[test]
    fn pings_quiet_clients_before_closing() {
        let start = Instant::now();
        let mut keepalive = Keepalive::new(start);
        keepalive.established();
        assert_eq!(keepalive.deadline(), start + KEEPALIVE);
        assert_eq!(keepalive.expired(), Idle::Ping);
        // The ping extends the wait, counted from when the client was heard.
        assert_eq!(keepalive.deadline(), start + IDLE_TIMEOUT);
        assert_eq!(keepalive.expired(), Idle::Close);
    }

    #[test]
    fn hearing_from_the_client_restarts_the_wait() {
        let start = Instant::now();
        let mut keepalive = Keepalive::new(start);
        keepalive.established();
        assert_eq!(keepalive.expired(), Idle::Ping);
        let answered = start + KEEPALIVE + Duration::from_secs(1);
        keepalive.heard(answered);
        assert_eq!(keepalive.deadline(), answered + KEEPALIVE);
        assert_eq!(keepalive.expired(), Idle::Ping);
    }

    #[test]
    fn closes_connections_that_never_got_going() {
        // Before any message, the encryption handshake may still be
        // pending, so there is no way to ping.
        let mut keepalive = Keepalive::new(Instant::now());
        keepalive.heard(Instant::now());
        assert_eq!(keepalive.expired(), Idle::Close);
    }

    #[test]
    fn payload_buffer_starts_with_room_for_a_full_batch() {
        let data = [0xAA; bleproxy_core::AdvData::MAX_LEN];
        let raw = [RawAdvertisement {
            address: u64::from(u32::MAX) << 16,
            rssi: i32::from(i8::MIN),
            address_type: 1,
            data: &data,
        }; BATCH_SIZE];
        let response = BluetoothLeRawAdvertisementsResponse {
            advertisements: &raw,
        };
        assert!(response.encoded_len() < PAYLOAD_BUFFER_LEN);
    }
}
