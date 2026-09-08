//! One session against real files and real SQLite.
//!
//! `docs/VERIFICATION.md` §L4 keeps this layer deliberately small: it verifies the adapters
//! the simulator replaces, not the logic. The same phone that drives the simulator drives the
//! real desktop here, so a difference between the two is a difference in the adapters.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::desk::Desk;
use photo_sync_core::effect::Effect;
use photo_sync_core::event::Event;
use photo_sync_sim::{Driver, Phone, PhoneFile};

const MTIME: i64 = 1_756_000_000;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-real-{}-{}",
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

    fn open(&self) -> Real {
        match Desk::open(&self.vault(), &self.path.join("data/index.db")) {
            Ok(mut desk) => {
                desk.take_log();
                Real { desk }
            }
            Err(error) => panic!("cannot open the desktop: {error}"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The real desktop, wearing the interface a phone talks to.
struct Real {
    desk: Desk,
}

impl Driver for Real {
    fn deliver(&mut self, event: Event) {
        self.desk.deliver(event);
    }

    fn take_log(&mut self) -> Vec<Effect> {
        self.desk.take_log()
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

fn contents(scratch: &Scratch, name: &str) -> Vec<u8> {
    match std::fs::read(scratch.vault().join(name)) {
        Ok(bytes) => bytes,
        Err(error) => panic!("cannot read {name}: {error}"),
    }
}

#[test]
fn one_photo_reaches_the_vault_with_the_bytes_it_was_sent() {
    let scratch = Scratch::new();
    let mut real = scratch.open();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let outcome = phone.run_session(&mut real);

    let names = vault_files(&scratch);
    assert_eq!(names.len(), 1);
    assert_eq!(contents(&scratch, &names[0]), b"one photograph".to_vec());
    assert_eq!(outcome.deleted.len(), 1);
    assert!(phone.files.is_empty());
}

#[test]
fn staging_is_empty_once_the_commit_has_finished() {
    let scratch = Scratch::new();
    let mut real = scratch.open();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    phone.run_session(&mut real);

    let staging = scratch.vault().join(".staging").join("phone-a");
    let left: Vec<PathBuf> = match std::fs::read_dir(&staging) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_none_or(|kind| kind != "db"))
            .collect(),
        Err(error) => panic!("cannot read staging: {error}"),
    };
    assert!(left.is_empty(), "staging still holds {left:?}");
}

#[test]
fn one_photograph_at_two_paths_reaches_the_vault_once() {
    let scratch = Scratch::new();
    let mut real = scratch.open();
    let mut phone = Phone::new("phone-a", "Kitchen phone")
        .holding(
            "DCIM/Camera/IMG_0001.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        )
        .holding(
            "DCIM/Camera/IMG_0002.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        );

    let outcome = phone.run_session(&mut real);

    assert_eq!(vault_files(&scratch).len(), 1);
    assert_eq!(outcome.deleted.len(), 2);
}

#[test]
fn a_photo_already_imported_is_not_sent_again_but_is_still_freed() {
    let scratch = Scratch::new();
    let file = PhoneFile::new(MTIME, b"one photograph".to_vec());

    {
        let mut real = scratch.open();
        let mut phone = Phone::new("phone-a", "Kitchen phone")
            .holding("DCIM/Camera/IMG_0001.jpg", file.clone());
        phone.run_session(&mut real);
    }

    // A new run, a new desktop, the same photo back on the phone.
    let mut restarted = scratch.open();
    let mut phone =
        Phone::new("phone-a", "Kitchen phone").holding("DCIM/Camera/IMG_0001.jpg", file);
    let outcome = phone.run_session(&mut restarted);

    assert_eq!(vault_files(&scratch).len(), 1);
    assert!(outcome.uploaded.is_empty());
    assert_eq!(outcome.deleted.len(), 1);
    let summary = match outcome.summary {
        Some(summary) => summary,
        None => panic!("the session never closed"),
    };
    assert_eq!(summary.sent, 0);
    assert_eq!(summary.deleted, 1);
}

#[test]
fn a_photo_whose_vault_copy_was_curated_away_stays_on_the_phone() {
    let scratch = Scratch::new();
    let file = PhoneFile::new(MTIME, b"one photograph".to_vec());

    {
        let mut real = scratch.open();
        let mut phone = Phone::new("phone-a", "Kitchen phone")
            .holding("DCIM/Camera/IMG_0001.jpg", file.clone());
        phone.run_session(&mut real);
    }
    for name in vault_files(&scratch) {
        if let Err(error) = std::fs::remove_file(scratch.vault().join(&name)) {
            panic!("cannot curate {name}: {error}");
        }
    }

    let mut restarted = scratch.open();
    let mut phone =
        Phone::new("phone-a", "Kitchen phone").holding("DCIM/Camera/IMG_0001.jpg", file);
    let outcome = phone.run_session(&mut restarted);

    assert!(outcome.offered.is_empty());
    assert_eq!(phone.files.len(), 1);
}
