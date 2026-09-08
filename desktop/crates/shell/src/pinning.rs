//! Which phones the desktop will talk to.
//!
//! `SPEC.md` §5.2 replaces certificate authorities with pinning: each side remembers the
//! other's public key, and nothing else is trusted. Outside the window where a person has
//! opened pairing on the desktop, an unknown peer is refused rather than questioned.
//!
//! The device a connection belongs to is decided here, from the key that authenticated it,
//! and never from a device identifier inside a message. A phone can say anything in a
//! message body; it cannot say anything with a key it does not hold.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use photo_sync_core::RunningDigest;
use photo_sync_core::id::{DeviceId, Sha256};
use serde::{Deserialize, Serialize};

const PAIRED_FILE: &str = "paired.toml";

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("cannot read or write the paired phones: {0}")]
    Storage(String),

    #[error("the paired phones could not be read: {0}")]
    Malformed(String),
}

/// What the desktop decided about a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// A phone the desktop knows. Its identifier comes from the key, not from the phone.
    Known(DeviceId),

    /// Pairing is open and this key is new, so a person is asked to confirm the code.
    Offered { key: Vec<u8> },

    /// Not paired, and nobody opened pairing.
    Refused,
}

/// One phone the desktop has agreed to talk to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedPhone {
    pub device: String,

    /// What the phone calls itself, for the window to display.
    pub name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paired {
    /// Keyed by the hexadecimal digest of the phone's `SubjectPublicKeyInfo`, which is the
    /// only thing a connection proves.
    #[serde(default, rename = "phone")]
    phones: BTreeMap<String, PairedPhone>,
}

impl Paired {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(directory: &Path) -> Result<Self, PairingError> {
        let path = file(directory);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => return Err(PairingError::Storage(error.to_string())),
        };
        toml::from_str(&text).map_err(|error| PairingError::Malformed(error.to_string()))
    }

    pub fn save(&self, directory: &Path) -> Result<(), PairingError> {
        std::fs::create_dir_all(directory)
            .map_err(|error| PairingError::Storage(error.to_string()))?;
        let text = toml::to_string_pretty(self)
            .map_err(|error| PairingError::Malformed(error.to_string()))?;
        std::fs::write(file(directory), text)
            .map_err(|error| PairingError::Storage(error.to_string()))
    }

    /// Remembers a phone by the key it authenticated with.
    pub fn pair(&mut self, key: &[u8], device: &DeviceId, name: &str) {
        self.phones.insert(
            digest_of(key).to_hex(),
            PairedPhone {
                device: device.to_string(),
                name: name.to_string(),
            },
        );
    }

    /// Forgets a phone. `SPEC.md` §5.2 offers this on both sides.
    pub fn unpair(&mut self, device: &DeviceId) {
        self.phones
            .retain(|_, phone| phone.device != device.as_str());
    }

    #[must_use]
    pub fn phone_for(&self, key: &[u8]) -> Option<&PairedPhone> {
        self.phones.get(&digest_of(key).to_hex())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.phones.is_empty()
    }
}

/// Decides what to do with a connecting peer.
///
/// Pairing being open is a deliberate act by the person at the desktop, and it is the only
/// thing that lets an unknown key past.
#[must_use]
pub fn admit(paired: &Paired, pairing_open: bool, certificate: &[u8]) -> Admission {
    let Some(key) = public_key_of(certificate) else {
        return Admission::Refused;
    };

    if let Some(phone) = paired.phone_for(&key) {
        return Admission::Known(DeviceId::new(&phone.device));
    }
    if pairing_open {
        return Admission::Offered { key };
    }
    Admission::Refused
}

/// The `SubjectPublicKeyInfo` inside a certificate.
///
/// Chain semantics are ignored on purpose: these certificates are self-signed and vouch for
/// nothing. The key is the whole of what is being checked.
#[must_use]
pub fn public_key_of(certificate: &[u8]) -> Option<Vec<u8>> {
    let (_, parsed) = x509_parser::parse_x509_certificate(certificate).ok()?;
    Some(parsed.tbs_certificate.subject_pki.raw.to_vec())
}

#[must_use]
pub fn digest_of(key: &[u8]) -> Sha256 {
    let mut digest = RunningDigest::new();
    digest.update(key);
    digest.peek()
}

fn file(directory: &Path) -> PathBuf {
    directory.join(PAIRED_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> DeviceId {
        DeviceId::new("phone-a")
    }

    #[test]
    fn an_unknown_key_is_refused_when_nobody_opened_pairing() {
        let paired = Paired::new();

        assert_eq!(
            admit(&paired, false, b"not a certificate"),
            Admission::Refused
        );
    }

    #[test]
    fn a_certificate_that_will_not_parse_is_refused_even_while_pairing() {
        let paired = Paired::new();

        assert_eq!(
            admit(&paired, true, b"not a certificate"),
            Admission::Refused
        );
    }

    #[test]
    fn a_paired_phone_is_recognised_by_its_key_alone() {
        let mut paired = Paired::new();
        paired.pair(b"a key", &phone(), "Kitchen phone");

        assert_eq!(
            paired.phone_for(b"a key").map(|found| found.device.clone()),
            Some("phone-a".to_string())
        );
        assert!(paired.phone_for(b"another key").is_none());
    }

    #[test]
    fn unpairing_forgets_the_phone() {
        let mut paired = Paired::new();
        paired.pair(b"a key", &phone(), "Kitchen phone");

        paired.unpair(&phone());

        assert!(paired.phone_for(b"a key").is_none());
        assert!(paired.is_empty());
    }

    #[test]
    fn pairing_again_replaces_the_key_that_phone_had() {
        let mut paired = Paired::new();
        paired.pair(b"an old key", &phone(), "Kitchen phone");
        paired.unpair(&phone());
        paired.pair(b"a new key", &phone(), "Kitchen phone");

        assert!(paired.phone_for(b"an old key").is_none());
        assert!(paired.phone_for(b"a new key").is_some());
    }
}
