//! Announces the proxy over mDNS, the way ESPHome devices do, so Home
//! Assistant discovers it without configuration.

use std::fmt;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tracing::warn;

use bleproxy_protocol::noise::NOISE_PROTOCOL;

use crate::Identity;

/// Service type ESPHome devices register.
pub const SERVICE_TYPE: &str = "_esphomelib._tcp.local.";

/// How long to wait for the goodbye packets when stopping.
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// A registered mDNS service; unregistered by [`Announcement::stop`].
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl fmt::Debug for Announcement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Announcement")
            .field("fullname", &self.fullname)
            .finish_non_exhaustive()
    }
}

impl Announcement {
    /// Starts announcing the API on `port` on every interface.
    ///
    /// The host name is the device name, which clients check against the
    /// name in the API hello. Addresses follow the host's interfaces as
    /// they change.
    ///
    /// # Errors
    ///
    /// Returns an error if the mDNS responder cannot start.
    pub fn start(identity: &Identity, port: u16) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;
        let host = format!("{}.local.", identity.name());
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            identity.name(),
            &host,
            (),
            port,
            &txt_records(identity)[..],
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        daemon.register(info)?;
        Ok(Self { daemon, fullname })
    }

    /// Withdraws the announcement, so clients notice right away, and stops
    /// the responder.
    pub fn stop(self) {
        match self.daemon.unregister(&self.fullname) {
            Ok(done) => {
                let _ = done.recv_timeout(STOP_TIMEOUT);
            }
            Err(err) => warn!(%err, "cannot withdraw the mDNS announcement"),
        }
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(STOP_TIMEOUT);
        }
    }
}

/// TXT records in the form ESPHome publishes them. Home Assistant uses
/// `mac` to recognize the device, `friendly_name` as its title, and asks
/// for the encryption key when `api_encryption` is present.
fn txt_records(identity: &Identity) -> Vec<(&'static str, String)> {
    let mut records = vec![
        ("friendly_name", identity.friendly_name().to_owned()),
        ("version", Identity::VERSION.to_owned()),
        ("mac", identity.mac_hex()),
        ("platform", std::env::consts::OS.to_owned()),
        ("board", std::env::consts::ARCH.to_owned()),
        ("project_name", Identity::PROJECT_NAME.to_owned()),
        ("project_version", Identity::VERSION.to_owned()),
    ];
    if identity.encrypted() {
        records.push(("api_encryption", NOISE_PROTOCOL.to_owned()));
    }
    records
}

#[cfg(test)]
mod tests {
    use bleproxy_core::BdAddr;

    use super::*;

    #[test]
    fn publishes_esphome_txt_records() {
        let mac = BdAddr::new([0xAA, 0xBB, 0xCC, 0xA1, 0xB2, 0xC3]);
        let identity = Identity::new(None, Some("Kitchen".into()), mac, mac).unwrap();
        let records = txt_records(&identity);
        let get = |key| {
            records
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("mac"), Some("aabbcca1b2c3"));
        assert_eq!(get("friendly_name"), Some("Kitchen"));
        assert_eq!(get("project_name"), Some("fiam.bleproxy"));
        assert_eq!(get("version"), Some(Identity::VERSION));
        assert_eq!(get("api_encryption"), None);

        let encrypted = txt_records(&identity.with_encryption());
        let encryption = encrypted.iter().find(|(key, _)| *key == "api_encryption");
        assert_eq!(
            encryption.map(|(_, value)| value.as_str()),
            Some("Noise_NNpsk0_25519_ChaChaPoly_SHA256")
        );
    }
}
