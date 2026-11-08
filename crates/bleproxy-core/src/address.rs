//! Bluetooth device addresses.

use core::fmt;
use core::str::FromStr;

/// Largest value representable by a 48-bit address.
const MAX_ADDRESS: u64 = 0xFFFF_FFFF_FFFF;

/// Errors produced when constructing a [`BdAddr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AddressError {
    /// The integer does not fit in 48 bits.
    #[error("address value {0:#x} does not fit in 48 bits")]
    OutOfRange(u64),
    /// The text is not six colon-separated pairs of hex digits.
    #[error("expected an address formatted as AA:BB:CC:DD:EE:FF")]
    InvalidSyntax,
}

/// A 48-bit Bluetooth device address (`BD_ADDR`).
///
/// Bytes are stored most significant first, in the same order as the
/// conventional `AA:BB:CC:DD:EE:FF` notation. The ESPHome Native API carries
/// addresses as `u64`, so lossless conversions to and from `u64` are provided.
///
/// ```
/// use bleproxy_core::BdAddr;
///
/// let addr: BdAddr = "A4:C1:38:00:11:22".parse()?;
/// assert_eq!(u64::from(addr), 0xA4C1_3800_1122);
/// assert_eq!(addr.to_string(), "A4:C1:38:00:11:22");
/// # Ok::<(), bleproxy_core::AddressError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BdAddr([u8; 6]);

impl BdAddr {
    /// Creates an address from its bytes, most significant first.
    #[must_use]
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    /// Returns the address bytes, most significant first.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 6] {
        self.0
    }

    /// Returns the address as a 48-bit integer, as used by the ESPHome API.
    #[must_use]
    pub const fn to_u64(self) -> u64 {
        let mut bytes = [0; 8];
        bytes.split_at_mut(2).1.copy_from_slice(&self.0);
        u64::from_be_bytes(bytes)
    }

    /// Creates an address from a 48-bit integer.
    ///
    /// # Errors
    ///
    /// Returns [`AddressError::OutOfRange`] if `value` needs more than 48 bits.
    pub const fn from_u64(value: u64) -> Result<Self, AddressError> {
        if value > MAX_ADDRESS {
            return Err(AddressError::OutOfRange(value));
        }
        let mut bytes = [0; 6];
        bytes.copy_from_slice(value.to_be_bytes().split_at(2).1);
        Ok(Self(bytes))
    }
}

impl From<BdAddr> for u64 {
    fn from(addr: BdAddr) -> Self {
        addr.to_u64()
    }
}

impl TryFrom<u64> for BdAddr {
    type Error = AddressError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::from_u64(value)
    }
}

impl fmt::Display for BdAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{byte:02X}")?;
        }
        Ok(())
    }
}

impl FromStr for BdAddr {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0_u8; 6];
        let mut parts = s.split(':');
        for byte in &mut bytes {
            let part = parts.next().ok_or(AddressError::InvalidSyntax)?;
            // `from_str_radix` accepts a leading sign, so validate digits first.
            if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(AddressError::InvalidSyntax);
            }
            *byte = u8::from_str_radix(part, 16).map_err(|_| AddressError::InvalidSyntax)?;
        }
        if parts.next().is_some() {
            return Err(AddressError::InvalidSyntax);
        }
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::string::ToString;

    use super::*;

    const ADDR: BdAddr = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);

    #[test]
    fn u64_round_trip() {
        assert_eq!(ADDR.to_u64(), 0xA4C1_3800_1122);
        assert_eq!(BdAddr::from_u64(0xA4C1_3800_1122), Ok(ADDR));
        assert_eq!(BdAddr::try_from(u64::from(ADDR)), Ok(ADDR));
    }

    #[test]
    fn u64_bounds() {
        assert_eq!(BdAddr::from_u64(MAX_ADDRESS), Ok(BdAddr::new([0xFF; 6])));
        assert_eq!(
            BdAddr::from_u64(MAX_ADDRESS + 1),
            Err(AddressError::OutOfRange(MAX_ADDRESS + 1))
        );
        assert_eq!(BdAddr::from_u64(0), Ok(BdAddr::new([0; 6])));
    }

    #[test]
    fn display_is_uppercase_colon_separated() {
        assert_eq!(ADDR.to_string(), "A4:C1:38:00:11:22");
    }

    #[test]
    fn parses_either_case() {
        assert_eq!("A4:C1:38:00:11:22".parse(), Ok(ADDR));
        assert_eq!("a4:c1:38:00:11:22".parse(), Ok(ADDR));
    }

    #[test]
    fn display_parse_round_trip() {
        assert_eq!(ADDR.to_string().parse(), Ok(ADDR));
    }

    #[test]
    fn rejects_malformed_text() {
        for input in [
            "",
            "A4:C1:38:00:11",
            "A4:C1:38:00:11:22:33",
            "A4:C1:38:00:11:2",
            "A4-C1-38-00-11-22",
            "A4:C1:38:00:11:+2",
            "A4:C1:38:00:11:G2",
            "A4:C1:38:00:11:22:",
        ] {
            assert_eq!(
                input.parse::<BdAddr>(),
                Err(AddressError::InvalidSyntax),
                "{input:?}"
            );
        }
    }
}
