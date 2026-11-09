//! Building advertising data from its parsed fields.
//!
//! Some platforms (BlueZ over D-Bus, for example) report an advertisement's
//! fields already parsed rather than as raw bytes. [`AdvDataBuilder`] turns
//! them back into AD structures, which is the form the ESPHome API forwards
//! and Home Assistant parses.

use crate::{AdvData, CompactUuid, Uuid};

/// AD type identifiers (Bluetooth Assigned Numbers, section 2.3).
pub mod ad_type {
    /// Incomplete list of 16-bit service UUIDs.
    pub const INCOMPLETE_UUIDS_16: u8 = 0x02;
    /// Complete list of 16-bit service UUIDs.
    pub const COMPLETE_UUIDS_16: u8 = 0x03;
    /// Incomplete list of 32-bit service UUIDs.
    pub const INCOMPLETE_UUIDS_32: u8 = 0x04;
    /// Complete list of 32-bit service UUIDs.
    pub const COMPLETE_UUIDS_32: u8 = 0x05;
    /// Incomplete list of 128-bit service UUIDs.
    pub const INCOMPLETE_UUIDS_128: u8 = 0x06;
    /// Complete list of 128-bit service UUIDs.
    pub const COMPLETE_UUIDS_128: u8 = 0x07;
    /// Shortened local name.
    pub const SHORTENED_LOCAL_NAME: u8 = 0x08;
    /// Complete local name.
    pub const COMPLETE_LOCAL_NAME: u8 = 0x09;
    /// TX power level.
    pub const TX_POWER_LEVEL: u8 = 0x0A;
    /// Service data with a 16-bit UUID.
    pub const SERVICE_DATA_16: u8 = 0x16;
    /// Service data with a 32-bit UUID.
    pub const SERVICE_DATA_32: u8 = 0x20;
    /// Service data with a 128-bit UUID.
    pub const SERVICE_DATA_128: u8 = 0x21;
    /// Manufacturer specific data.
    pub const MANUFACTURER_SPECIFIC_DATA: u8 = 0xFF;
}

/// Builds [`AdvData`] one AD structure at a time.
///
/// Every method appends one or more structures and returns whether the
/// whole value fit. A structure that does not fit is skipped (or shortened,
/// where the format allows it), so callers should add the most important
/// fields first.
///
/// ```
/// use bleproxy_core::AdvDataBuilder;
///
/// let mut builder = AdvDataBuilder::new();
/// builder.manufacturer_data(0x004C, &[0x02, 0x15]);
/// builder.local_name("Kitchen");
/// let data = builder.build();
/// assert_eq!(
///     data.as_slice(),
///     [5, 0xFF, 0x4C, 0x00, 0x02, 0x15, 8, 0x09, b'K', b'i', b't', b'c', b'h', b'e', b'n']
/// );
/// ```
#[derive(Debug, Clone, Default)]
pub struct AdvDataBuilder {
    data: AdvData,
}

impl AdvDataBuilder {
    /// Creates an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a structure with an arbitrary type and value.
    pub fn push(&mut self, ad_type: u8, value: &[u8]) -> bool {
        self.data
            .push_structure(ad_type, value.len(), |out| out.copy_from_slice(value))
    }

    /// Appends manufacturer specific data for `company_id`.
    pub fn manufacturer_data(&mut self, company_id: u16, data: &[u8]) -> bool {
        self.data
            .push_structure(ad_type::MANUFACTURER_SPECIFIC_DATA, 2 + data.len(), |out| {
                let (id, rest) = out.split_at_mut(2);
                id.copy_from_slice(&company_id.to_le_bytes());
                rest.copy_from_slice(data);
            })
    }

