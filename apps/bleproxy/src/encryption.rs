//! The API encryption key.

use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Error returned when an encryption key cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the encryption key must be 32 bytes encoded as base64 (try `openssl rand -base64 32`)")]
pub struct InvalidKey;

/// A 32-byte pre-shared key for the ESPHome API's Noise encryption, in the
/// base64 form ESPHome configurations and Home Assistant use.
#[derive(Clone, PartialEq, Eq)]
pub struct EncryptionKey([u8; 32]);

impl EncryptionKey {
    /// Returns the raw key.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl FromStr for EncryptionKey {
    type Err = InvalidKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Tolerate surrounding whitespace, as in key files.
        let bytes = STANDARD.decode(s.trim()).map_err(|_| InvalidKey)?;
        bytes.try_into().map(Self).map_err(|_| InvalidKey)
    }
}

impl fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the secret, even in debug logs.
        f.write_str("EncryptionKey(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    #[test]
    fn parses_base64_keys() {
        let key: EncryptionKey = KEY.parse().unwrap();
        assert_eq!(key.as_bytes()[..4], [0, 1, 2, 3]);
        assert_eq!(format!(" {KEY}\n").parse(), Ok(key));
    }

    #[test]
    fn rejects_malformed_keys() {
        for input in [
            "",
            "not base64!",
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHg==",
            "AAEC",
        ] {
            assert_eq!(input.parse::<EncryptionKey>(), Err(InvalidKey), "{input:?}");
        }
    }

    #[test]
    fn keeps_the_key_out_of_debug_output() {
        let key: EncryptionKey = KEY.parse().unwrap();
        assert_eq!(format!("{key:?}"), "EncryptionKey(..)");
    }
}
