//! Protobuf-style (unsigned LEB128) variable-length integers.

/// Maximum encoded length of a `u32`.
pub(crate) const MAX_U32_LEN: usize = 5;

/// Maximum encoded length of a `u64`.
pub(crate) const MAX_U64_LEN: usize = 10;

/// The value does not fit in the requested integer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overflow;

/// Encodes `value` into a scratch buffer, returning it and the used length.
pub(crate) fn encode(mut value: u64) -> ([u8; MAX_U64_LEN], usize) {
    let mut buf = [0; MAX_U64_LEN];
    let mut len = 0;
    for slot in &mut buf {
        let low = value.to_le_bytes()[0] & 0x7F;
        value >>= 7;
        len += 1;
        if value == 0 {
            *slot = low;
            break;
        }
        *slot = low | 0x80;
    }
    (buf, len)
}

/// Writes `value` to the start of `buf`.
///
/// Returns the number of bytes written, or `None` if `buf` is too small.
pub(crate) fn encode_u32(value: u32, buf: &mut [u8]) -> Option<usize> {
    let (bytes, len) = encode(value.into());
    buf.get_mut(..len)?.copy_from_slice(bytes.get(..len)?);
    Some(len)
}

/// Reads a `u64` from the start of `buf`.
///
/// Returns the value and the number of bytes it occupied, or `Ok(None)` if
/// `buf` ends before the varint does.
pub(crate) fn decode_u64(buf: &[u8]) -> Result<Option<(u64, usize)>, Overflow> {
    let mut value = 0_u64;
    for (i, &byte) in buf.iter().take(MAX_U64_LEN).enumerate() {
        // The tenth byte carries the top bit and must end the varint.
        if i == MAX_U64_LEN - 1 && byte > 0x01 {
            return Err(Overflow);
        }
        value |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(Some((value, i + 1)));
        }
    }
    Ok(None)
}

/// Reads a `u32` from the start of `buf`.
///
/// Returns the value and the number of bytes it occupied, or `Ok(None)` if
/// `buf` ends before the varint does.
pub(crate) fn decode_u32(buf: &[u8]) -> Result<Option<(u32, usize)>, Overflow> {
    // A u32 varint is at most five bytes long; reject longer ones before
    // they are mistaken for incomplete input.
    let window = buf.get(..MAX_U32_LEN).unwrap_or(buf);
    match decode_u64(window) {
        Ok(Some((value, len))) => u32::try_from(value)
            .map(|value| Some((value, len)))
            .map_err(|_| Overflow),
        Ok(None) if window.len() == MAX_U32_LEN => Err(Overflow),
        Ok(None) => Ok(None),
        Err(Overflow) => Err(Overflow),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_encodings() {
        let cases: [(u64, &[u8]); 7] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7F]),
            (128, &[0x80, 0x01]),
            (300, &[0xAC, 0x02]),
            (u32::MAX.into(), &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
            (
                u64::MAX,
                &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01],
            ),
        ];
        for (value, expected) in cases {
            let (buf, len) = encode(value);
            assert_eq!(&buf[..len], expected, "{value}");
            assert_eq!(decode_u64(expected), Ok(Some((value, expected.len()))));
        }
    }

    #[test]
    fn round_trips_boundaries() {
        for shift in 0..64 {
            for value in [1_u64 << shift, (1_u64 << shift) - 1] {
                let (buf, len) = encode(value);
                assert_eq!(decode_u64(&buf[..len]), Ok(Some((value, len))));
            }
        }
    }

    #[test]
    fn encode_u32_reports_short_buffer() {
        assert_eq!(encode_u32(128, &mut [0; 1]), None);
        assert_eq!(encode_u32(0, &mut []), None);
        assert_eq!(encode_u32(128, &mut [0; 2]), Some(2));
    }

    #[test]
    fn decode_waits_for_more_input() {
        assert_eq!(decode_u64(&[]), Ok(None));
        assert_eq!(decode_u64(&[0x80]), Ok(None));
        assert_eq!(decode_u32(&[0xFF, 0xFF, 0xFF, 0xFF]), Ok(None));
    }

    #[test]
    fn decode_rejects_overlong_values() {
        assert_eq!(decode_u32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x1F]), Err(Overflow));
        assert_eq!(
            decode_u32(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]),
            Err(Overflow)
        );
        assert_eq!(
            decode_u64(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02]),
            Err(Overflow)
        );
    }
}
