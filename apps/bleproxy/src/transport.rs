//! Framing of API connections, plaintext or Noise-encrypted.
//!
//! A [`Transport`] turns received bytes into messages and messages into
//! bytes to send. The encrypted transport also runs the Noise handshake,
//! answering the client's handshake frames itself.

use bleproxy_protocol::noise::{
    self, DataHeader, HANDSHAKE_OK, HANDSHAKE_REJECT, MAC_FAILURE, MAX_HANDSHAKE_PAYLOAD_LEN,
    NOISE_HEADER_LEN, NOISE_PROTOCOL, PROLOGUE_PREFIX, PROTOCOL_ID,
};
use bleproxy_protocol::{FrameError, FrameHeader};

use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::{CryptoResolver, DefaultResolver, FallbackResolver};
use snow::types::{Cipher, Dh, Hash, Random};

use crate::encryption::EncryptionKey;

/// Largest plaintext message payload accepted, in bytes. Clients only send
/// small requests, so anything bigger is a broken or hostile peer.
pub const MAX_PAYLOAD_LEN: usize = 16 * 1024;

/// What a plaintext server answers to an encrypted client, as ESPHome does.
/// The leading `0x00` tells the client the device uses plaintext.
const PLAINTEXT_ONLY: &[u8] = b"\x00Bad indicator byte";

/// Length of the authentication tag added to each encrypted frame.
const TAG_LEN: usize = 16;

/// Errors that end a connection at the transport level.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The client sent an invalid frame.
    #[error("invalid frame: {0}")]
    Frame(#[from] FrameError),
    /// The client announced a message larger than [`MAX_PAYLOAD_LEN`].
    #[error("message of {0} bytes exceeds the {MAX_PAYLOAD_LEN} byte limit")]
    MessageTooLarge(usize),
    /// The Noise handshake failed; the reason was sent to the client.
    #[error("encryption handshake failed: {0}")]
    Handshake(&'static str),
    /// Encrypting, or decrypting a frame, failed.
    #[error("encryption: {0}")]
    Noise(#[from] snow::Error),
    /// A message was sent before the encryption handshake completed.
    #[error("message sent before the encryption handshake completed")]
    NotReady,
    /// The connection was used after a transport error.
    #[error("the transport failed earlier")]
    Failed,
}

/// A message received from the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Message type identifier.
    pub message_type: u16,
    /// Protobuf payload.
    pub payload: Vec<u8>,
}

/// The framing of one connection.
#[derive(Debug)]
pub enum Transport {
    /// Unencrypted ESPHome framing.
    Plaintext,
    /// Noise-encrypted framing.
    Noise(Box<NoiseTransport>),
}

impl Transport {
    /// Creates an encrypted transport for a server identifying itself as
    /// `name` with MAC address `mac` (12 lowercase hex digits).
    #[must_use]
    pub fn noise(key: &EncryptionKey, name: &str, mac: &str) -> Self {
        let mut hello = vec![PROTOCOL_ID];
        for field in [name, mac] {
            hello.extend_from_slice(field.as_bytes());
            hello.push(0);
        }
        Self::Noise(Box::new(NoiseTransport {
            key: key.clone(),
            server_hello: hello,
            state: NoiseState::ClientHello,
        }))
    }

    /// Takes the next complete message out of `inbox`.
    ///
    /// Returns `Ok(None)` when more input is needed. Bytes for the client
    /// (handshake frames, or rejections before an error) are appended to
    /// `out`; send them even when this returns an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the client violates the framing or the
    /// encryption handshake fails; the connection should then be closed.
    pub fn read(
        &mut self,
        inbox: &mut Vec<u8>,
        out: &mut Vec<u8>,
    ) -> Result<Option<Message>, TransportError> {
        match self {
            Self::Plaintext => read_plaintext(inbox, out),
            Self::Noise(noise) => noise.read(inbox, out),
        }
    }

    /// Appends a framed message to `out`.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot be framed or encrypted.
    pub fn write(
        &mut self,
        message_type: u16,
        payload: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), TransportError> {
        match self {
            Self::Plaintext => {
                let header = FrameHeader {
                    payload_len: u32::try_from(payload.len())
                        .map_err(|_| FrameError::PayloadTooLarge(payload.len()))?,
                    message_type,
                };
                let mut buf = [0; bleproxy_protocol::MAX_HEADER_LEN];
                let len = header.encode(&mut buf)?;
                out.extend_from_slice(buf.get(..len).unwrap_or_default());
                out.extend_from_slice(payload);
                Ok(())
            }
            Self::Noise(noise) => noise.write(message_type, payload, out),
        }
    }
}

fn read_plaintext(
    inbox: &mut Vec<u8>,
    out: &mut Vec<u8>,
) -> Result<Option<Message>, TransportError> {
    let (header, header_len) = match FrameHeader::decode(inbox) {
        Ok(Some(decoded)) => decoded,
        Ok(None) => return Ok(None),
        Err(FrameError::UnsupportedIndicator(indicator)) => {
            if indicator == noise::NOISE_INDICATOR {
                // An encrypted client: tell it this device uses plaintext.
                out.extend_from_slice(PLAINTEXT_ONLY);
            }
            return Err(FrameError::UnsupportedIndicator(indicator).into());
        }
        Err(err) => return Err(err.into()),
    };
    let payload_len = header.payload_len as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(TransportError::MessageTooLarge(payload_len));
    }
    let end = header_len + payload_len;
    if inbox.len() < end {
        return Ok(None);
    }
    let payload = inbox.get(header_len..end).unwrap_or_default().to_vec();
    inbox.drain(..end);
    Ok(Some(Message {
        message_type: header.message_type,
        payload,
    }))
}

