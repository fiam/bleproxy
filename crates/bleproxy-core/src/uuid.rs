//! Bluetooth UUIDs.

use core::fmt;
use core::str::FromStr;

/// Error returned when parsing a [`Uuid`] fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("expected a UUID formatted as xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx")]
pub struct UuidParseError;

/// A 128-bit Bluetooth UUID.
///
/// Assigned numbers such as `0x180F` (Battery Service) are shorthands for
/// UUIDs derived from the Bluetooth Base UUID; [`Uuid::compact`] recovers
/// the shortest form, which is what advertising data uses.
///
/// ```
/// use bleproxy_core::{CompactUuid, Uuid};
///
/// let battery: Uuid = "0000180f-0000-1000-8000-00805f9b34fb".parse()?;
/// assert_eq!(battery, Uuid::from_u16(0x180F));
/// assert_eq!(battery.compact(), CompactUuid::U16(0x180F));
/// # Ok::<(), bleproxy_core::UuidParseError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid(u128);

/// The shortest representation of a [`Uuid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompactUuid {
    /// A 16-bit alias of a Base UUID.
    U16(u16),
    /// A 32-bit alias of a Base UUID.
    U32(u32),
    /// A UUID that is not derived from the Base UUID.
    U128(u128),
}

impl Uuid {
    /// The Bluetooth Base UUID, `00000000-0000-1000-8000-00805f9b34fb`.
    pub const BASE: Self = Self(0x0000_0000_0000_1000_8000_0080_5F9B_34FB);

    /// Creates a UUID from its 128-bit value.
    #[must_use]
    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    /// Creates the Base UUID derivative for a 16-bit alias.
    #[must_use]
    pub const fn from_u16(alias: u16) -> Self {
        Self::from_u32(alias as u32)
    }

    /// Creates the Base UUID derivative for a 32-bit alias.
    #[must_use]
    pub const fn from_u32(alias: u32) -> Self {
        Self(Self::BASE.0 | (alias as u128) << 96)
    }

    /// Returns the 128-bit value.
    #[must_use]
    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// Returns the shortest representation of this UUID.
    #[must_use]
    pub fn compact(self) -> CompactUuid {
        const LOW_96_BITS: u128 = (1 << 96) - 1;
        if self.0 & LOW_96_BITS != Self::BASE.0 {
            return CompactUuid::U128(self.0);
        }
        let alias = self.alias();
        u16::try_from(alias).map_or(CompactUuid::U32(alias), CompactUuid::U16)
    }

    /// Returns the top 32 bits, which hold the alias of a Base UUID
    /// derivative.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "shifting out 96 bits leaves at most 32"
    )]
    const fn alias(self) -> u32 {
        (self.0 >> 96) as u32
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            v >> 96,
            (v >> 80) & 0xFFFF,
            (v >> 64) & 0xFFFF,
            (v >> 48) & 0xFFFF,
            v & 0xFFFF_FFFF_FFFF
        )
    }
}

impl FromStr for Uuid {
    type Err = UuidParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != 36 {
            return Err(UuidParseError);
        }
        let mut value = 0_u128;
        for (i, &byte) in bytes.iter().enumerate() {
            if matches!(i, 8 | 13 | 18 | 23) {
                if byte != b'-' {
                    return Err(UuidParseError);
                }
                continue;
            }
            let digit = char::from(byte).to_digit(16).ok_or(UuidParseError)?;
            value = value << 4 | u128::from(digit);
        }
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::string::ToString;

    use super::*;

    #[test]
    fn compacts_base_uuid_derivatives() {
        assert_eq!(Uuid::from_u16(0xFE95).compact(), CompactUuid::U16(0xFE95));
        assert_eq!(
            Uuid::from_u32(0x1234_5678).compact(),
            CompactUuid::U32(0x1234_5678)
        );
        assert_eq!(Uuid::BASE.compact(), CompactUuid::U16(0));
        let custom = Uuid::from_u128(0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E);
        assert_eq!(custom.compact(), CompactUuid::U128(custom.as_u128()));
    }

    #[test]
    fn parses_and_formats() {
        let text = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
        let uuid: Uuid = text.parse().unwrap();
        assert_eq!(uuid.as_u128(), 0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E);
        assert_eq!(uuid.to_string(), text);
        assert_eq!(text.to_uppercase().parse(), Ok(uuid));
        assert_eq!(
            Uuid::from_u16(0x180F).to_string(),
            "0000180f-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn rejects_malformed_text() {
        for input in [
            "",
            "180f",
            "0000180f00001000800000805f9b34fb",
            "0000180f-0000-1000-8000-00805f9b34f",
            "0000180f-0000-1000-8000-00805f9b34fbb",
            "0000180f_0000-1000-8000-00805f9b34fb",
            "0000180g-0000-1000-8000-00805f9b34fb",
            "+000180f-0000-1000-8000-00805f9b34fb",
        ] {
            assert_eq!(input.parse::<Uuid>(), Err(UuidParseError), "{input:?}");
        }
    }
}
