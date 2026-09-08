//! Admitting a peer, against certificates that really exist.
//!
//! The load-bearing check here is that the desktop and the parser agree on what a public key
//! is. One side of a pairing reports its own `SubjectPublicKeyInfo`; the other reads it out
//! of a certificate. If those two ever disagree, every pin silently stops matching and every
//! phone is asked to pair again.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::identity::{Identity, pairing_code};
use photo_sync::pinning::{Admission, Paired, admit, public_key_of};
use photo_sync_core::id::DeviceId;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-pinning-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }
        Self { path }
    }

    fn identity(&self, name: &str) -> Identity {
        match Identity::load_or_create(&self.path.join(name)) {
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

fn key_in(certificate: &[u8]) -> Vec<u8> {
    match public_key_of(certificate) {
        Some(key) => key,
        None => panic!("the certificate did not yield a public key"),
    }
}

#[test]
fn the_key_read_out_of_a_certificate_is_the_key_its_holder_reports() {
    let scratch = Scratch::new();
    let identity = scratch.identity("desktop");

    assert_eq!(key_in(identity.certificate()), identity.public_key());
}

#[test]
fn a_phone_paired_once_is_admitted_without_pairing_being_open() {
    let scratch = Scratch::new();
    let phone = scratch.identity("phone");
    let mut paired = Paired::new();
    paired.pair(
        phone.public_key(),
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    );

    let admission = admit(&paired, false, phone.certificate());

    assert_eq!(admission, Admission::Known(DeviceId::new("phone-a")));
}

#[test]
fn a_phone_nobody_paired_is_refused() {
    let scratch = Scratch::new();
    let stranger = scratch.identity("stranger");

    let admission = admit(&Paired::new(), false, stranger.certificate());

    assert_eq!(admission, Admission::Refused);
}

#[test]
fn a_new_phone_is_offered_only_while_pairing_is_open() {
    let scratch = Scratch::new();
    let phone = scratch.identity("phone");

    let admission = admit(&Paired::new(), true, phone.certificate());

    assert_eq!(
        admission,
        Admission::Offered {
            key: phone.public_key().to_vec()
        }
    );
}

#[test]
fn a_phone_that_changed_its_key_is_not_admitted_on_the_old_pin() {
    let scratch = Scratch::new();
    let first = scratch.identity("phone");
    let reinstalled = scratch.identity("phone-after-reinstall");
    let mut paired = Paired::new();
    paired.pair(
        first.public_key(),
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    );

    let admission = admit(&paired, false, reinstalled.certificate());

    // SPEC.md §5.2: a changed key is a visible re-pair, never silent trust.
    assert_eq!(admission, Admission::Refused);
}

#[test]
fn paired_phones_survive_being_written_and_read_back() {
    let scratch = Scratch::new();
    let phone = scratch.identity("phone");
    let mut paired = Paired::new();
    paired.pair(
        phone.public_key(),
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    );
    if let Err(error) = paired.save(&scratch.path) {
        panic!("cannot save the paired phones: {error}");
    }

    let reloaded = match Paired::load(&scratch.path) {
        Ok(reloaded) => reloaded,
        Err(error) => panic!("cannot load the paired phones: {error}"),
    };

    assert_eq!(
        admit(&reloaded, false, phone.certificate()),
        Admission::Known(DeviceId::new("phone-a"))
    );
}

#[test]
fn a_desktop_that_has_paired_nobody_reads_as_empty() {
    let scratch = Scratch::new();

    let paired = match Paired::load(&scratch.path) {
        Ok(paired) => paired,
        Err(error) => panic!("cannot load the paired phones: {error}"),
    };

    assert!(paired.is_empty());
}

#[test]
fn both_ends_of_a_pairing_read_the_same_code_off_their_screens() {
    let scratch = Scratch::new();
    let desktop = scratch.identity("desktop");
    let phone = scratch.identity("phone");

    // The desktop knows its own key and reads the phone's out of the certificate it
    // presented; the phone does the mirror of that. Both must arrive at one code.
    let on_the_desktop = pairing_code(desktop.public_key(), &key_in(phone.certificate()));
    let on_the_phone = pairing_code(&key_in(desktop.certificate()), phone.public_key());

    assert_eq!(on_the_desktop, on_the_phone);
}

#[test]
fn a_third_key_in_the_middle_makes_the_two_screens_disagree() {
    let scratch = Scratch::new();
    let desktop = scratch.identity("desktop");
    let phone = scratch.identity("phone");
    let attacker = scratch.identity("attacker");

    // An attacker holding a separate connection with each side sees its own key on both.
    let on_the_desktop = pairing_code(desktop.public_key(), attacker.public_key());
    let on_the_phone = pairing_code(attacker.public_key(), phone.public_key());

    assert_ne!(on_the_desktop, on_the_phone);
}
