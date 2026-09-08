//! The desktop's identity, on disk.
//!
//! `SPEC.md` §5.2 says a changed desktop key triggers a visible re-pair rather than silent
//! trust, so the one thing this must never do is quietly generate a second identity.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::identity::Identity;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-identity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }
        Self { path }
    }

    fn identity(&self) -> Identity {
        match Identity::load_or_create(&self.path.join("photo-sync")) {
            Ok(identity) => identity,
            Err(error) => panic!("cannot open the identity: {error}"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn an_identity_is_generated_once_and_then_kept() {
    let scratch = Scratch::new();

    let first = scratch.identity();
    let second = scratch.identity();

    assert_eq!(first.public_key(), second.public_key());
    assert_eq!(first.certificate(), second.certificate());
    assert_eq!(first.private_key(), second.private_key());
}

#[test]
fn two_desktops_do_not_share_a_key() {
    let one = Scratch::new();
    let other = Scratch::new();

    assert_ne!(one.identity().public_key(), other.identity().public_key());
}

#[test]
fn an_identity_carries_a_public_key_and_a_certificate() {
    let scratch = Scratch::new();

    let identity = scratch.identity();

    assert!(!identity.public_key().is_empty());
    assert!(!identity.certificate().is_empty());
    assert!(!identity.private_key().is_empty());
    // The public key is the SubjectPublicKeyInfo rather than the whole certificate, which is
    // what lets a reissued certificate for the same key stay pinned.
    assert!(identity.public_key().len() < identity.certificate().len());
}

#[test]
fn nobody_but_the_owner_can_read_the_private_key() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new();
    scratch.identity();

    let key = scratch.path.join("photo-sync").join("key.der");
    let mode = match std::fs::metadata(&key) {
        Ok(facts) => facts.permissions().mode() & 0o777,
        Err(error) => panic!("cannot read the key's permissions: {error}"),
    };

    assert_eq!(mode, 0o600, "the private key is readable by others");
}