/// Server side of a Noise-encrypted connection.
pub struct NoiseTransport {
    key: EncryptionKey,
    server_hello: Vec<u8>,
    state: NoiseState,
}

impl std::fmt::Debug for NoiseTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match self.state {
            NoiseState::ClientHello => "client hello",
            NoiseState::Handshake(_) => "handshake",
            NoiseState::Data(_) => "data",
            NoiseState::Failed => "failed",
        };
        f.debug_struct("NoiseTransport")
            .field("state", &state)
            .finish_non_exhaustive()
    }
}

enum NoiseState {
    ClientHello,
    Handshake(Box<snow::HandshakeState>),
    Data(Box<snow::TransportState>),
    Failed,
}

impl NoiseTransport {
    fn read(
        &mut self,
        inbox: &mut Vec<u8>,
        out: &mut Vec<u8>,
    ) -> Result<Option<Message>, TransportError> {
        loop {
            let Some(frame) = self.take_frame(inbox, out)? else {
                return Ok(None);
            };
            // Each step consumes the current state and stores the next one;
            // a failure leaves the transport failed, and the connection ends.
            let state = std::mem::replace(&mut self.state, NoiseState::Failed);
            self.state = match state {
                NoiseState::ClientHello => {
                    // The hello's contents are reserved; its length and
                    // contents extend the prologue, binding them to the
                    // handshake.
                    let len = u16::try_from(frame.len())
                        .map_err(|_| FrameError::PayloadTooLarge(frame.len()))?;
                    let mut prologue = PROLOGUE_PREFIX.to_vec();
                    prologue.extend_from_slice(&len.to_be_bytes());
                    prologue.extend_from_slice(&frame);
                    let handshake = noise_builder()?
                        .psk(0, self.key.as_bytes())?
                        .prologue(&prologue)?
                        .build_responder()?;
                    write_frame(&self.server_hello, out)?;
                    NoiseState::Handshake(Box::new(handshake))
                }
                NoiseState::Handshake(mut handshake) => {
                    let message = match frame.split_first() {
                        None => return reject(out, "Empty handshake message"),
                        Some((&HANDSHAKE_OK, message)) => message,
                        Some(_) => return reject(out, "Bad handshake error byte"),
                    };
                    let mut scratch = [0; MAX_HANDSHAKE_PAYLOAD_LEN];
                    match handshake.read_message(message, &mut scratch) {
                        Ok(_) => {}
                        Err(snow::Error::Decrypt) => return reject(out, MAC_FAILURE),
                        Err(_) => return reject(out, "Handshake error"),
                    }
                    let mut reply = [0; MAX_HANDSHAKE_PAYLOAD_LEN];
                    reply[0] = HANDSHAKE_OK;
                    let len = handshake.write_message(&[], &mut reply[1..])?;
                    write_frame(reply.get(..=len).unwrap_or_default(), out)?;
                    NoiseState::Data(Box::new(handshake.into_transport_mode()?))
                }
                NoiseState::Data(mut transport) => {
                    let mut plain = vec![0; frame.len()];
                    let len = transport.read_message(&frame, &mut plain)?;
                    let (header, payload) =
                        DataHeader::decode(plain.get(..len).unwrap_or_default())?;
                    let message = Message {
                        message_type: header.message_type,
                        payload: payload.to_vec(),
                    };
                    self.state = NoiseState::Data(transport);
                    return Ok(Some(message));
                }
                NoiseState::Failed => return Err(TransportError::Failed),
            };
        }
    }