    /// Appends service data for `uuid`, using its shortest form.
    pub fn service_data(&mut self, uuid: Uuid, data: &[u8]) -> bool {
        let (kind, uuid_bytes, uuid_len) = match uuid.compact() {
            CompactUuid::U16(alias) => (ad_type::SERVICE_DATA_16, le_bytes(alias.into()), 2),
            CompactUuid::U32(alias) => (ad_type::SERVICE_DATA_32, le_bytes(alias.into()), 4),
            CompactUuid::U128(value) => (ad_type::SERVICE_DATA_128, le_bytes(value), 16),
        };
        self.data
            .push_structure(kind, uuid_len + data.len(), |out| {
                let (id, rest) = out.split_at_mut(uuid_len);
                id.copy_from_slice(uuid_bytes.get(..uuid_len).unwrap_or_default());
                rest.copy_from_slice(data);
            })
    }

    /// Appends lists of service UUIDs, one per UUID size.
    ///
    /// A list that does not fit is replaced by an "incomplete" list holding
    /// as many UUIDs as fit.
    pub fn service_uuids(&mut self, uuids: &[Uuid]) -> bool {
        let classes = [
            (2, ad_type::COMPLETE_UUIDS_16, ad_type::INCOMPLETE_UUIDS_16),
            (4, ad_type::COMPLETE_UUIDS_32, ad_type::INCOMPLETE_UUIDS_32),
            (
                16,
                ad_type::COMPLETE_UUIDS_128,
                ad_type::INCOMPLETE_UUIDS_128,
            ),
        ];
        let mut complete = true;
        for (size, complete_type, incomplete_type) in classes {
            let matching = || {
                uuids
                    .iter()
                    .filter_map(move |uuid| compact_bytes(*uuid, size))
            };
            let count = matching().count();
            if count == 0 {
                continue;
            }
            let fitting = count.min(self.data.remaining().saturating_sub(2) / size);
            if fitting == 0 {
                complete = false;
                continue;
            }
            let kind = if fitting == count {
                complete_type
            } else {
                complete = false;
                incomplete_type
            };
            self.data.push_structure(kind, fitting * size, |out| {
                for (chunk, bytes) in out.chunks_exact_mut(size).zip(matching()) {
                    chunk.copy_from_slice(bytes.get(..size).unwrap_or_default());
                }
            });
        }
        complete
    }

    /// Appends the local name, shortening it if it does not fit.
    ///
    /// Returns `true` only if the complete name was added.
    pub fn local_name(&mut self, name: &str) -> bool {
        if name.is_empty() {
            return true;
        }
        if self.push(ad_type::COMPLETE_LOCAL_NAME, name.as_bytes()) {
            return true;
        }
        let room = self.data.remaining().saturating_sub(2);
        let mut cut = room.min(name.len());
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(prefix) = name.get(..cut).filter(|prefix| !prefix.is_empty()) {
            self.push(ad_type::SHORTENED_LOCAL_NAME, prefix.as_bytes());
        }
        false
    }

    /// Appends the TX power level in dBm.
    pub fn tx_power(&mut self, dbm: i8) -> bool {
        self.push(ad_type::TX_POWER_LEVEL, &dbm.to_le_bytes())
    }

    /// Returns the built advertising data.
    #[must_use]
    pub const fn build(self) -> AdvData {
        self.data
    }
}

/// Little-endian bytes of `value`, the byte order of advertising data.
const fn le_bytes(value: u128) -> [u8; 16] {
    value.to_le_bytes()
}

