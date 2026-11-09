//! ESPHome Native API plaintext frame headers.
//!
//! Every plaintext frame on the wire looks like this:
//!
//! ```text
//! +------+------------------+------------------+-----------------+
//! | 0x00 | payload length   | message type     | protobuf payload|
//! |  u8  | varint (u32)     | varint (u16)     | `length` bytes  |
//! +------+------------------+------------------+-----------------+
//! ```

use crate::Message;
use crate::varint;
use crate::wire::EncodeError;

/// First byte of every plaintext frame.
pub const PLAINTEXT_INDICATOR: u8 = 0x00;

/// Longest possible encoded frame header, in bytes.
pub const MAX_HEADER_LEN: usize = 1 + varint::MAX_U32_LEN + 3;

/// Errors produced while encoding or decoding frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FrameError {
    /// The frame starts with the indicator of the other framing: `0x00`
    /// ([`PLAINTEXT_INDICATOR`]) for plaintext, `0x01`
    /// ([`NOISE_INDICATOR`](crate::noise::NOISE_INDICATOR)) for encrypted
    /// connections, or something else entirely.
    #[error("unexpected frame indicator {0:#04x}")]
    UnsupportedIndicator(u8),
    /// A frame payload is too long for its length field.
    #[error("frame payload of {0} bytes is too large")]
    PayloadTooLarge(usize),
    /// A decrypted data frame is shorter than its header or its length
    /// doesn't match.
    #[error("malformed encrypted data frame")]
    MalformedDataFrame,
    /// A header field is wider than its type allows.
    #[error("frame header field is out of range")]
    FieldOverflow,
    /// The output buffer cannot hold the encoded header.
    #[error("buffer too small for the frame header")]
    BufferTooSmall,
}

impl From<varint::Overflow> for FrameError {
    fn from(_: varint::Overflow) -> Self {
        Self::FieldOverflow
    }
}

/// Header preceding each protobuf message in a plaintext frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHeader {
    /// Length of the protobuf payload that follows the header, in bytes.
    pub payload_len: u32,
    /// ESPHome message type identifier of the payload.
    pub message_type: u16,
}

impl FrameHeader {
    /// Writes the header to the start of `buf`.
    ///
    /// Returns the number of bytes written. A buffer of [`MAX_HEADER_LEN`]
    /// bytes is always large enough.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::BufferTooSmall`] if `buf` cannot hold the header.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, FrameError> {
        let (indicator, rest) = buf.split_first_mut().ok_or(FrameError::BufferTooSmall)?;
        *indicator = PLAINTEXT_INDICATOR;
        let len_bytes =
            varint::encode_u32(self.payload_len, rest).ok_or(FrameError::BufferTooSmall)?;
        let rest = rest
            .get_mut(len_bytes..)
            .ok_or(FrameError::BufferTooSmall)?;
        let type_bytes =
            varint::encode_u32(self.message_type.into(), rest).ok_or(FrameError::BufferTooSmall)?;
        Ok(1 + len_bytes + type_bytes)
    }

    /// Reads a header from the start of `buf`.
    ///
    /// Returns the header and the number of bytes it occupied, or `Ok(None)`
    /// if `buf` does not yet contain a complete header. Bytes after the header
    /// are left untouched, so this can be called directly on a receive buffer.
    ///
    /// ```
    /// use bleproxy_protocol::FrameHeader;
    ///
    /// // HelloRequest (type 1) with a 2-byte payload, followed by the payload.
    /// let wire = [0x00, 0x02, 0x01, 0xAA, 0xBB];
    /// let (header, consumed) = FrameHeader::decode(&wire)?.expect("complete header");
    /// assert_eq!(header, FrameHeader { payload_len: 2, message_type: 1 });
    /// assert_eq!(&wire[consumed..], [0xAA, 0xBB]);
    /// # Ok::<(), bleproxy_protocol::FrameError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::UnsupportedIndicator`] if `buf` does not start a
    /// plaintext frame, or [`FrameError::FieldOverflow`] if a field does not
    /// fit its type.
    pub fn decode(buf: &[u8]) -> Result<Option<(Self, usize)>, FrameError> {
        let Some((&indicator, rest)) = buf.split_first() else {
            return Ok(None);
        };
        if indicator != PLAINTEXT_INDICATOR {
            return Err(FrameError::UnsupportedIndicator(indicator));
        }
        let Some((payload_len, len_bytes)) = varint::decode_u32(rest)? else {
            return Ok(None);
        };
        let rest = rest.get(len_bytes..).unwrap_or_default();
        let Some((message_type, type_bytes)) = varint::decode_u32(rest)? else {
            return Ok(None);
        };
        let message_type = u16::try_from(message_type).map_err(|_| FrameError::FieldOverflow)?;
        let header = Self {
            payload_len,
            message_type,
        };
        Ok(Some((header, 1 + len_bytes + type_bytes)))
    }
}

