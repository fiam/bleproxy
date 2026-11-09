//! Framing of Noise-encrypted connections, ESPHome's API encryption.
//!
//! Encrypted connections use `Noise_NNpsk0_25519_ChaChaPoly_SHA256` with a
//! 32-byte pre-shared key. Every frame is a 3-byte header (`0x01`, then the
//! payload length as a big-endian `u16`) followed by the payload:
//!
//! 1. The client sends a hello frame, normally empty. Its length and
//!    contents extend the [`PROLOGUE_PREFIX`].
//! 2. The server answers with its hello: [`PROTOCOL_ID`], then the device
//!    name and MAC address, each terminated by a zero byte.
//! 3. The client sends [`HANDSHAKE_OK`] followed by the first Noise
//!    handshake message; the server replies the same way, or with
//!    [`HANDSHAKE_REJECT`] followed by a reason such as [`MAC_FAILURE`].
//! 4. Each later frame's payload is an encrypted [`DataHeader`] followed by
//!    the protobuf message.
//!
//! The cryptography itself is left to the platform.

use crate::FrameError;

/// The Noise protocol name.
pub const NOISE_PROTOCOL: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";

/// First byte of every encrypted frame.
pub const NOISE_INDICATOR: u8 = 0x01;

/// Length of an encrypted frame's header.
pub const NOISE_HEADER_LEN: usize = 3;

/// Largest frame payload accepted before the handshake completes.
pub const MAX_HANDSHAKE_PAYLOAD_LEN: usize = 128;

/// Start of the Noise prologue; the client hello's length (big-endian
/// `u16`) and contents follow.
pub const PROLOGUE_PREFIX: &[u8] = b"NoiseAPIInit";

/// First byte of the server hello: the chosen protocol, [`NOISE_PROTOCOL`].
pub const PROTOCOL_ID: u8 = 0x01;

/// Status byte of a handshake frame that carries a Noise message.
pub const HANDSHAKE_OK: u8 = 0x00;

/// Status byte of a handshake frame that rejects the handshake; a reason
/// follows as text.
pub const HANDSHAKE_REJECT: u8 = 0x01;

/// Reject reason clients recognize as a wrong encryption key.
pub const MAC_FAILURE: &str = "Handshake MAC failure";

/// Encodes the header of an encrypted frame carrying `payload_len` bytes.
///
/// # Errors
///
/// Returns [`FrameError::PayloadTooLarge`] if the length doesn't fit a
/// `u16`.
pub fn encode_noise_header(payload_len: usize) -> Result<[u8; NOISE_HEADER_LEN], FrameError> {
    let len = u16::try_from(payload_len).map_err(|_| FrameError::PayloadTooLarge(payload_len))?;
    let [high, low] = len.to_be_bytes();
    Ok([NOISE_INDICATOR, high, low])
}

/// Reads the header of an encrypted frame at the start of `buf`, returning
/// the payload length, or `Ok(None)` if `buf` is shorter than a header.
///
/// # Errors
///
/// Returns [`FrameError::UnsupportedIndicator`] if `buf` doesn't start an
/// encrypted frame (for example, `0x00` for a plaintext client).
pub fn decode_noise_header(buf: &[u8]) -> Result<Option<usize>, FrameError> {
    let Some(&[indicator, high, low]) = buf.first_chunk::<NOISE_HEADER_LEN>() else {
        return match buf.first() {
            Some(&indicator) if indicator != NOISE_INDICATOR => {
                Err(FrameError::UnsupportedIndicator(indicator))
            }
            _ => Ok(None),
        };
    };
    if indicator != NOISE_INDICATOR {
        return Err(FrameError::UnsupportedIndicator(indicator));
    }
    Ok(Some(usize::from(u16::from_be_bytes([high, low]))))
}

/// Header inside each decrypted data frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataHeader {
    /// Message type identifier.
    pub message_type: u16,
    /// Length of the protobuf payload that follows.
    pub payload_len: u16,
}

impl DataHeader {
    /// Encoded length.
    pub const LEN: usize = 4;

    /// Encodes the header (both fields big-endian).
    #[must_use]
    pub const fn encode(&self) -> [u8; Self::LEN] {
        let [type_high, type_low] = self.message_type.to_be_bytes();
        let [len_high, len_low] = self.payload_len.to_be_bytes();
        [type_high, type_low, len_high, len_low]
    }

    /// Reads a header from the start of a decrypted frame, returning it and
    /// the payload that follows it.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::MalformedDataFrame`] if the frame is shorter
    /// than the header or the payload length doesn't match.
    pub fn decode(frame: &[u8]) -> Result<(Self, &[u8]), FrameError> {
        let Some((&[type_high, type_low, len_high, len_low], payload)) =
            frame.split_first_chunk::<{ Self::LEN }>()
        else {
            return Err(FrameError::MalformedDataFrame);
        };
        let header = Self {
            message_type: u16::from_be_bytes([type_high, type_low]),
            payload_len: u16::from_be_bytes([len_high, len_low]),
        };
        if usize::from(header.payload_len) != payload.len() {
            return Err(FrameError::MalformedDataFrame);
        }
        Ok((header, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_frame_headers() {
        assert_eq!(encode_noise_header(0), Ok([0x01, 0x00, 0x00]));
        assert_eq!(encode_noise_header(0x0102), Ok([0x01, 0x01, 0x02]));
        assert_eq!(
            encode_noise_header(65_536),
            Err(FrameError::PayloadTooLarge(65_536))
        );
        assert_eq!(
            decode_noise_header(&[0x01, 0x01, 0x02, 0xFF]),
            Ok(Some(0x0102))
        );
        assert_eq!(decode_noise_header(&[0x01, 0x01]), Ok(None));
        assert_eq!(decode_noise_header(&[]), Ok(None));
    }

    #[test]
    fn rejects_plaintext_frames_right_away() {
        // A plaintext client is recognizable from its first byte.
        assert_eq!(
            decode_noise_header(&[0x00]),
            Err(FrameError::UnsupportedIndicator(0x00))
        );
        assert_eq!(
            decode_noise_header(&[0x00, 0x02, 0x01]),
            Err(FrameError::UnsupportedIndicator(0x00))
        );
    }

    #[test]
    fn data_headers_round_trip() {
        let header = DataHeader {
            message_type: 93,
            payload_len: 2,
        };
        let mut frame = [0; 6];
        frame[..4].copy_from_slice(&header.encode());
        frame[4..].copy_from_slice(&[0xAA, 0xBB]);
        assert_eq!(frame[..4], [0x00, 93, 0x00, 0x02]);
        assert_eq!(DataHeader::decode(&frame), Ok((header, &[0xAA, 0xBB][..])));
    }

    #[test]
    fn rejects_malformed_data_frames() {
        assert_eq!(
            DataHeader::decode(&[0x00, 0x07]),
            Err(FrameError::MalformedDataFrame)
        );
        // Announces 2 payload bytes, carries 1.
        assert_eq!(
            DataHeader::decode(&[0x00, 0x07, 0x00, 0x02, 0xAA]),
            Err(FrameError::MalformedDataFrame)
        );
    }
}
