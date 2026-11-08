//! Bluetooth LE advertisements.

use crate::BdAddr;

/// Errors produced when constructing advertisement types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdvertisementError {
    /// The payload is longer than a legacy advertising PDU allows.
    #[error("advertising data is {len} bytes, at most {max} are allowed", max = AdvData::MAX_LEN)]
    DataTooLong {
        /// Length of the rejected payload.
        len: usize,
    },
    /// The value is not a known address type.
    #[error("unknown address type {0}")]
    UnknownAddressType(u8),
}

/// Whether a device address is public (IEEE-assigned) or random.
///
/// The discriminants match the HCI encoding, which the ESPHome Native API
/// reuses for its `address_type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AddressType {
    /// A public device address.
    Public = 0,
    /// A random (static or private) device address.
    Random = 1,
}

impl From<AddressType> for u8 {
    fn from(kind: AddressType) -> Self {
        kind as Self
    }
}

impl TryFrom<u8> for AddressType {
    type Error = AdvertisementError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Public),
            1 => Ok(Self::Random),
            other => Err(AdvertisementError::UnknownAddressType(other)),
        }
    }
}

/// Payload of a single legacy advertising or scan response PDU.
///
/// Stored inline with a fixed capacity so advertisements can be passed
/// around without heap allocation, which matters on the scanning hot path
/// and on microcontrollers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdvData {
    len: u8,
    // Bytes past `len` are always zero, so the derived impls only see the
    // payload itself.
    bytes: [u8; Self::MAX_LEN],
}

impl AdvData {
    /// Maximum payload length of a legacy advertising PDU.
    pub const MAX_LEN: usize = 31;

    /// Copies `data` into a new payload.
    ///
    /// # Errors
    ///
    /// Returns [`AdvertisementError::DataTooLong`] if `data` is longer than
    /// [`AdvData::MAX_LEN`].
    pub fn new(data: &[u8]) -> Result<Self, AdvertisementError> {
        let too_long = AdvertisementError::DataTooLong { len: data.len() };
        let len = u8::try_from(data.len()).map_err(|_| too_long)?;
        let mut bytes = [0; Self::MAX_LEN];
        bytes
            .get_mut(..data.len())
            .ok_or(too_long)?
            .copy_from_slice(data);
        Ok(Self { len, bytes })
    }

    /// Returns the payload bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.split_at(usize::from(self.len)).0
    }

    /// Returns the payload length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns `true` if the payload is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl AsRef<[u8]> for AdvData {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// A received advertisement, independent of the platform that scanned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Advertisement {
    /// Address of the advertising device.
    pub address: BdAddr,
    /// Kind of `address`.
    pub address_type: AddressType,
    /// Received signal strength, in dBm.
    pub rssi: i8,
    /// Raw advertising data.
    pub data: AdvData,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_type_round_trips_through_hci_value() {
        for kind in [AddressType::Public, AddressType::Random] {
            assert_eq!(AddressType::try_from(u8::from(kind)), Ok(kind));
        }
        assert_eq!(
            AddressType::try_from(2),
            Err(AdvertisementError::UnknownAddressType(2))
        );
    }

    #[test]
    fn adv_data_keeps_payload() {
        let payload = [0x02, 0x01, 0x06, 0x03, 0xFF, 0x4C, 0x00];
        let data = AdvData::new(&payload).unwrap();
        assert_eq!(data.as_slice(), payload);
        assert_eq!(data.len(), payload.len());
        assert!(!data.is_empty());
    }

    #[test]
    fn adv_data_accepts_empty_and_full_payloads() {
        let empty = AdvData::new(&[]).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.as_slice(), [] as [u8; 0]);

        let full = [0xAB; AdvData::MAX_LEN];
        assert_eq!(AdvData::new(&full).unwrap().as_slice(), full);
    }

    #[test]
    fn adv_data_rejects_oversized_payloads() {
        for len in [AdvData::MAX_LEN + 1, 255, 256, 1024] {
            let payload = [0; 1024];
            assert_eq!(
                AdvData::new(&payload[..len]),
                Err(AdvertisementError::DataTooLong { len })
            );
        }
    }

    #[test]
    fn adv_data_equality_depends_only_on_payload() {
        let a = AdvData::new(&[1, 2, 3]).unwrap();
        let b = AdvData::new(&[1, 2, 3]).unwrap();
        let c = AdvData::new(&[1, 2, 3, 0]).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
