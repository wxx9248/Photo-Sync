//! Identifiers and small values used across the core.

use std::fmt;

/// Defines a newtype around a string so two identifiers cannot be swapped at a call site.
macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(DeviceId, "Identity of a phone, taken from ANDROID_ID.");
string_id!(
    DevicePath,
    "Path of a file on the phone: RELATIVE_PATH joined with DISPLAY_NAME."
);
string_id!(
    VaultName,
    "File name inside the vault, assigned at commit time."
);

/// Staging file assigned by the manifest. A re-transfer of the same path receives a new
/// identifier so it can never be confused with an earlier attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u64);

/// Correlates an effect with the event reporting its outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpId(pub u64);

/// Seconds since the Unix epoch. MediaStore reports whole seconds, so both ends use the same
/// resolution and file identity never differs through rounding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

/// A SHA-256 digest of file content.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({})", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_renders_as_lowercase_hex() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        bytes[31] = 0x0f;

        let hex = Sha256(bytes).to_hex();

        assert!(hex.starts_with("ab"));
        assert!(hex.ends_with("0f"));
        assert_eq!(hex.len(), 64);
    }
}
