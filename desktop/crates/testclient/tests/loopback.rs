//! One session over a real socket, with real TLS, real files and real SQLite.
//!
//! This is the layer `docs/VERIFICATION.md` §L4 calls real-bytes end-to-end. The phone here
//! makes the same decisions it makes in the simulator and differs only in how they travel, so
//! anything that behaves differently is the transport's doing.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use photo_sync::desk::Desk;
use photo_sync::identity::Identity;
use photo_sync::pinning::Paired;
use photo_sync::serve::{Listening, listen};
use photo_sync::tls::PairingWindow;
use photo_sync_core::covers;
use photo_sync_core::id::DeviceId;
use photo_sync_sim::{Phone, PhoneFile, SessionOutcome};
use photo_sync_testclient::Connected;
use tokio::runtime::Runtime;
use tokio::sync::Mutex as AsyncMutex;

const MTIME: i64 = 1_756_000_000;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-loopback-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }
        Self { path }
    }

    fn vault(&self) -> PathBuf {
        self.path.join("Camera")
    }

    fn identity(&self, who: &str) -> Identity {
        match Identity::load_or_create(&self.path.join(who)) {
            Ok(identity) => identity,
            Err(error) => panic!("cannot open the identity for {who}: {error}"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A desktop listening on a port the machine chose.
struct Desktop {
    runtime: Runtime,
    listening: Option<Listening>,
    address: SocketAddr,
}

impl Desktop {
    fn start(scratch: &Scratch, identity: &Identity, paired: Paired) -> Self {
        let runtime = match Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => panic!("cannot start a runtime: {error}"),
        };

        let desk = match Desk::open(&scratch.vault(), &scratch.path.join("data/index.db")) {
            Ok(desk) => desk,
            Err(error) => panic!("cannot open the desktop: {error}"),
        };

        let held = Arc::new(AsyncMutex::new(desk));
        let identity = Arc::new(identity.clone());
        let listening = runtime.block_on(listen(
            "127.0.0.1:0".parse().unwrap_or_else(|_| unreachable!()),
            identity,
            Arc::new(Mutex::new(paired)),
            PairingWindow::closed(),
            held,
            "Test desktop",
        ));
        let listening = match listening {
            Ok(listening) => listening,
            Err(error) => panic!("cannot listen: {error}"),
        };

        let address = listening.address;
        Self {
            runtime,
            listening: Some(listening),
            address,
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(listening) = self.listening.take() {
            listening.stop();
        }
    }
}

fn vault_files(scratch: &Scratch) -> Vec<String> {
    let entries = match std::fs::read_dir(scratch.vault()) {
        Ok(entries) => entries,
        Err(error) => panic!("cannot read the vault: {error}"),
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().to_str().map(ToString::to_string))
        .collect();
    names.sort();
    names
}

/// A desktop a phone is already paired with, kept running so it can be dialled more than
/// once. A reconnection is an ordinary part of `SPEC.md` §6, so it has to be arrangeable.
struct Known {
    desktop: Desktop,
    desktop_identity: Identity,
    phone_identity: Identity,
}

fn known(scratch: &Scratch, phone: &Phone) -> Known {
    let desktop_identity = scratch.identity("desktop");
    let phone_identity = scratch.identity("phone");

    let mut paired = Paired::new();
    paired.pair(phone_identity.public_key(), &phone.device, &phone.name);

    Known {
        desktop: Desktop::start(scratch, &desktop_identity, paired),
        desktop_identity,
        phone_identity,
    }
}

fn dial(known: &Known, phone: &Phone) -> Connected {
    match Connected::dial(
        known.desktop.runtime.handle(),
        known.desktop.address,
        &known.phone_identity,
        known.desktop_identity.public_key(),
        &phone.device,
    ) {
        Ok(connected) => connected,
        Err(error) => panic!("cannot reach the desktop: {error}"),
    }
}

/// Pairs a phone with a desktop and runs one whole session between them.
fn one_session(scratch: &Scratch, phone: &mut Phone) -> SessionOutcome {
    let known = known(scratch, phone);
    let mut connected = dial(&known, phone);
    phone.run_session(&mut connected)
}

#[test]
fn one_photo_makes_the_round_trip_over_a_socket() {
    let scratch = Scratch::new();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let outcome = one_session(&scratch, &mut phone);

    let names = vault_files(&scratch);
    assert_eq!(names.len(), 1, "the vault holds {names:?}");
    assert_eq!(outcome.uploaded.len(), 1);
    assert_eq!(outcome.deleted.len(), 1);
    assert!(phone.files.is_empty());

    let summary = match outcome.summary {
        Some(summary) => summary,
        None => panic!("the session never closed"),
    };
    assert_eq!(summary.sent, 1);
    assert_eq!(summary.deleted, 1);
    assert_eq!(summary.bytes_freed, 14);
}

#[test]
fn a_phone_that_is_not_paired_cannot_reach_the_desktop() {
    let scratch = Scratch::new();
    let desktop_identity = scratch.identity("desktop");
    let stranger = scratch.identity("stranger");
    let desktop = Desktop::start(&scratch, &desktop_identity, Paired::new());

    let dialled = Connected::dial(
        desktop.runtime.handle(),
        desktop.address,
        &stranger,
        desktop_identity.public_key(),
        &DeviceId::new("phone-a"),
    );

    // Whether the refusal lands while connecting or on the first call, nothing it sends can
    // reach the desktop's state.
    if let Ok(mut connected) = dialled {
        let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
            "DCIM/Camera/IMG_0001.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        );
        let outcome = phone.run_session(&mut connected);
        assert!(outcome.deleted.is_empty());
    }
    assert!(vault_files(&scratch).is_empty());
}

#[test]
fn one_photograph_at_two_paths_crosses_once_and_frees_both() {
    let scratch = Scratch::new();
    let mut phone = Phone::new("phone-a", "Kitchen phone")
        .holding(
            "DCIM/Camera/IMG_0001.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        )
        .holding(
            "DCIM/Camera/IMG_0002.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        );

    let outcome = one_session(&scratch, &mut phone);

    assert_eq!(vault_files(&scratch).len(), 1);
    assert_eq!(outcome.deleted.len(), 2);
    assert!(phone.files.is_empty());
}

#[test]
fn a_transfer_cut_short_carries_on_from_the_watermark_over_a_socket() {
    covers!("R-STAGE-008", "R-XFER-006");
    let whole = b"a photograph long enough to be worth carrying on with".to_vec();
    let scratch = Scratch::new();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, whole.clone()),
    );

    let known = known(&scratch, &phone);

    // The first connection gets twenty bytes across and then dies with no digest behind it.
    // Nothing is in the vault, and nothing could be: the file is not whole.
    {
        let mut connected = dial(&known, &phone);
        phone.send_partly(&mut connected, 20);
    }
    assert!(
        vault_files(&scratch).is_empty(),
        "an unfinished file reached the vault"
    );

    // Those twenty bytes are on the desktop's own disk, in a partial named by the manifest.
    // A desktop that had gathered the stream up in memory would have nothing here at all.
    let staging = scratch.vault().join(".staging").join("phone-a");
    let partials: Vec<PathBuf> = match std::fs::read_dir(&staging) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|suffix| suffix == "part"))
            .collect(),
        Err(error) => panic!("nothing was staged: {error}"),
    };
    assert_eq!(partials.len(), 1, "staging holds {partials:?}");
    let kept = match std::fs::read(&partials[0]) {
        Ok(bytes) => bytes,
        Err(error) => panic!("cannot read the partial: {error}"),
    };
    assert_eq!(
        kept,
        whole[..20],
        "the partial is not the start of the photograph"
    );

    // The phone comes back with the same frozen catalog. The desktop had to have written
    // those bytes to its own disk as they arrived to be able to ask for the rest of them.
    let mut connected = dial(&known, &phone);
    let outcome = phone.run_session(&mut connected);

    assert_eq!(outcome.uploaded.len(), 1);
    let names = vault_files(&scratch);
    assert_eq!(names.len(), 1, "the vault holds {names:?}");
    let stored = match std::fs::read(scratch.vault().join(&names[0])) {
        Ok(bytes) => bytes,
        Err(error) => panic!("cannot read the vault copy: {error}"),
    };
    assert_eq!(stored, whole, "the resumed file is not the photograph");
}

