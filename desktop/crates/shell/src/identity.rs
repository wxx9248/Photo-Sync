//! The desktop's long-lived key, and the code that pairs it with a phone.
//!
//! `SPEC.md` §5.2 pins keys on both sides rather than trusting a certificate authority, so
//! this identity is generated once and then never changes: a new one is a new desktop, and
//! the phone is meant to notice.
//!
//! The pairing code is a short digest over *both* public keys. Binding both is what makes an
//! active man-in-the-middle visible: it necessarily holds different keys with each side, so
//! the two screens show different codes rather than agreeing on a lie.

use std::path::{Path, PathBuf};

use photo_sync_core::RunningDigest;
use rcgen::{CertifiedKey, KeyPair, PublicKeyData};

/// Ties the digest to this protocol, so a hash taken for some other purpose can never be
/// mistaken for a pairing code.
const DOMAIN: &[u8] = b"photo-sync pairing v1";

/// How many digits the person at the desktop reads out. Six is the familiar length, and the
/// code is worth guessing only during the seconds pairing is open.
const DIGITS: u32 = 6;

const CERTIFICATE_FILE: &str = "certificate.der";
const KEY_FILE: &str = "key.der";

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("cannot read or write the identity: {0}")]
    Storage(String),

    #[error("cannot create a key: {0}")]
    Generation(String),
}

/// One desktop's certificate and the key behind it.
pub struct Identity {
    certificate: Vec<u8>,
    private_key: Vec<u8>,
    public_key: Vec<u8>,
}

impl Identity {
    /// Loads the identity from the data directory, generating one the first time.
    pub fn load_or_create(directory: &Path) -> Result<Self, IdentityError> {
        std::fs::create_dir_all(directory).map_err(storage)?;
        let certificate_file = directory.join(CERTIFICATE_FILE);
        let key_file = directory.join(KEY_FILE);

        if certificate_file.is_file() && key_file.is_file() {
            let certificate = std::fs::read(&certificate_file).map_err(storage)?;
            let private_key = std::fs::read(&key_file).map_err(storage)?;
            let pair = KeyPair::try_from(private_key.as_slice())
                .map_err(|error| IdentityError::Generation(error.to_string()))?;
            return Ok(Self {
                certificate,
                public_key: pair.subject_public_key_info(),
                private_key,
            });
        }

        let identity = Self::generate()?;
        std::fs::write(&certificate_file, &identity.certificate).map_err(storage)?;
        write_privately(&key_file, &identity.private_key)?;
        Ok(identity)
    }

    fn generate() -> Result<Self, IdentityError> {
        let CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["photo-sync".to_string()])
                .map_err(|error| IdentityError::Generation(error.to_string()))?;

        Ok(Self {
            certificate: cert.der().to_vec(),
            public_key: signing_key.subject_public_key_info(),
            private_key: signing_key.serialize_der(),
        })
    }

    /// The certificate this desktop presents, as DER.
    #[must_use]
    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }

    /// The private key, as PKCS#8 DER.
    #[must_use]
    pub fn private_key(&self) -> &[u8] {
        &self.private_key
    }

    /// The `SubjectPublicKeyInfo`, which is what both ends pin and what the pairing code is
    /// built from. It survives a certificate being reissued for the same key.
    #[must_use]
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }
}

/// The code shown on the desktop and confirmed on the phone.
///
/// Both ends compute it from the same two keys, so nothing about it travels over the
/// connection it is protecting. The lengths are part of the digest, so no pair of keys can be
/// rearranged into another pair with the same code.
#[must_use]
pub fn pairing_code(desktop_key: &[u8], phone_key: &[u8]) -> String {
    let mut digest = RunningDigest::new();
    digest.update(DOMAIN);
    for key in [desktop_key, phone_key] {
        digest.update(&(key.len() as u32).to_be_bytes());
        digest.update(key);
    }

    let bytes = digest.peek().0;
    let leading = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let modulus = 10_u32.pow(DIGITS);
    format!(
        "{:0width$}",
        leading % modulus,
        width = usize::try_from(DIGITS).unwrap_or(6)
    )
}

/// Writes a file only its owner can read. A pinned identity is only worth as much as the key
/// behind it.
fn write_privately(path: &Path, bytes: &[u8]) -> Result<(), IdentityError> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(storage)?;
    std::io::Write::write_all(&mut file, bytes).map_err(storage)?;
    std::io::Write::flush(&mut file).map_err(storage)
}

/// Where the identity lives, following the XDG base directory specification.
#[must_use]
pub fn data_directory() -> PathBuf {
    if let Ok(home) = std::env::var("XDG_DATA_HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home).join("photo-sync");
    }
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".local/share/photo-sync"),
        Err(_) => PathBuf::from(".photo-sync"),
    }
}

fn storage(error: std::io::Error) -> IdentityError {
    IdentityError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_six_digits() {
        let code = pairing_code(b"desktop key", b"phone key");

        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|digit| digit.is_ascii_digit()));
    }

    #[test]
    fn the_same_two_keys_always_give_the_same_code() {
        assert_eq!(
            pairing_code(b"desktop key", b"phone key"),
            pairing_code(b"desktop key", b"phone key")
        );
    }

    #[test]
    fn a_different_phone_gives_a_different_code() {
        assert_ne!(
            pairing_code(b"desktop key", b"phone key"),
            pairing_code(b"desktop key", b"another phone")
        );
    }

    #[test]
    fn a_different_desktop_gives_a_different_code() {
        assert_ne!(
            pairing_code(b"desktop key", b"phone key"),
            pairing_code(b"another desktop", b"phone key")
        );
    }

    #[test]
    fn the_two_keys_cannot_be_swapped_without_changing_the_code() {
        assert_ne!(pairing_code(b"one", b"two"), pairing_code(b"two", b"one"));
    }

    /// Without the lengths in the digest, "ab" + "c" and "a" + "bc" would hash the same.
    #[test]
    fn keys_that_run_together_are_still_told_apart() {
        assert_ne!(pairing_code(b"ab", b"c"), pairing_code(b"a", b"bc"));
    }

    /// Anchors the derivation against a value worked out from `STACK.md` §4.6 by hand rather
    /// than read back from this code. The phone has to arrive at the same six digits from the
    /// same two keys, so a change here has to fail here rather than in front of a parent.
    #[test]
    fn a_code_is_fixed_by_the_keys_and_nothing_else() {
        assert_eq!(pairing_code(b"desktop key", b"phone key"), "793352");
    }
}
