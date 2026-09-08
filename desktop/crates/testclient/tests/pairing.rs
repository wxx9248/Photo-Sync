//! Meeting a desktop for the first time.
//!
//! `SPEC.md` §5.2 puts one person between an unknown phone and a trusted one. Both ends work
//! the code out from the two keys they hold, so nothing about it travels over the connection
//! it is protecting, and a person who can see both screens is the only thing that decides.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use photo_sync::desk::Desk;
use photo_sync::identity::Identity;
use photo_sync::pinning::Paired;
use photo_sync::serve::{Listening, listen_in};
use photo_sync::tls::PairingWindow;
use photo_sync_core::covers;
use photo_sync_core::id::DeviceId;
use photo_sync_sim::{Phone, PhoneFile};
use photo_sync_testclient::{Connected, pair};
use tokio::runtime::Runtime;
use tokio::sync::Mutex as AsyncMutex;

struct Desktop {
    path: PathBuf,
    runtime: Runtime,
    listening: Option<Listening>,
    address: SocketAddr,
    paired: Arc<Mutex<Paired>>,
    window: PairingWindow,
}

impl Desktop {
    fn start() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "photo-sync-pairing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }

        let runtime = match Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => panic!("cannot start a runtime: {error}"),
        };
        let identity = match Identity::load_or_create(&path.join("desktop")) {
            Ok(identity) => identity,
            Err(error) => panic!("cannot make an identity: {error}"),
        };
        let desk = match Desk::open(&path.join("Camera"), &path.join("data/index.db")) {
            Ok(desk) => desk,
            Err(error) => panic!("cannot open the desktop: {error}"),
        };

        let paired = Arc::new(Mutex::new(Paired::new()));
        let window = PairingWindow::closed();
        let listening = runtime.block_on(listen_in(
            "127.0.0.1:0".parse().unwrap_or_else(|_| unreachable!()),
            Arc::new(identity.clone()),
            Arc::clone(&paired),
            window.clone(),
            Arc::new(AsyncMutex::new(desk)),
            "Test desktop",
            &path,
        ));
        let listening = match listening {
            Ok(listening) => listening,
            Err(error) => panic!("cannot listen: {error}"),
        };

        let address = listening.address;
        Self {
            path,
            runtime,
            listening: Some(listening),
            address,
            paired,
            window,
        }
    }

    fn phone_identity(&self, who: &str) -> Identity {
        match Identity::load_or_create(&self.path.join(who)) {
            Ok(identity) => identity,
            Err(error) => panic!("cannot make an identity: {error}"),
        }
    }

    /// Waits for a code to appear on the desktop's screen.
    fn showing(&self) -> Option<String> {
        for _ in 0..200 {
            if let Some(code) = self.window.showing() {
                return Some(code);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    fn knows_anyone(&self) -> bool {
        self.paired
            .lock()
            .map(|paired| !paired.is_empty())
            .unwrap_or(false)
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(listening) = self.listening.take() {
            listening.stop();
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn a_first_connection_puts_the_same_code_on_both_screens() {
    covers!("R-PAIR-001");
    let desktop = Desktop::start();
    let phone = desktop.phone_identity("phone");
    desktop.window.open();

    let pairing = match pair(
        desktop.runtime.handle(),
        desktop.address,
        &phone,
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    ) {
        Ok(pairing) => pairing,
        Err(error) => panic!("cannot start pairing: {error}"),
    };

    let on_the_desktop = match desktop.showing() {
        Some(code) => code,
        None => panic!("the desktop never showed a code"),
    };
    assert_eq!(on_the_desktop, pairing.code);
    assert_eq!(on_the_desktop.len(), 6);

    desktop.window.confirm();
    assert!(pairing.settled());
    assert!(desktop.knows_anyone());
}

#[test]
fn a_person_who_says_the_codes_differ_leaves_the_phone_unpaired() {
    covers!("R-PAIR-001");
    let desktop = Desktop::start();
    let phone = desktop.phone_identity("phone");
    desktop.window.open();

    let pairing = match pair(
        desktop.runtime.handle(),
        desktop.address,
        &phone,
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    ) {
        Ok(pairing) => pairing,
        Err(error) => panic!("cannot start pairing: {error}"),
    };
    assert!(desktop.showing().is_some());

    desktop.window.reject();

    assert!(!pairing.settled());
    assert!(!desktop.knows_anyone());
}

#[test]
fn a_desktop_nobody_opened_will_not_meet_a_phone() {
    let desktop = Desktop::start();
    let phone = desktop.phone_identity("phone");

    // The window is closed, so the connection itself is refused before any code exists.
    let attempted = pair(
        desktop.runtime.handle(),
        desktop.address,
        &phone,
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    );

    if let Ok(pairing) = attempted {
        assert!(!pairing.settled());
    }
    assert!(!desktop.knows_anyone());
    assert!(desktop.window.showing().is_none());
}

#[test]
fn a_phone_can_run_a_session_the_moment_it_is_paired() {
    let desktop = Desktop::start();
    let phone_identity = desktop.phone_identity("phone");
    desktop.window.open();
    let pairing = match pair(
        desktop.runtime.handle(),
        desktop.address,
        &phone_identity,
        &DeviceId::new("phone-a"),
        "Kitchen phone",
    ) {
        Ok(pairing) => pairing,
        Err(error) => panic!("cannot start pairing: {error}"),
    };
    assert!(desktop.showing().is_some());
    desktop.window.confirm();
    let desktop_key = pairing.desktop_key.clone();
    assert!(pairing.settled());

    let mut connected = match Connected::dial(
        desktop.runtime.handle(),
        desktop.address,
        &phone_identity,
        &desktop_key,
        &DeviceId::new("phone-a"),
    ) {
        Ok(connected) => connected,
        Err(error) => panic!("cannot reach the desktop: {error}"),
    };
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(1_756_000_000, b"one photograph".to_vec()),
    );

    let outcome = phone.run_session(&mut connected);

    assert_eq!(outcome.uploaded.len(), 1);
    assert_eq!(outcome.deleted.len(), 1);
}
