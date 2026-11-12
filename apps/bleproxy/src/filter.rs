//! Limits the proxy to chosen devices.

use std::fmt;
use std::sync::Arc;

use bleproxy_core::BdAddr;

/// The devices the proxy forwards advertisements from and connects to.
///
/// Home Assistant only hears about allowed devices, which keeps a proxy
/// placed next to one device from relaying the whole neighborhood.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddressFilter {
    /// Allowed addresses; empty allows every device.
    allowed: Arc<[BdAddr]>,
}

impl AddressFilter {
    /// Allows only `addresses`, or every device if there are none.
    #[must_use]
    pub fn new(addresses: impl Into<Arc<[BdAddr]>>) -> Self {
        Self {
            allowed: addresses.into(),
        }
    }

    /// Returns whether the proxy may forward `address`.
    #[must_use]
    pub fn allows(&self, address: BdAddr) -> bool {
        self.allowed.is_empty() || self.allowed.contains(&address)
    }

    /// Returns whether some devices are left out.
    #[must_use]
    pub fn is_restricted(&self) -> bool {
        !self.allowed.is_empty()
    }
}

impl fmt::Display for AddressFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.allowed.is_empty() {
            return f.write_str("all devices");
        }
        for (i, address) in self.allowed.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{address}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALE: BdAddr = BdAddr::new([0x5C, 0xCA, 0xD3, 0xBA, 0xBA, 0xCC]);
    const OTHER: BdAddr = BdAddr::new([0xA4, 0xC1, 0x38, 0x00, 0x11, 0x22]);

    #[test]
    fn allows_everything_by_default() {
        let filter = AddressFilter::default();
        assert!(filter.allows(SCALE) && filter.allows(OTHER));
        assert!(!filter.is_restricted());
        assert_eq!(filter.to_string(), "all devices");
    }

    #[test]
    fn allows_only_listed_addresses() {
        let filter = AddressFilter::new(vec![SCALE]);
        assert!(filter.allows(SCALE));
        assert!(!filter.allows(OTHER));
        assert!(filter.is_restricted());
        assert_eq!(filter.to_string(), "5C:CA:D3:BA:BA:CC");
        let both = AddressFilter::new(vec![SCALE, OTHER]);
        assert!(both.allows(OTHER));
        assert_eq!(both.to_string(), "5C:CA:D3:BA:BA:CC, A4:C1:38:00:11:22");
    }
}
