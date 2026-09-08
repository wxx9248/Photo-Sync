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

/// Pairs a phone with a desktop and runs one whole session between them.
fn one_session(scratch: &Scratch, phone: &mut Phone) -> SessionOutcome {
    let desktop_identity = scratch.identity("desktop");
    let phone_identity = scratch.identity("phone");

    let mut paired = Paired::new();
    paired.pair(
        phone_identity.public_key(),
        &phone.device.clone(),
        &phone.name.clone(),
    );

    let desktop = Desktop::start(scratch, &desktop_identity, paired);
    let mut connected = match Connected::dial(
        desktop.runtime.handle(),
        desktop.address,
        &phone_identity,
        desktop_identity.public_key(),
        &phone.device.clone(),
    ) {
        Ok(connected) => connected,
        Err(error) => panic!("cannot reach the desktop: {error}"),
    };

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
