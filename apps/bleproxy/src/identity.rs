//! How the proxy identifies itself to Home Assistant.

use bleproxy_core::BdAddr;
use bleproxy_core::scanner::ScanMode;
use bleproxy_core::session::DeviceIdentity;

/// Maximum length of a device name, as limited by the ESPHome API.
pub const MAX_NAME_LEN: usize = 31;

/// Errors produced when building an [`Identity`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    /// The device name is not a valid host name label.
    #[error(
        "invalid device name {0:?}: use 1-{MAX_NAME_LEN} lowercase letters, digits or hyphens, \
         not starting or ending with a hyphen"
    )]
    InvalidName(String),
}

/// The proxy's identity: names, addresses and version information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    name: String,
    friendly_name: String,
    mac_address: String,
    bluetooth_mac_address: String,
    model: String,
    server_info: String,
    scanner: Option<ScanMode>,
}

impl Identity {
    /// Manufacturer reported to clients.
    pub const MANUFACTURER: &'static str = "bleproxy";
    /// Project name reported to clients. Identifying as a project rather
    /// than plain ESPHome keeps Home Assistant from comparing our version
    /// with ESPHome releases.
    pub const PROJECT_NAME: &'static str = "fiam.bleproxy";
    /// Software version reported to clients.
    pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Creates an identity.
    ///
    /// `name` defaults to `bleproxy-` followed by the last three bytes of
    /// `mac`, which is also the default friendly name's suffix.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::InvalidName`] if `name` is not usable as a
    /// host name label.
    pub fn new(
        name: Option<String>,
        friendly_name: Option<String>,
        mac: BdAddr,
        bluetooth_mac: BdAddr,
    ) -> Result<Self, IdentityError> {
        let [.., d, e, f] = mac.to_bytes();
        let suffix = format!("{d:02x}{e:02x}{f:02x}");
        let name = name.unwrap_or_else(|| format!("bleproxy-{suffix}"));
        if !is_valid_name(&name) {
            return Err(IdentityError::InvalidName(name));
        }
        Ok(Self {
            friendly_name: friendly_name
                .unwrap_or_else(|| format!("bleproxy {}", suffix.to_uppercase())),
            name,
            mac_address: mac.to_string(),
            bluetooth_mac_address: bluetooth_mac.to_string(),
            model: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            server_info: format!("bleproxy {}", Self::VERSION),
            scanner: None,
        })
    }

    /// Declares a scanner configured with `mode` whose mode clients may
    /// change.
    #[must_use]
    pub const fn with_scanner(mut self, mode: ScanMode) -> Self {
        self.scanner = Some(mode);
        self
    }

    /// Device name, which is also the mDNS host name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-friendly name.
    #[must_use]
    pub fn friendly_name(&self) -> &str {
        &self.friendly_name
    }

    /// MAC address identifying the device, as `AA:BB:CC:DD:EE:FF`.
    #[must_use]
    pub fn mac_address(&self) -> &str {
        &self.mac_address
    }

    /// Model reported to clients (operating system and architecture).
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the identity in the form the API session uses.
    #[must_use]
    pub fn device_identity(&self) -> DeviceIdentity<'_> {
        DeviceIdentity {
            name: &self.name,
            friendly_name: &self.friendly_name,
            mac_address: &self.mac_address,
            bluetooth_mac_address: &self.bluetooth_mac_address,
            model: &self.model,
            manufacturer: Self::MANUFACTURER,
            project_name: Self::PROJECT_NAME,
            version: Self::VERSION,
            server_info: &self.server_info,
            scanner: self.scanner,
            encrypted: false,
        }
    }
}

/// Derives a stable, locally administered MAC address from `seed`, for
/// hosts without a Bluetooth adapter to take one from.
#[must_use]
pub fn derived_mac(seed: &str) -> BdAddr {
    // FNV-1a: tiny, stable across releases and platforms.
    let hash = seed.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    });
    // The low 48 bits of the hash, with the locally administered bit set
    // and the multicast bit cleared.
    let local = 0x0200_0000_0000;
    let multicast = 0x0100_0000_0000;
    let address = (hash | local) & !multicast;
    let mut bytes = [0; 6];
    bytes.copy_from_slice(&address.to_be_bytes()[2..]);
    BdAddr::new(bytes)
}

/// Checks that `name` can be used as a device name.
///
/// # Errors
///
/// Returns [`IdentityError::InvalidName`] if it is not a valid host name
/// label of at most [`MAX_NAME_LEN`] lowercase characters.
pub fn validate_name(name: &str) -> Result<(), IdentityError> {
    if is_valid_name(name) {
        Ok(())
    } else {
        Err(IdentityError::InvalidName(name.to_owned()))
    }
}

fn is_valid_name(name: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: BdAddr = BdAddr::new([0xAA, 0xBB, 0xCC, 0xA1, 0xB2, 0xC3]);

    #[test]
    fn defaults_names_from_the_mac() {
        let identity = Identity::new(None, None, MAC, MAC).unwrap();
        assert_eq!(identity.name(), "bleproxy-a1b2c3");
        assert_eq!(identity.friendly_name(), "bleproxy A1B2C3");
        assert_eq!(identity.mac_address(), "AA:BB:CC:A1:B2:C3");
        let device = identity.device_identity();
        assert_eq!(device.project_name, "fiam.bleproxy");
        assert_eq!(
            device.server_info,
            format!("bleproxy {}", Identity::VERSION)
        );
    }

    #[test]
    fn keeps_explicit_names() {
        let identity =
            Identity::new(Some("garage".into()), Some("Garage proxy".into()), MAC, MAC).unwrap();
        assert_eq!(identity.name(), "garage");
        assert_eq!(identity.friendly_name(), "Garage proxy");
    }

    #[test]
    fn rejects_invalid_names() {
        for name in [
            "",
            "Garage",
            "garage_1",
            "-garage",
            "garage-",
            &"a".repeat(32),
        ] {
            assert_eq!(
                Identity::new(Some(name.to_owned()), None, MAC, MAC),
                Err(IdentityError::InvalidName(name.to_owned())),
            );
        }
        assert!(Identity::new(Some("a".repeat(31)), None, MAC, MAC).is_ok());
    }

    #[test]
    fn derives_stable_local_unicast_macs() {
        let mac = derived_mac("my-host");
        assert_eq!(mac, derived_mac("my-host"));
        assert_ne!(mac, derived_mac("other-host"));
        let first = mac.to_bytes()[0];
        assert_eq!(first & 0x03, 0x02, "locally administered unicast");
    }
}
