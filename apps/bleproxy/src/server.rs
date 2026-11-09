//! The ESPHome Native API server: accepts connections and runs a
//! [`Session`] for each.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use bleproxy_core::Advertisement;
use bleproxy_core::scanner::ScannerStatus;
use bleproxy_core::session::{Reply, Session};
use bleproxy_protocol::wire::{DecodeError, Encode, EncodeError};
use bleproxy_protocol::{
    BluetoothLeRawAdvertisementsResponse, ClientMessage, Message as _, RawAdvertisement,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{Instrument, debug, info, info_span, warn};

use crate::Identity;
use crate::encryption::EncryptionKey;
use crate::scanner::ScannerHandle;
use crate::transport::{Message, Transport, TransportError};

/// Most advertisements sent in one message, as ESPHome does.
const BATCH_SIZE: usize = 16;

/// How long advertisements may wait to fill a batch.
const BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// Connections that send nothing for this long are closed. Clients ping
/// every 20 seconds or so.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Size of the buffer message payloads are encoded into. A full
/// advertisement batch is under 1.5 KiB.
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
    /// Key that encrypts every connection, or `None` for plaintext.
    pub encryption: Option<EncryptionKey>,
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

    let mut inbox = Vec::with_capacity(1024);
    let mut subscription: Option<broadcast::Receiver<Advertisement>> = None;
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut flush = tokio::time::interval(BATCH_INTERVAL);
    flush.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let idle = tokio::time::sleep(IDLE_TIMEOUT);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            read = reader.read_buf(&mut inbox) => {
                if read? == 0 {
                    return Ok(());
                }
                idle.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
                while let Some(Message { message_type, payload }) = out.next_message(&mut inbox).await? {
                    let message = ClientMessage::decode(message_type, &payload)?;
                    log_message(&message);
                    let outcome = session.handle(&message);
                    if let Some(reply) = outcome.reply {
                        out.reply(&reply).await?;
                    }
                    if let (Some(mode), Some(scanner)) = (outcome.scan_mode, &context.scanner) {
                        info!(?mode, "client requested a scan mode");
                        scanner.request(mode);
                    }
                    if outcome.close {
                        out.writer.shutdown().await?;
                        return Ok(());
                    }
                }
                // Follow subscription changes made by the messages above.
                match (&subscription, session.wants_raw_advertisements()) {
                    (None, true) => {
                        subscription = Some(context.advertisements.subscribe());
                        // Like ESPHome, report the scanner's state to new
                        // subscribers, then follow its changes.
                        if let Some(status) = &mut scanner_status {
                            let current = *status.borrow_and_update();
                            if let Some(reply) = session.scanner_state(current) {
                                out.reply(&reply).await?;
                            }
                        }
                    }
                    (Some(_), false) => {
                        subscription = None;
                        batch.clear();
                    }
                    _ => {}
                }
            }
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
            Some(status) = status_change(&mut scanner_status), if subscription.is_some() => {
                if let Some(reply) = session.scanner_state(status) {
                    out.reply(&reply).await?;
                }
            }
            _ = flush.tick(), if !batch.is_empty() => {
                out.advertisements(&mut batch).await?;
            }
            () = &mut idle => {
                info!("closing idle connection");
                return Ok(());
            }
        }
    }
}

/// Waits for the scanner's status to change. Never completes without a
/// scanner, or once the scanner is gone.
async fn status_change(
    status: &mut Option<watch::Receiver<ScannerStatus>>,
) -> Option<ScannerStatus> {
    if let Some(receiver) = status
        && receiver.changed().await.is_ok()
    {
        return Some(*receiver.borrow_and_update());
    }
    std::future::pending().await
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

    async fn advertisements(
        &mut self,
        batch: &mut Vec<Advertisement>,
    ) -> Result<(), ConnectionError> {
        let raw: Vec<RawAdvertisement<'_>> = batch.iter().map(Advertisement::as_raw).collect();
        let response = BluetoothLeRawAdvertisementsResponse {
            advertisements: &raw,
        };
        let len = response.encode_to_slice(&mut self.payload)?;
        self.send(BluetoothLeRawAdvertisementsResponse::TYPE, len)
            .await?;
        debug!(count = batch.len(), "forwarded advertisements");
        batch.clear();
        Ok(())
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
        ClientMessage::Unknown(message_type) => debug!(message_type, "ignoring unknown message"),
        other => debug!(?other, "received"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_buffer_fits_a_full_batch() {
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
