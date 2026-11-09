//! The ESPHome Native API server: accepts connections and runs a
//! [`Session`] for each.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use bleproxy_core::Advertisement;
use bleproxy_core::scanner::ScannerStatus;
use bleproxy_core::session::{Reply, Session};
use bleproxy_protocol::wire::{DecodeError, EncodeError};
use bleproxy_protocol::{
    BluetoothLeRawAdvertisementsResponse, ClientMessage, FrameError, FrameHeader, RawAdvertisement,
    encode_frame,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{Instrument, debug, info, info_span, warn};

use crate::Identity;
use crate::scanner::ScannerHandle;

/// Largest client message payload accepted, in bytes. Clients only send
/// small requests, so anything bigger is a broken or hostile peer.
pub const MAX_PAYLOAD_LEN: usize = 16 * 1024;

/// Most advertisements sent in one message, as ESPHome does.
const BATCH_SIZE: usize = 16;

/// How long advertisements may wait to fill a batch.
const BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// Connections that send nothing for this long are closed. Clients ping
/// every 20 seconds or so.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Size of the buffer replies are encoded into. A full advertisement batch
/// is under 1.5 KiB.
const OUTPUT_BUFFER_LEN: usize = 4096;

/// Errors that end a client connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// Reading from or writing to the socket failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The client sent an invalid frame.
    #[error("invalid frame: {0}")]
    Frame(#[from] FrameError),
    /// The client sent an invalid message.
    #[error("invalid message: {0}")]
    Decode(#[from] DecodeError),
    /// The client announced a message larger than [`MAX_PAYLOAD_LEN`].
    #[error("message of {0} bytes exceeds the {MAX_PAYLOAD_LEN} byte limit")]
    MessageTooLarge(u32),
    /// A reply did not fit the output buffer.
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
    let (mut reader, mut writer) = stream.into_split();
    let device = context.identity.device_identity();
    let mut session = Session::new(&device);
    let mut scanner_status = context.scanner.as_ref().map(ScannerHandle::status);

    let mut inbox = Vec::with_capacity(1024);
    let mut out = vec![0; OUTPUT_BUFFER_LEN];
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
                while let Some((header, consumed)) = next_frame(&inbox)? {
                    let payload = inbox.get(consumed - header.payload_len as usize..consumed)
                        .unwrap_or_default();
                    let message = ClientMessage::decode(header.message_type, payload)?;
                    log_message(&message);
                    let outcome = session.handle(&message);
                    inbox.drain(..consumed);
                    if let Some(reply) = outcome.reply {
                        send_reply(&mut writer, &reply, &mut out).await?;
                    }
                    if let (Some(mode), Some(scanner)) = (outcome.scan_mode, &context.scanner) {
                        info!(?mode, "client requested a scan mode");
                        scanner.request(mode);
                    }
                    if outcome.close {
                        writer.shutdown().await?;
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
                                send_reply(&mut writer, &reply, &mut out).await?;
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
                        send_batch(&mut writer, &mut batch, &mut out).await?;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    warn!(missed, "client is too slow, dropped advertisements");
                }
                Err(broadcast::error::RecvError::Closed) => subscription = None,
            },
            Some(status) = status_change(&mut scanner_status), if subscription.is_some() => {
                if let Some(reply) = session.scanner_state(status) {
                    send_reply(&mut writer, &reply, &mut out).await?;
                }
            }
            _ = flush.tick(), if !batch.is_empty() => {
                send_batch(&mut writer, &mut batch, &mut out).await?;
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

async fn send_reply(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reply: &Reply<'_>,
    out: &mut [u8],
) -> Result<(), ConnectionError> {
    let len = reply.encode_frame(out)?;
    writer.write_all(out.get(..len).unwrap_or_default()).await?;
    Ok(())
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

/// Returns the header of the first complete frame in `buf` and the number
/// of bytes up to the end of its payload.
fn next_frame(buf: &[u8]) -> Result<Option<(FrameHeader, usize)>, ConnectionError> {
    let Some((header, header_len)) = FrameHeader::decode(buf)? else {
        return Ok(None);
    };
    let payload_len = header.payload_len as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(ConnectionError::MessageTooLarge(header.payload_len));
    }
    let end = header_len + payload_len;
    Ok((buf.len() >= end).then_some((header, end)))
}

async fn send_batch(
    writer: &mut (impl AsyncWriteExt + Unpin),
    batch: &mut Vec<Advertisement>,
    out: &mut [u8],
) -> Result<(), ConnectionError> {
    let raw: Vec<RawAdvertisement<'_>> = batch.iter().map(Advertisement::as_raw).collect();
    let response = BluetoothLeRawAdvertisementsResponse {
        advertisements: &raw,
    };
    let len = encode_frame(&response, out)?;
    writer.write_all(out.get(..len).unwrap_or_default()).await?;
    debug!(count = batch.len(), "forwarded advertisements");
    batch.clear();
    Ok(())
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
    use bleproxy_protocol::MAX_HEADER_LEN;

    use super::*;

    #[test]
    fn finds_complete_frames() {
        assert!(next_frame(&[]).unwrap().is_none());
        // Header announcing 2 bytes, only 1 present.
        assert!(next_frame(&[0x00, 0x02, 0x07, 0xAA]).unwrap().is_none());
        let (header, end) = next_frame(&[0x00, 0x02, 0x07, 0xAA, 0xBB, 0xCC])
            .unwrap()
            .unwrap();
        assert_eq!((header.payload_len, header.message_type, end), (2, 7, 5));
    }

    #[test]
    fn rejects_oversized_frames() {
        // 0x80 0x80 0x02 = 32768 bytes.
        let err = next_frame(&[0x00, 0x80, 0x80, 0x02, 0x07]).unwrap_err();
        assert!(matches!(err, ConnectionError::MessageTooLarge(32768)));
    }

    #[test]
    fn output_buffer_fits_a_full_batch() {
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
        let mut out = vec![0; OUTPUT_BUFFER_LEN];
        let len = encode_frame(&response, &mut out).unwrap();
        assert!(len + MAX_HEADER_LEN < OUTPUT_BUFFER_LEN);
    }
}