    /// Takes the next complete frame's payload out of `inbox`.
    fn take_frame(
        &self,
        inbox: &mut Vec<u8>,
        out: &mut Vec<u8>,
    ) -> Result<Option<Vec<u8>>, TransportError> {
        let payload_len = match noise::decode_noise_header(inbox) {
            Ok(Some(len)) => len,
            Ok(None) => return Ok(None),
            Err(err) => {
                if !matches!(self.state, NoiseState::Data(_)) {
                    // Most likely a plaintext client. The reject frame makes
                    // it report that the device requires encryption.
                    write_reject(out, "Bad indicator byte")?;
                }
                return Err(err.into());
            }
        };
        if !matches!(self.state, NoiseState::Data(_)) && payload_len > MAX_HANDSHAKE_PAYLOAD_LEN {
            return reject(out, "Bad handshake packet len");
        }
        let end = NOISE_HEADER_LEN + payload_len;
        if inbox.len() < end {
            return Ok(None);
        }
        let frame = inbox
            .get(NOISE_HEADER_LEN..end)
            .unwrap_or_default()
            .to_vec();
        inbox.drain(..end);
        Ok(Some(frame))
    }

    fn write(
        &mut self,
        message_type: u16,
        payload: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), TransportError> {
        let NoiseState::Data(transport) = &mut self.state else {
            return Err(TransportError::NotReady);
        };
        let header = DataHeader {
            message_type,
            payload_len: u16::try_from(payload.len())
                .map_err(|_| FrameError::PayloadTooLarge(payload.len()))?,
        };
        let mut plain = Vec::with_capacity(DataHeader::LEN + payload.len());
        plain.extend_from_slice(&header.encode());
        plain.extend_from_slice(payload);
        let mut cipher = vec![0; plain.len() + TAG_LEN];
        let len = transport.write_message(&plain, &mut cipher)?;
        write_frame(cipher.get(..len).unwrap_or_default(), out)
    }
}

/// A Noise builder for ESPHome's protocol, using snow's primitives and the
/// operating system's random numbers.
fn noise_builder() -> Result<snow::Builder<'static>, snow::Error> {
    let resolver = FallbackResolver::new(Box::new(OsRandom), Box::new(DefaultResolver));
    Ok(snow::Builder::with_resolver(
        NOISE_PROTOCOL.parse()?,
        Box::new(resolver),
    ))
}

/// Resolves only snow's random number generator, to the operating system's.
struct OsRandom;

impl CryptoResolver for OsRandom {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(Self))
    }

    fn resolve_dh(&self, _: &DHChoice) -> Option<Box<dyn Dh>> {
        None
    }

    fn resolve_hash(&self, _: &HashChoice) -> Option<Box<dyn Hash>> {
        None
    }

    fn resolve_cipher(&self, _: &CipherChoice) -> Option<Box<dyn Cipher>> {
        None
    }
}

impl Random for OsRandom {
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), snow::Error> {
        getrandom::fill(dest).map_err(|_| snow::Error::Rng)
    }
}

fn write_frame(payload: &[u8], out: &mut Vec<u8>) -> Result<(), TransportError> {
    out.extend_from_slice(&noise::encode_noise_header(payload.len())?);
    out.extend_from_slice(payload);
    Ok(())
}

fn write_reject(out: &mut Vec<u8>, reason: &str) -> Result<(), TransportError> {
    let mut payload = vec![HANDSHAKE_REJECT];
    payload.extend_from_slice(reason.as_bytes());
    write_frame(&payload, out)
}

