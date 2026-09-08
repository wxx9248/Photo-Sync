//! Whole sessions, driven by a phone rather than by hand-written events.
//!
//! These are the closest thing the project has to an acceptance test today: the client plays
//! `SPEC.md` §6 from the handshake to the summary and runs §8's own gates before deleting
//! anything, so a photo only disappears from the phone when both halves agree it is safe.

use photo_sync_core::covers;
use photo_sync_core::id::{DevicePath, Timestamp, VaultName};
use photo_sync_core::store::DeviceFileRow;
use photo_sync_core::{CivilTime, Moment, RunningDigest, Sha256};
use photo_sync_sim::{Phone, PhoneFile, Simulation};

const MTIME: i64 = 1_756_000_000;

const IMPORTED_AT: Moment = Moment {
    at: Timestamp(1_757_000_000),
    local: CivilTime {
        year: 2026,
        month: 9,
        day: 7,
        hour: 23,
        minute: 0,
        second: 0,
    },
};

const CAPTURED: CivilTime = CivilTime {
    year: 2026,
    month: 8,
    day: 24,
    hour: 9,
    minute: 30,
    second: 0,
};

fn desktop() -> Simulation {
    let mut sim = Simulation::new(IMPORTED_AT);
    sim.storage.set_default_name_sources(vec![CAPTURED]);
    sim.start();
    sim.take_log();
    sim
}

fn digest_of(content: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(content);
    running.peek()
}

fn path(name: &str) -> DevicePath {
    DevicePath::new(format!("DCIM/Camera/{name}"))
}

#[test]
fn one_photo_makes_the_whole_round_trip() {
    covers!("R-XFER-001", "R-COMMIT-004", "R-DELETE-002");
    let mut sim = desktop();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let outcome = phone.run_session(&mut sim);

    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(
        sim.storage
            .vault()
            .get(&VaultName::new("2026-08-24_093000.jpg")),
        Some(&b"one photograph".to_vec())
    );
    assert_eq!(outcome.deleted, vec![path("IMG_0001.jpg")]);
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
fn a_phone_with_nothing_new_still_finishes() {
    covers!("R-SESSION-002");
    let mut sim = desktop();
    let mut phone = Phone::new("phone-a", "Kitchen phone");

    let outcome = phone.run_session(&mut sim);

    assert!(outcome.offered.is_empty());
    assert!(outcome.summary.is_some());
    assert!(sim.storage.vault().is_empty());
}

#[test]
fn the_same_photo_at_two_paths_is_stored_once_and_both_are_freed() {
    covers!("R-COMMIT-005", "R-STAGE-004");
    let mut sim = desktop();
    let mut phone = Phone::new("phone-a", "Kitchen phone")
        .holding(
            "DCIM/Camera/IMG_0001.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        )
        .holding(
            "DCIM/Camera/IMG_0002.jpg",
            PhoneFile::new(MTIME, b"one photograph".to_vec()),
        );

    let outcome = phone.run_session(&mut sim);

    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(outcome.deleted.len(), 2);
    assert_eq!(sim.store.device_files().len(), 2);
    assert!(phone.files.is_empty());
}

#[test]
fn a_photo_from_an_earlier_session_is_freed_after_the_phone_rehashes_it() {
    covers!("R-DELETE-007", "R-INDEX-003");
    let content = b"an older photograph".to_vec();
    let mut sim = desktop();
    sim.storage
        .put_in_vault(&VaultName::new("2026-08-01_120000.jpg"), content.clone());
    sim.store.remember_import(DeviceFileRow {
        device: photo_sync_core::DeviceId::new("phone-a"),
        path: path("IMG_0001.jpg"),
        size: content.len() as u64,
        mtime: Timestamp(MTIME),
        digest: digest_of(&content),
        vault_name: VaultName::new("2026-08-01_120000.jpg"),
        committed_at: Timestamp(MTIME),
    });
    let mut phone = Phone::new("phone-a", "Kitchen phone")
        .holding("DCIM/Camera/IMG_0001.jpg", PhoneFile::new(MTIME, content));

    let outcome = phone.run_session(&mut sim);

    // It was never sent, because the index already covered it.
    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(outcome.deleted, vec![path("IMG_0001.jpg")]);
    let summary = match outcome.summary {
        Some(summary) => summary,
        None => panic!("the session never closed"),
    };
    assert_eq!(summary.sent, 0);
    assert_eq!(summary.deleted, 1);
}

#[test]
fn a_photo_whose_vault_copy_was_curated_away_stays_on_the_phone() {
    covers!("R-DELETE-002");
    let content = b"an older photograph".to_vec();
    let mut sim = desktop();
    sim.store.remember_import(DeviceFileRow {
        device: photo_sync_core::DeviceId::new("phone-a"),
        path: path("IMG_0001.jpg"),
        size: content.len() as u64,
        mtime: Timestamp(MTIME),
        digest: digest_of(&content),
        vault_name: VaultName::new("2026-08-01_120000.jpg"),
        committed_at: Timestamp(MTIME),
    });
    // The user deleted the vault copy after it was imported.
    let mut phone = Phone::new("phone-a", "Kitchen phone")
        .holding("DCIM/Camera/IMG_0001.jpg", PhoneFile::new(MTIME, content));

    let outcome = phone.run_session(&mut sim);

    assert!(outcome.offered.is_empty());
    assert!(outcome.deleted.is_empty());
    assert_eq!(phone.files.len(), 1);
}

#[test]
fn a_photo_replaced_on_the_phone_is_imported_again_as_itself() {
    covers!("R-INDEX-002");
    let mut sim = desktop();
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"the original".to_vec()),
    );
    phone.run_session(&mut sim);

    // The user edited the photo, so the phone holds different bytes at the same path.
    phone.files.insert(
        path("IMG_0001.jpg"),
        PhoneFile::new(MTIME + 60, b"the edited version".to_vec()),
    );
    let outcome = phone.run_session(&mut sim);

    assert_eq!(sim.storage.vault().len(), 2);
    assert_eq!(sim.store.device_files().len(), 1);
    let summary = match outcome.summary {
        Some(summary) => summary,
        None => panic!("the session never closed"),
    };
    assert_eq!(summary.sent, 1);
}

#[test]
fn a_session_that_does_not_fit_is_turned_away_before_anything_moves() {
    covers!("R-DIFF-005");
    let mut sim = desktop();
    sim.free_space = 3;
    let mut phone = Phone::new("phone-a", "Kitchen phone").holding(
        "DCIM/Camera/IMG_0001.jpg",
        PhoneFile::new(MTIME, b"one photograph".to_vec()),
    );

    let outcome = phone.run_session(&mut sim);

    assert!(outcome.rejected.is_some());
    assert!(sim.storage.vault().is_empty());
    assert_eq!(phone.files.len(), 1);
}
