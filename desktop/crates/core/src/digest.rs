//! The digest of a file as its bytes stream past.
//!
//! The desktop hashes what arrives on the wire rather than what it later reads back from disk.
//! `SPEC.md` §6.5 uses the digest to prove the transfer was faithful, and §7.6 uses watermarks
//! to prove the bytes reached durable storage. Keeping the two separate is what lets a torn
//! tail after a power loss be caught by the watermark instead of hiding behind a digest that
//! was computed over the same damaged bytes.

use sha2::{Digest as _, Sha256 as Hasher};

use crate::id::Sha256;

/// Accumulates a SHA-256 over the bytes handed to it, in order.
#[derive(Clone)]
pub struct RunningDigest {
    hasher: Hasher,
    bytes: u64,
}

impl RunningDigest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: Hasher::new(),
            bytes: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
        self.bytes += data.len() as u64;
    }

    /// How many bytes have been hashed. A resumed transfer may only continue from a digest
    /// that covers exactly the prefix the desktop holds durably.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The digest of everything hashed so far. Reading it does not end the accumulation, so a
    /// partial file can be compared and then continued.
    #[must_use]
    pub fn peek(&self) -> Sha256 {
        Sha256(self.hasher.clone().finalize().into())
    }
}

impl Default for RunningDigest {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RunningDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunningDigest({} bytes)", self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published digest of the empty input, which anchors the implementation to the
    /// standard rather than to itself.
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn an_empty_input_matches_the_published_digest() {
        assert_eq!(RunningDigest::new().peek().to_hex(), EMPTY);
    }

    #[test]
    fn chunk_boundaries_do_not_change_the_digest() {
        let mut whole = RunningDigest::new();
        whole.update(b"abcdef");

        let mut split = RunningDigest::new();
        split.update(b"ab");
        split.update(b"cdef");

        assert_eq!(whole.peek(), split.peek());
    }

    #[test]
    fn peeking_leaves_the_accumulation_open() {
        let mut running = RunningDigest::new();
        running.update(b"abc");
        let peeked = running.peek();
        running.update(b"def");

        assert_ne!(running.peek(), peeked);
        assert_eq!(running.bytes(), 6);
    }
}
