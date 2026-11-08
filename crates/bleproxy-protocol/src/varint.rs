//! Protobuf-style (unsigned LEB128) variable-length integers.

/// Maximum encoded length of a `u32`.
pub(crate) const MAX_U32_LEN: usize = 5;

/// The value does not fit in a `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overflow;

/// Writes `value` to the start of `buf`.
///
/// Returns the number of bytes written, or `None` if `buf` is too small.
pub(crate) fn encode_u32(mut value: u32, buf: &mut [u8]) -> Option<usize> {
    let mut written = 0;
    loop {
        let slot = buf.get_mut(written)?;
        let low = value.to_le_bytes()[0] & 0x7F;
        value >>= 7;
        written += 1;
        if value == 0 {
            *slot = low;
            return Some(written);
        }
        *slot = low | 0x80;
    }
}

/// Reads a `u32` from the start of `buf`.
///
/// Returns the value and the number of bytes it occupied, or `Ok(None)` if
/// `buf` ends before the varint does.
pub(crate) fn decode_u32(buf: &[u8]) -> Result<Option<(u32, usize)>, Overflow> {
    let mut value = 0_u32;
    for (i, &byte) in buf.iter().take(MAX_U32_LEN).enumerate() {
        // The fifth byte carries the top 4 bits and must end the varint.
        if i == MAX_U32_LEN - 1 && byte > 0x0F {
            return Err(Overflow);
        }
        value |= u32::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(Some((value, i + 1)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: u32) -> ([u8; MAX_U32_LEN], usize) {
        let mut buf = [0; MAX_U32_LEN];
        let len = encode_u32(value, &mut buf).unwrap();
        (buf, len)
    }

    #[test]
    fn known_encodings() {
        let cases: [(u32, &[u8]); 6] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7F]),
            (128, &[0x80, 0x01]),
            (300, &[0xAC, 0x02]),
            (u32::MAX, &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        ];
        for (value, expected) in cases {
            let (buf, len) = encode(value);
            assert_eq!(&buf[..len], expected, "{value}");
            assert_eq!(decode_u32(expected), Ok(Some((value, expected.len()))));
        }
    }

    #[test]
    fn round_trips_boundaries() {
        for value in [
            0,
            0x7F,
            0x80,
            0x3FFF,
            0x4000,
            0x1F_FFFF,
            0x20_0000,
            u32::MAX,
        ] {
            let (buf, len) = encode(value);
            assert_eq!(decode_u32(&buf[..len]), Ok(Some((value, len))));
        }
    }

    #[test]
    fn encode_reports_short_buffer() {
        assert_eq!(encode_u32(128, &mut [0; 1]), None);
        assert_eq!(encode_u32(0, &mut []), None);
    }

    #[test]
    fn decode_waits_for_more_input() {
        assert_eq!(decode_u32(&[]), Ok(None));
        assert_eq!(decode_u32(&[0x80]), Ok(None));
        assert_eq!(decode_u32(&[0xFF, 0xFF, 0xFF, 0xFF]), Ok(None));
    }

    #[test]
    fn decode_rejects_values_wider_than_u32() {
        assert_eq!(decode_u32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x1F]), Err(Overflow));
        assert_eq!(
            decode_u32(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]),
            Err(Overflow)
        );
    }
}