/// Sends a handshake rejection and fails with its reason.
fn reject<T>(out: &mut Vec<u8>, reason: &'static str) -> Result<T, TransportError> {
    write_reject(out, reason)?;
    Err(TransportError::Handshake(reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    fn key() -> EncryptionKey {
        KEY.parse().unwrap()
    }

    /// The client side of the handshake, as aioesphomeapi performs it.
    fn client(key: &EncryptionKey) -> snow::HandshakeState {
        let mut prologue = PROLOGUE_PREFIX.to_vec();
        prologue.extend_from_slice(&[0, 0]);
        noise_builder()
            .unwrap()
            .psk(0, key.as_bytes())
            .unwrap()
            .prologue(&prologue)
            .unwrap()
            .build_initiator()
            .unwrap()
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(payload, &mut bytes).unwrap();
        bytes
    }

    /// Runs the handshake; returns the server's output and the client's
    /// transport, if it succeeded.
    fn handshake(
        server: &mut Transport,
        client_key: &EncryptionKey,
    ) -> (Vec<u8>, Result<snow::TransportState, TransportError>) {
        let mut client = client(client_key);
        let mut first = [0; 64];
        let len = client.write_message(&[], &mut first).unwrap();
        let mut inbox = frame(&[]);
        let mut message = vec![HANDSHAKE_OK];
        message.extend_from_slice(&first[..len]);
        inbox.extend_from_slice(&frame(&message));

        let mut out = Vec::new();
        if let Err(err) = server.read(&mut inbox, &mut out) {
            return (out, Err(err));
        }
        // Server hello, then the handshake response.
        let hello_len = usize::from(u16::from_be_bytes([out[1], out[2]]));
        let response = &out[NOISE_HEADER_LEN + hello_len..];
        assert_eq!(response[NOISE_HEADER_LEN], HANDSHAKE_OK);
        let mut scratch = [0; 64];
        client
            .read_message(&response[NOISE_HEADER_LEN + 1..], &mut scratch)
            .unwrap();
        (out, Ok(client.into_transport_mode().unwrap()))
    }

    #[test]
    fn plaintext_round_trip() {
        let mut transport = Transport::Plaintext;
        let mut out = Vec::new();
        transport.write(7, &[0xAA], &mut out).unwrap();
        assert_eq!(out, [0x00, 0x01, 0x07, 0xAA]);

        let mut inbox = out.clone();
        inbox.extend_from_slice(&[0x00, 0x00]); // start of another frame
        let message = transport.read(&mut inbox, &mut Vec::new()).unwrap();
        assert_eq!(
            message,
            Some(Message {
                message_type: 7,
                payload: vec![0xAA],
            })
        );
        assert_eq!(inbox, [0x00, 0x00]);
        assert_eq!(transport.read(&mut inbox, &mut Vec::new()).unwrap(), None);
    }

    #[test]
    fn plaintext_rejects_oversized_and_encrypted_clients() {
        let mut inbox = vec![0x00, 0x80, 0x80, 0x02, 0x07];
        let err = Transport::Plaintext
            .read(&mut inbox, &mut Vec::new())
            .unwrap_err();
        assert!(matches!(err, TransportError::MessageTooLarge(32768)));

        let mut out = Vec::new();
        let mut inbox = frame(&[]);
        assert!(Transport::Plaintext.read(&mut inbox, &mut out).is_err());
        assert_eq!(out, PLAINTEXT_ONLY);
    }

    #[test]
    fn noise_handshake_and_messages() {
        let mut server = Transport::noise(&key(), "bleproxy-a1b2c3", "aabbcca1b2c3");
        let (out, client) = handshake(&mut server, &key());
        let mut client = client.unwrap();
        assert_eq!(&out[..NOISE_HEADER_LEN], [0x01, 0x00, 0x1E]);
        assert_eq!(&out[3..33], b"\x01bleproxy-a1b2c3\0aabbcca1b2c3\0");

        // Client to server.
        let plain = [0x00, 0x07, 0x00, 0x01, 0xAA];
        let mut cipher = [0; 64];
        let len = client.write_message(&plain, &mut cipher).unwrap();
        let mut inbox = frame(&cipher[..len]);
        let message = server.read(&mut inbox, &mut Vec::new()).unwrap();
        assert_eq!(
            message,
            Some(Message {
                message_type: 7,
                payload: vec![0xAA],
            })
        );

        // Server to client.
        let mut out = Vec::new();
        server.write(8, &[0xBB, 0xCC], &mut out).unwrap();
        let mut plain = [0; 64];
        let len = client
            .read_message(&out[NOISE_HEADER_LEN..], &mut plain)
            .unwrap();
        assert_eq!(&plain[..len], [0x00, 0x08, 0x00, 0x02, 0xBB, 0xCC]);
    }

    #[test]
    fn noise_rejects_a_wrong_key() {
        let mut server = Transport::noise(&key(), "x", "y");
        let wrong = "BwECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
            .parse()
            .unwrap();
        let (out, result) = handshake(&mut server, &wrong);
        assert!(matches!(
            result,
            Err(TransportError::Handshake(MAC_FAILURE))
        ));
        let reject = frame(b"\x01Handshake MAC failure");
        assert!(out.ends_with(&reject), "{out:02x?}");
    }

    #[test]
    fn noise_tells_plaintext_clients_to_encrypt() {
        let mut server = Transport::noise(&key(), "x", "y");
        let mut out = Vec::new();
        let mut inbox = vec![0x00, 0x00, 0x01];
        assert!(server.read(&mut inbox, &mut out).is_err());
        assert_eq!(out, frame(b"\x01Bad indicator byte"));
    }

    #[test]
    fn noise_refuses_to_send_before_the_handshake() {
        let mut server = Transport::noise(&key(), "x", "y");
        let err = server.write(8, &[], &mut Vec::new()).unwrap_err();
        assert!(matches!(err, TransportError::NotReady));
    }

    #[test]
    fn noise_limits_handshake_frames() {
        let mut server = Transport::noise(&key(), "x", "y");
        let mut out = Vec::new();
        let mut inbox = frame(&[0; MAX_HANDSHAKE_PAYLOAD_LEN + 1]);
        let err = server.read(&mut inbox, &mut out).unwrap_err();
        assert!(matches!(
            err,
            TransportError::Handshake("Bad handshake packet len")
        ));
    }
}