#[test]
fn a_phone_that_misnames_itself_is_still_known_by_the_key_it_holds() {
    covers!("R-SESSION-005");
    let scratch = Scratch::new();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let known = known(&scratch, &phone);

    // A first, honest session. The photograph is imported and the phone keeps it, so the
    // catalog of the second session is the same.
    {
        let mut connected = dial(&known, &phone);
        let outcome = phone.run_session(&mut connected);
        assert_eq!(outcome.uploaded.len(), 1);
    }
    let mut kept = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    // The same key, now claiming to be a phone the desktop has never met. If the desktop
    // took the device from what it was told, this session would look like a stranger's and
    // the photograph would be asked for all over again.
    let mut connected = dial(&known, &kept);
    connected.claimed_device = Some("phone-b".to_string());
    let outcome = kept.run_session(&mut connected);

    assert!(
        outcome.uploaded.is_empty(),
        "the desktop believed a device name it was handed"
    );
    assert_eq!(vault_files(&scratch).len(), 1, "a second copy was stored");
}

#[test]
fn a_phone_speaking_another_version_is_turned_away() {
    covers!("R-PAIR-004");
    let scratch = Scratch::new();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let known = known(&scratch, &phone);
    let mut connected = dial(&known, &phone);
    // STACK.md §5.4: an unknown major version is refused in plain language rather than
    // negotiated, and the version travels in the handshake as well as the TXT record.
    connected.claimed_version = Some(photo_sync_protocol::PROTOCOL_VERSION + 1);

    let outcome = phone.run_session(&mut connected);

    assert!(
        outcome.rejected.is_some(),
        "a version nobody speaks was accepted"
    );
    assert!(outcome.uploaded.is_empty());
    assert!(vault_files(&scratch).is_empty());
    assert_eq!(
        phone.files.len(),
        1,
        "the phone gave up a photograph anyway"
    );
}

#[test]
fn a_desktop_key_that_changed_is_never_trusted_silently() {
    covers!("R-PAIR-003");
    let scratch = Scratch::new();
    let phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let known = known(&scratch, &phone);

    // First that the desktop is reachable at all, so what follows is about the key and not
    // about the socket.
    assert!(
        Connected::dial(
            known.desktop.runtime.handle(),
            known.desktop.address,
            &known.phone_identity,
            known.desktop_identity.public_key(),
            &phone.device,
        )
        .is_ok(),
        "the desktop the phone remembers cannot be reached"
    );

    // Now the desktop answering is not the one the phone remembers: a reinstall, or somebody
    // standing in the middle. §5.2 says this is never waved through, and the refusal comes
    // before a single byte of the protocol — the connection itself does not open.
    let stranger = scratch.identity("another-desktop");
    let refused = Connected::dial(
        known.desktop.runtime.handle(),
        known.desktop.address,
        &known.phone_identity,
        stranger.public_key(),
        &phone.device,
    );

    assert!(
        refused.is_err(),
        "a phone opened a connection to a desktop whose key it did not recognise"
    );
    assert!(vault_files(&scratch).is_empty());
}