/// Encodes `message` as a complete plaintext frame at the start of `buf`.
///
/// Returns the frame length. A buffer of [`MAX_HEADER_LEN`] bytes plus the
/// message's [`encoded_len`](crate::wire::Encode::encoded_len) is always
/// large enough.
///
/// ```
/// use bleproxy_protocol::{PingResponse, encode_frame};
///
/// let mut buf = [0; 16];
/// let len = encode_frame(&PingResponse, &mut buf)?;
/// assert_eq!(&buf[..len], [0x00, 0x00, 0x08]);
/// # Ok::<(), bleproxy_protocol::wire::EncodeError>(())
/// ```
///
/// # Errors
///
/// Returns [`EncodeError::BufferTooSmall`] if `buf` cannot hold the frame.
pub fn encode_frame<M: Message>(message: &M, buf: &mut [u8]) -> Result<usize, EncodeError> {
    let header = FrameHeader {
        payload_len: u32::try_from(message.encoded_len())
            .map_err(|_| EncodeError::BufferTooSmall)?,
        message_type: M::TYPE,
    };
    let header_len = header
        .encode(buf)
        .map_err(|_| EncodeError::BufferTooSmall)?;
    let payload = buf
        .get_mut(header_len..)
        .ok_or(EncodeError::BufferTooSmall)?;
    Ok(header_len + message.encode_to_slice(payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HelloResponse, PingResponse};

    fn encode(header: FrameHeader) -> ([u8; MAX_HEADER_LEN], usize) {
        let mut buf = [0; MAX_HEADER_LEN];
        let len = header.encode(&mut buf).unwrap();
        (buf, len)
    }

    #[test]
    fn encodes_known_header() {
        let header = FrameHeader {
            payload_len: 300,
            message_type: 1,
        };
        let (buf, len) = encode(header);
        assert_eq!(&buf[..len], [0x00, 0xAC, 0x02, 0x01]);
    }

    #[test]
    fn round_trips_extremes() {
        for payload_len in [0, 1, 127, 128, u32::MAX] {
            for message_type in [0, 1, 127, 128, u16::MAX] {
                let header = FrameHeader {
                    payload_len,
                    message_type,
                };
                let (buf, len) = encode(header);
                assert_eq!(FrameHeader::decode(&buf[..len]), Ok(Some((header, len))));
            }
        }
    }

    #[test]
    fn max_header_fits_in_max_header_len() {
        let header = FrameHeader {
            payload_len: u32::MAX,
            message_type: u16::MAX,
        };
        assert_eq!(encode(header).1, MAX_HEADER_LEN);
    }

    #[test]
    fn every_prefix_is_incomplete() {
        let header = FrameHeader {
            payload_len: u32::MAX,
            message_type: u16::MAX,
        };
        let (buf, len) = encode(header);
        for end in 0..len {
            assert_eq!(FrameHeader::decode(&buf[..end]), Ok(None), "prefix {end}");
        }
    }

    #[test]
    fn decode_leaves_trailing_bytes() {
        let wire = [0x00, 0x03, 0x07, 0xDE, 0xAD, 0xBE];
        let expected = FrameHeader {
            payload_len: 3,
            message_type: 7,
        };
        assert_eq!(FrameHeader::decode(&wire), Ok(Some((expected, 3))));
    }

    #[test]
    fn rejects_encrypted_frames() {
        assert_eq!(
            FrameHeader::decode(&[0x01, 0x00, 0x01]),
            Err(FrameError::UnsupportedIndicator(0x01))
        );
    }

    #[test]
    fn rejects_out_of_range_fields() {
        // Payload length wider than u32.
        assert_eq!(
            FrameHeader::decode(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F, 0x01]),
            Err(FrameError::FieldOverflow)
        );
        // Message type 65536, wider than u16.
        assert_eq!(
            FrameHeader::decode(&[0x00, 0x00, 0x80, 0x80, 0x04]),
            Err(FrameError::FieldOverflow)
        );
    }

    #[test]
    fn encodes_complete_frames() {
        let hello = HelloResponse {
            api_version_major: 1,
            api_version_minor: 14,
            server_info: "x",
            name: "y",
        };
        let mut buf = [0; 32];
        let len = encode_frame(&hello, &mut buf).unwrap();
        let (header, header_len) = FrameHeader::decode(&buf[..len]).unwrap().unwrap();
        assert_eq!(header.message_type, 2);
        assert_eq!(header.payload_len as usize, len - header_len);
        assert_eq!(
            &buf[header_len..len],
            [0x08, 0x01, 0x10, 0x0E, 0x1A, 0x01, b'x', 0x22, 0x01, b'y']
        );

        for short in 0..len {
            assert_eq!(
                encode_frame(&hello, &mut buf[..short]),
                Err(EncodeError::BufferTooSmall)
            );
        }
        assert_eq!(encode_frame(&PingResponse, &mut buf), Ok(3));
    }

    #[test]
    fn encode_reports_short_buffer() {
        let header = FrameHeader {
            payload_len: 300,
            message_type: 1,
        };
        for len in 0..4 {
            assert_eq!(
                header.encode(&mut [0; 4][..len]),
                Err(FrameError::BufferTooSmall),
                "len {len}"
            );
        }
    }
}