/// Returns the little-endian bytes of `uuid` if its compact form is `size`
/// bytes long.
fn compact_bytes(uuid: Uuid, size: usize) -> Option<[u8; 16]> {
    let (value, len) = match uuid.compact() {
        CompactUuid::U16(alias) => (u128::from(alias), 2),
        CompactUuid::U32(alias) => (u128::from(alias), 4),
        CompactUuid::U128(value) => (value, 16),
    };
    (len == size).then(|| le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUSTOM: Uuid = Uuid::from_u128(0x6E40_0001_B5A3_F393_E0A9_E50E_24DC_CA9E);

    #[test]
    fn manufacturer_and_service_data() {
        let mut b = AdvDataBuilder::new();
        assert!(b.manufacturer_data(0x0499, &[0x05, 0x12]));
        assert!(b.service_data(Uuid::from_u16(0xFCD2), &[0x40]));
        assert!(b.service_data(Uuid::from_u32(0x1234_5678), &[]));
        assert_eq!(
            b.build().as_slice(),
            [
                5, 0xFF, 0x99, 0x04, 0x05, 0x12, // manufacturer
                4, 0x16, 0xD2, 0xFC, 0x40, // 16-bit service data
                5, 0x20, 0x78, 0x56, 0x34, 0x12, // 32-bit service data
            ]
        );
    }

    #[test]
    fn service_data_with_128_bit_uuid() {
        let mut b = AdvDataBuilder::new();
        assert!(b.service_data(CUSTOM, &[0xAA]));
        let data = b.build();
        assert_eq!(&data.as_slice()[..2], [18, ad_type::SERVICE_DATA_128]);
        assert_eq!(data.as_slice()[2..18], CUSTOM.as_u128().to_le_bytes());
        assert_eq!(data.as_slice()[18], 0xAA);
    }

    #[test]
    fn groups_service_uuids_by_size() {
        let mut b = AdvDataBuilder::new();
        let uuids = [Uuid::from_u16(0x180F), CUSTOM, Uuid::from_u16(0x181A)];
        assert!(b.service_uuids(&uuids));
        let data = b.build();
        assert_eq!(
            &data.as_slice()[..6],
            [5, ad_type::COMPLETE_UUIDS_16, 0x0F, 0x18, 0x1A, 0x18]
        );
        assert_eq!(&data.as_slice()[6..8], [17, ad_type::COMPLETE_UUIDS_128]);
        assert_eq!(data.len(), 6 + 18);
    }

    #[test]
    fn truncates_uuid_lists_that_do_not_fit() {
        let mut b = AdvDataBuilder::new();
        assert!(b.push(0x01, &[0; 52])); // 54 bytes used, 8 left
        let uuids = [1, 2, 3, 4].map(Uuid::from_u16);
        assert!(!b.service_uuids(&uuids));
        assert_eq!(
            &b.build().as_slice()[54..],
            [7, ad_type::INCOMPLETE_UUIDS_16, 1, 0, 2, 0, 3, 0]
        );
    }

    #[test]
    fn shortens_names_on_char_boundaries() {
        let mut b = AdvDataBuilder::new();
        assert!(b.push(0x01, &[0; 55])); // 57 bytes used, 5 left: 3 for the name
        assert!(!b.local_name("añb-sensor"));
        // "añ" is 3 bytes, so the cut lands on a character boundary.
        assert_eq!(
            &b.build().as_slice()[57..],
            [4, ad_type::SHORTENED_LOCAL_NAME, b'a', 0xC3, 0xB1]
        );
    }

    #[test]
    fn complete_name_and_tx_power() {
        let mut b = AdvDataBuilder::new();
        assert!(b.local_name("ab"));
        assert!(b.local_name(""));
        assert!(b.tx_power(-4));
        assert_eq!(
            b.build().as_slice(),
            [
                3,
                ad_type::COMPLETE_LOCAL_NAME,
                b'a',
                b'b',
                2,
                ad_type::TX_POWER_LEVEL,
                0xFC
            ]
        );
    }

    #[test]
    fn rejected_structures_leave_data_untouched() {
        let mut b = AdvDataBuilder::new();
        assert!(b.push(0x01, &[0; 58])); // 60 bytes used, 2 left
        assert!(!b.manufacturer_data(0x004C, &[]));
        assert!(!b.tx_power(0));
        assert!(!b.local_name("x"));
        assert!(!b.service_uuids(&[CUSTOM]));
        assert_eq!(b.build().len(), 60);
    }

    #[test]
    fn fills_to_capacity() {
        let mut b = AdvDataBuilder::new();
        assert!(b.push(0x01, &[0; AdvData::MAX_LEN - 2]));
        assert_eq!(b.build().len(), AdvData::MAX_LEN);
        assert!(!AdvDataBuilder::new().push(0x01, &[0; AdvData::MAX_LEN - 1]));
    }
}
