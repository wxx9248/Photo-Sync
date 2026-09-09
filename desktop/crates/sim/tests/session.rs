//! The desktop's behaviour, driven through the simulator.
//!
//! Storage answers everything and nothing crashes, which is enough to check the decisions and
//! the order they happen in. What survives a crash, a torn write, or a filesystem that only
//! pretends to sync belongs to milestone M2.

use photo_sync_core::covers;
use photo_sync_core::effect::{
    CandidateOrigin, DeletionCandidate, DiffSummary, Effect, SessionSummary, ToSend, UploadOutcome,
};
use photo_sync_core::event::{DeletionOutcome, DeletionResult, Event};
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};
use photo_sync_core::store::{DeviceFileRow, StagingEntry, StoreRequest};
use photo_sync_core::{CatalogEntry, CivilTime, Moment, RunningDigest};
use photo_sync_sim::Simulation;

/// The moment every session in these tests believes it is running at.
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

/// A started desktop with nothing on it.
fn fresh() -> Simulation {
    let mut sim = Simulation::new(IMPORTED_AT);
    sim.storage.set_default_name_sources(vec![CAPTURED]);
    sim.start();
    sim.take_log();
    sim
}

// ---- reading the effects ----------------------------------------------------------------

fn diff_of(log: &[Effect]) -> (Vec<ToSend>, DiffSummary) {
    for effect in log {
        if let Effect::SendDiff {
            to_send, summary, ..
        } = effect
        {
            return (to_send.clone(), *summary);
        }
    }
    panic!("no diff was sent");
}

fn upload_outcome(log: &[Effect]) -> UploadOutcome {
    for effect in log {
        if let Effect::SendUploadResult { outcome, .. } = effect {
            return *outcome;
        }
    }
    panic!("no upload result was sent");
}

fn count(log: &[Effect], wanted: fn(&Effect) -> bool) -> usize {
    log.iter().filter(|effect| wanted(effect)).count()
}

fn position_of(log: &[Effect], wanted: fn(&Effect) -> bool) -> usize {
    match log.iter().position(wanted) {
        Some(index) => index,
        None => panic!("the effect never appeared"),
    }
}

fn is_sync(effect: &Effect) -> bool {
    matches!(effect, Effect::SyncFile { .. })
}

fn is_finalize(effect: &Effect) -> bool {
    matches!(effect, Effect::FinalizeStagingFile { .. })
}

fn is_marked_verified(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::Store {
            request: StoreRequest::MarkStagingEntryVerified { .. },
            ..
        }
    )
}

fn is_watermark_advance(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::Store {
            request: StoreRequest::AdvanceWatermark { .. },
            ..
        }
    )
}

fn is_removal(effect: &Effect) -> bool {
    matches!(effect, Effect::RemoveStagingFile { .. })
}

fn is_staging_creation(effect: &Effect) -> bool {
    matches!(effect, Effect::CreateStagingFile { .. })
}

fn is_vault_sync(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::SyncDirectory {
            directory: photo_sync_core::Directory::Vault,
            ..
        }
    )
}

fn is_staging_sync(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::SyncDirectory {
            directory: photo_sync_core::Directory::Staging { .. },
            ..
        }
    )
}

fn is_store(effect: &Effect, wanted: fn(&StoreRequest) -> bool) -> bool {
    matches!(effect, Effect::Store { request, .. } if wanted(request))
}

fn is_done_mark(effect: &Effect) -> bool {
    is_store(effect, |request| {
        matches!(request, StoreRequest::MarkPlanEntriesDone { .. })
    })
}

fn is_row_insert(effect: &Effect) -> bool {
    is_store(effect, |request| {
        matches!(request, StoreRequest::InsertCommittedBatch { .. })
    })
}

fn is_log_clear(effect: &Effect) -> bool {
    is_store(effect, |request| {
        matches!(request, StoreRequest::ClearCommitPlan { .. })
    })
}

fn is_manifest_clear(effect: &Effect) -> bool {
    is_store(effect, |request| {
        matches!(request, StoreRequest::ClearStagingManifest { .. })
    })
}

fn is_directory_clear(effect: &Effect) -> bool {
    matches!(effect, Effect::ClearStagingDirectory { .. })
}

fn is_entry_drop(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::Store {
            request: StoreRequest::DropStagingEntry { .. },
            ..
        }
    )
}

// ---- the session, said in one place ------------------------------------------------------

const PHOTO: &[u8] = b"the bytes of one photograph";
const PATH: &str = "DCIM/Camera/IMG_0001.jpg";
const MTIME: i64 = 1_756_000_000;

/// The reading the shell reports for a photo it could date.
const CAPTURED: CivilTime = CivilTime {
    year: 2026,
    month: 8,
    day: 24,
    hour: 9,
    minute: 30,
    second: 0,
};

fn phone() -> DeviceId {
    DeviceId::new("phone-a")
}

fn photo_path() -> DevicePath {
    DevicePath::new(PATH)
}

fn photo_entry() -> CatalogEntry {
    CatalogEntry {
        path: photo_path(),
        size: PHOTO.len() as u64,
        mtime: Timestamp(MTIME),
    }
}

fn digest_of(data: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(data);
    running.peek()
}

/// Starts the desktop, connects one phone, and hands over a catalog of one photo.
fn offer_one_photo(sim: &mut Simulation) {
    sim.deliver(Event::Started {
        now: Timestamp(MTIME),
    });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![photo_entry()],
        total_bytes: PHOTO.len() as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
}

fn open_upload(sim: &mut Simulation, file: FileId, offset: u64) {
    open_upload_of(sim, file, offset, PHOTO.len() as u64);
}

/// The same, for a photograph that is not the usual one. The header states what the phone
/// holds now, which is how `SPEC.md` §6.4 notices a file that changed mid-session.
fn open_upload_of(sim: &mut Simulation, file: FileId, offset: u64, size: u64) {
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        size,
        mtime: Timestamp(MTIME),
        offset,
    });
}

fn send_bytes(sim: &mut Simulation, file: FileId, offset: u64, data: &[u8]) {
    sim.deliver(Event::ChunkArrived {
        device: phone(),
        file,
        offset,
        data: data.to_vec(),
    });
}

fn close_upload(sim: &mut Simulation, file: FileId, digest: Sha256) {
    sim.deliver(Event::UploadClosed {
        device: phone(),
        file,
        digest,
    });
}

fn indexed_row(size: u64, mtime: i64) -> DeviceFileRow {
    DeviceFileRow {
        device: phone(),
        path: photo_path(),
        size,
        mtime: Timestamp(mtime),
        digest: digest_of(PHOTO),
        vault_name: VaultName::new("2026-08-01_120000.jpg"),
        committed_at: Timestamp(MTIME),
    }
}

// ---- the tests ---------------------------------------------------------------------------

#[test]
fn one_photo_is_staged_and_verified_from_end_to_end() {
    covers!("R-XFER-001", "R-STAGE-001", "R-STAGE-005");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, summary) = diff_of(&sim.take_log());
    assert_eq!(to_send.len(), 1);
    assert_eq!(summary.to_send_bytes, PHOTO.len() as u64);
    let file = to_send[0].file;

    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));

    let log = sim.take_log();
    assert_eq!(
        upload_outcome(&log),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
    let entry = match sim.store.staged(file) {
        Some(entry) => entry,
        None => panic!("the manifest lost the entry"),
    };
    assert_eq!(entry.digest, Some(digest_of(PHOTO)));
    assert_eq!(entry.durable_bytes, PHOTO.len() as u64);
    assert_eq!(entry.path, photo_path());
}

#[test]
fn a_verified_file_is_synced_then_stripped_then_recorded() {
    covers!("R-STAGE-003");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));

    let log = sim.take_log();
    assert!(position_of(&log, is_sync) < position_of(&log, is_finalize));
    assert!(position_of(&log, is_finalize) < position_of(&log, is_marked_verified));
}

#[test]
fn the_watermark_waits_for_the_sync_that_justifies_it() {
    covers!("R-STAGE-006");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);

    let closing = sim.step(Event::UploadClosed {
        device: phone(),
        file,
        digest: digest_of(PHOTO),
    });

    assert!(closing.iter().any(is_sync));
    assert!(!closing.iter().any(is_watermark_advance));
}

#[test]
fn a_photo_the_index_already_covers_is_never_asked_for() {
    covers!("R-DIFF-001");
    let mut sim = fresh();
    sim.store
        .remember_import(indexed_row(PHOTO.len() as u64, MTIME));

    offer_one_photo(&mut sim);

    let (to_send, summary) = diff_of(&sim.take_log());
    assert!(to_send.is_empty());
    assert_eq!(summary.already_imported, 1);
}

#[test]
fn a_dropped_connection_leaves_a_partial_the_next_diff_resumes() {
    covers!("R-SESSION-004", "R-STAGE-008", "R-DIFF-003");
    let mut sim = fresh();
    let (head, tail) = PHOTO.split_at(10);

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, head);
    sim.deliver(Event::PeerDisconnected { device: phone() });

    assert_eq!(
        sim.store.staged(file).map(|entry| entry.durable_bytes),
        Some(10)
    );

    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![photo_entry()],
        total_bytes: PHOTO.len() as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });

    let (resumed, summary) = diff_of(&sim.take_log());
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].file, file);
    assert_eq!(resumed[0].resume_offset, 10);
    assert_eq!(summary.to_send_bytes, tail.len() as u64);

    open_upload(&mut sim, file, 10);
    send_bytes(&mut sim, file, 10, tail);
    close_upload(&mut sim, file, digest_of(PHOTO));

    let log = sim.take_log();
    assert_eq!(
        upload_outcome(&log),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
}

#[test]
fn an_offset_the_desktop_did_not_offer_is_refused() {
    covers!("R-SESSION-006");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;

    open_upload(&mut sim, file, 12);

    assert_eq!(upload_outcome(&sim.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn more_bytes_than_the_file_declared_end_the_transfer() {
    covers!("R-XFER-005");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);

    send_bytes(
        &mut sim,
        file,
        0,
        b"far more bytes than the catalog ever declared for it",
    );

    assert_eq!(upload_outcome(&sim.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn a_digest_that_does_not_match_costs_the_partial_and_buys_one_more_try() {
    covers!("R-XFER-004");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(b"a different photograph"));

    let log = sim.take_log();
    assert_eq!(upload_outcome(&log), UploadOutcome::HashMismatch);
    assert!(log.iter().any(is_removal));
    assert!(sim.store.staged(file).is_none());

    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));

    assert_eq!(
        upload_outcome(&sim.take_log()),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
}

#[test]
fn a_second_mismatch_skips_the_photo() {
    covers!("R-XFER-004");
    let mut sim = fresh();
    let wrong = digest_of(b"a different photograph");

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;

    for _ in 0..2 {
        open_upload(&mut sim, file, 0);
        send_bytes(&mut sim, file, 0, PHOTO);
        close_upload(&mut sim, file, wrong);
        assert_eq!(upload_outcome(&sim.take_log()), UploadOutcome::HashMismatch);
    }

    open_upload(&mut sim, file, 0);

    assert_eq!(upload_outcome(&sim.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn a_transfer_that_does_not_fit_is_turned_away() {
    covers!("R-DIFF-005");
    let mut sim = fresh();
    sim.free_space = (PHOTO.len() - 1) as u64;

    offer_one_photo(&mut sim);

    let log = sim.take_log();
    let refused = log.iter().any(|effect| {
        matches!(
            effect,
            Effect::RejectSession {
                reason: photo_sync_core::RejectReason::NotEnoughSpace { .. },
                ..
            }
        )
    });
    assert!(refused);
    assert!(
        !log.iter()
            .any(|effect| matches!(effect, Effect::SendDiff { .. }))
    );
}

#[test]
fn a_second_connection_supersedes_the_first() {
    covers!("R-SESSION-007");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    sim.take_log();

    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    // The superseded session is gone, so its diff cannot be asked for again until the new
    // connection has sent the frozen catalog itself.
    sim.deliver(Event::DiffRequested { device: phone() });

    let log = sim.take_log();
    assert!(
        !log.iter()
            .any(|effect| matches!(effect, Effect::SendDiff { .. }))
    );
}

#[test]
fn a_fresh_identifier_never_collides_with_another_device_partial() {
    covers!("R-STAGE-001");
    let mut sim = fresh();
    let other = DeviceId::new("phone-b");
    sim.stage(
        &other,
        StagingEntry {
            file: FileId(41),
            path: DevicePath::new("DCIM/Camera/IMG_9999.jpg"),
            size: 10,
            mtime: Timestamp(MTIME),
            durable_bytes: 0,
            digest: None,
            name_sources: Vec::new(),
        },
        b"",
    );

    offer_one_photo(&mut sim);

    let (to_send, _) = diff_of(&sim.take_log());
    assert_eq!(to_send[0].file, FileId(42));
}

#[test]
fn a_diff_is_answered_once_for_each_connection() {
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (first, _) = diff_of(&sim.take_log());
    assert_eq!(first.len(), 1);

    sim.deliver(Event::DiffRequested { device: phone() });

    let log = sim.take_log();
    assert!(
        !log.iter()
            .any(|effect| matches!(effect, Effect::SendDiff { .. }))
    );
}

#[test]
fn a_catalog_of_nothing_produces_a_diff_of_nothing() {
    let mut sim = fresh();

    sim.deliver(Event::Started {
        now: Timestamp(MTIME),
    });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: Vec::new(),
        total_bytes: 0,
    });
    sim.deliver(Event::DiffRequested { device: phone() });

    let (to_send, summary) = diff_of(&sim.take_log());
    assert!(to_send.is_empty());
    assert_eq!(summary, DiffSummary::default());
}

#[test]
fn a_long_file_is_made_durable_while_it_arrives() {
    covers!("R-STAGE-006");
    // Two chunks either side of the 16 MiB the desktop lets sit unsynced.
    let half = vec![b'p'; 9 << 20];
    let whole: Vec<u8> = half.iter().chain(half.iter()).copied().collect();
    let mut sim = fresh();

    sim.deliver(Event::Started {
        now: Timestamp(MTIME),
    });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![CatalogEntry {
            path: photo_path(),
            size: whole.len() as u64,
            mtime: Timestamp(MTIME),
        }],
        total_bytes: whole.len() as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;

    open_upload_of(&mut sim, file, 0, whole.len() as u64);
    send_bytes(&mut sim, file, 0, &half);
    let first = sim.take_log();
    send_bytes(&mut sim, file, half.len() as u64, &half);
    let second = sim.take_log();

    // The first chunk is under the interval, the second crosses it.
    assert!(!first.iter().any(is_sync));
    assert!(second.iter().any(is_sync));
    assert_eq!(
        sim.store.staged(file).map(|entry| entry.durable_bytes),
        Some(whole.len() as u64)
    );

    close_upload(&mut sim, file, digest_of(&whole));
    assert_eq!(
        upload_outcome(&sim.take_log()),
        UploadOutcome::Verified {
            durable_bytes: whole.len() as u64
        }
    );
}

#[test]
fn a_partial_of_a_photo_that_has_since_changed_is_dropped_and_removed() {
    covers!("R-STAGE-002", "R-DIFF-004");
    let mut sim = fresh();
    let stale = FileId(3);
    sim.stage(
        &phone(),
        StagingEntry {
            file: stale,
            path: photo_path(),
            size: 11,
            mtime: Timestamp(MTIME - 500),
            durable_bytes: 4,
            digest: None,
            name_sources: Vec::new(),
        },
        b"four",
    );

    offer_one_photo(&mut sim);

    let log = sim.take_log();
    assert!(log.iter().any(is_entry_drop));
    assert!(log.iter().any(is_removal));
    assert!(sim.store.staged(stale).is_none());
    let (to_send, _) = diff_of(&log);
    assert_ne!(to_send[0].file, stale);
    assert_eq!(to_send[0].resume_offset, 0);
}

#[test]
fn a_transfer_that_exactly_fits_is_allowed() {
    covers!("R-DIFF-005");
    let mut sim = fresh();
    sim.free_space = PHOTO.len() as u64;

    offer_one_photo(&mut sim);

    let (to_send, _) = diff_of(&sim.take_log());
    assert_eq!(to_send.len(), 1);
}

#[test]
fn a_resumed_transfer_reuses_the_staging_file_it_already_has() {
    covers!("R-STAGE-008");
    let mut sim = fresh();
    let (head, tail) = PHOTO.split_at(10);

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, head);
    sim.deliver(Event::PeerDisconnected { device: phone() });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![photo_entry()],
        total_bytes: PHOTO.len() as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    sim.take_log();

    open_upload(&mut sim, file, 10);
    let opening = sim.take_log();
    send_bytes(&mut sim, file, 10, tail);

    assert!(!opening.iter().any(is_staging_creation));
    assert!(!opening.iter().any(is_entry_drop));
}

#[test]
fn an_aborted_upload_keeps_its_partial_and_finalizes_the_watermark() {
    covers!("R-STAGE-006");
    let mut sim = fresh();
    let (head, _) = PHOTO.split_at(10);

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, head);
    sim.take_log();

    sim.deliver(Event::UploadAborted {
        device: phone(),
        file,
    });

    let log = sim.take_log();
    assert!(log.iter().any(is_sync));
    assert!(log.iter().any(is_watermark_advance));
    assert_eq!(
        sim.store.staged(file).map(|entry| entry.durable_bytes),
        Some(10)
    );
}

#[test]
fn a_write_the_disk_refuses_ends_that_transfer_and_no_other() {
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    sim.faults.refuse_writes = true;

    send_bytes(&mut sim, file, 0, PHOTO);

    let log = sim.take_log();
    assert_eq!(upload_outcome(&log), UploadOutcome::WriteFailed);
    // The partial and its row stay, so the next diff resumes rather than starting again.
    assert!(sim.store.staged(file).is_some());
}

#[test]
fn a_manifest_the_store_refuses_ends_that_transfer() {
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    sim.faults.refuse_store = true;

    open_upload(&mut sim, file, 0);

    assert_eq!(upload_outcome(&sim.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn a_verified_photo_records_where_its_name_may_come_from() {
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));

    let log = sim.take_log();
    let sources_read = position_of(&log, |effect| {
        matches!(effect, Effect::ReadNameSources { .. })
    });
    // Taken after the suffix is stripped and before the manifest records the file.
    assert!(position_of(&log, is_finalize) < sources_read);
    assert!(sources_read < position_of(&log, is_marked_verified));
    // The order of SPEC.md §7.2: what the photograph says about itself, then when it was
    // last modified.
    assert_eq!(
        sim.store
            .staged(file)
            .map(|entry| entry.name_sources.clone()),
        Some(vec![
            CAPTURED,
            photo_sync_core::naming::civil_from_unix(MTIME)
        ])
    );
}

#[test]
fn a_photo_that_will_not_say_when_it_was_taken_is_still_kept() {
    let mut sim = fresh();
    sim.faults.refuse_name_sources = true;

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));

    assert_eq!(
        upload_outcome(&sim.take_log()),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
    let entry = match sim.store.staged(file) {
        Some(entry) => entry,
        None => panic!("the manifest lost the entry"),
    };
    assert_eq!(entry.digest, Some(digest_of(PHOTO)));
    assert!(entry.name_sources.is_empty());
}

// ---- committing ---------------------------------------------------------------------------

/// Runs one photo all the way from the handshake to a committed vault copy.
fn stage_and_commit(sim: &mut Simulation) -> FileId {
    offer_one_photo(sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(sim, file, 0);
    send_bytes(sim, file, 0, PHOTO);
    close_upload(sim, file, digest_of(PHOTO));
    sim.take_log();
    sim.deliver(Event::FinishRequested { device: phone() });
    file
}

fn only_vault_name(sim: &Simulation) -> VaultName {
    let mut names = sim.storage.vault().keys();
    match (names.next(), names.next()) {
        (Some(name), None) => name.clone(),
        _ => panic!("the vault holds {} files", sim.storage.vault().len()),
    }
}

#[test]
fn a_staged_photo_is_renamed_into_the_vault_and_recorded() {
    covers!("R-COMMIT-004", "R-INDEX-001", "R-INDEX-002");
    let mut sim = fresh();

    let file = stage_and_commit(&mut sim);

    assert_eq!(only_vault_name(&sim).as_str(), "2026-08-24_093000.jpg");
    assert_eq!(
        sim.store.content().get(&digest_of(PHOTO)),
        sim.storage.vault().keys().next()
    );
    assert_eq!(sim.store.device_files().len(), 1);
    assert_eq!(sim.store.device_files()[0].path, photo_path());
    assert_eq!(sim.store.device_files()[0].committed_at, IMPORTED_AT.at);
    // Staging is left with nothing: no row, no file, no write-log.
    assert!(sim.store.manifest().is_empty());
    assert!(!sim.storage.holds(file));
    assert!(sim.store.sealed_plan(&phone()).is_none());
}

#[test]
fn the_directories_are_made_durable_before_any_done_mark() {
    covers!("R-COMMIT-008");
    let mut sim = fresh();

    stage_and_commit(&mut sim);

    let log = sim.take_log();
    let marked = position_of(&log, is_done_mark);
    assert!(position_of(&log, is_vault_sync) < marked);
    assert!(position_of(&log, is_staging_sync) < marked);
    assert!(!(sim.store.done_marks(&phone()) == 0));
}

#[test]
fn the_plan_is_sealed_before_a_single_file_moves() {
    covers!("R-COMMIT-007");
    let mut sim = fresh();

    stage_and_commit(&mut sim);

    let log = sim.take_log();
    let sealed = position_of(&log, |effect| {
        is_store(effect, |request| {
            matches!(request, StoreRequest::SealCommitPlan { .. })
        })
    });
    assert!(
        sealed
            < position_of(&log, |effect| matches!(
                effect,
                Effect::RenameIntoVault { .. }
            ))
    );
}

#[test]
fn the_index_rows_go_in_before_staging_is_cleared() {
    covers!("R-COMMIT-010", "R-COMMIT-011");
    let mut sim = fresh();

    stage_and_commit(&mut sim);

    let log = sim.take_log();
    assert!(position_of(&log, is_row_insert) < position_of(&log, is_log_clear));
    assert!(position_of(&log, is_log_clear) < position_of(&log, is_manifest_clear));
    assert!(position_of(&log, is_manifest_clear) < position_of(&log, is_directory_clear));
}

#[test]
fn content_the_vault_already_holds_is_deleted_from_staging_instead() {
    covers!("R-COMMIT-005");
    let mut sim = fresh();
    sim.store.remember_import(DeviceFileRow {
        device: DeviceId::new("phone-earlier"),
        path: DevicePath::new("DCIM/Camera/OLD.jpg"),
        size: PHOTO.len() as u64,
        mtime: Timestamp(MTIME),
        digest: digest_of(PHOTO),
        vault_name: VaultName::new("2020-01-01_000000.jpg"),
        committed_at: Timestamp(MTIME),
    });
    sim.storage
        .put_in_vault(&VaultName::new("2020-01-01_000000.jpg"), PHOTO.to_vec());

    let file = stage_and_commit(&mut sim);

    // Nothing new reached the vault, and the staged copy was deleted rather than moved.
    assert_eq!(only_vault_name(&sim).as_str(), "2020-01-01_000000.jpg");
    assert!(!sim.storage.holds(file));

    let row = match sim
        .store
        .device_files()
        .into_iter()
        .find(|row| row.device == phone())
    {
        Some(row) => row,
        None => panic!("this phone earned no index row"),
    };
    assert_eq!(row.vault_name, VaultName::new("2020-01-01_000000.jpg"));
}

#[test]
fn a_phone_that_returns_while_its_import_finishes_is_asked_to_wait() {
    covers!("R-COMMIT-002");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));

    // Take the finish signal without answering any of the work it started.
    sim.step(Event::FinishRequested { device: phone() });
    let refusal = sim.step(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });

    let waiting = refusal.iter().any(|effect| {
        matches!(
            effect,
            Effect::RejectSession {
                reason: photo_sync_core::RejectReason::CommitInProgress,
                ..
            }
        )
    });
    assert!(waiting);
}

#[test]
fn one_commit_runs_at_a_time() {
    covers!("R-COMMIT-001");
    let other = DeviceId::new("phone-b");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    sim.take_log();

    // The first device's commit is left mid-flight, then a second device finishes too.
    sim.step(Event::FinishRequested { device: phone() });
    let queued = sim.step(Event::FinishRequested {
        device: other.clone(),
    });

    assert!(!queued.iter().any(|effect| {
        matches!(
            effect,
            Effect::NotifyUi {
                update: photo_sync_core::effect::UiUpdate::CommitStarted { .. }
            }
        )
    }));
    // The second device is locked from the moment its finish signal was taken.
    let refusal = sim.step(Event::PeerConnected {
        device: other,
        name: "Hallway phone".to_string(),
    });
    assert!(refusal.iter().any(|effect| {
        matches!(
            effect,
            Effect::RejectSession {
                reason: photo_sync_core::RejectReason::CommitInProgress,
                ..
            }
        )
    }));
}

#[test]
fn a_rename_that_fails_stops_the_commit_and_keeps_the_device_locked() {
    covers!("R-COMMIT-009");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));
    sim.take_log();
    sim.faults.refuse_renames = true;

    sim.deliver(Event::FinishRequested { device: phone() });

    // The sealed write-log survives, because it is what a replay would work from.
    assert!(sim.store.sealed_plan(&phone()).is_some());
    assert!(sim.storage.vault().is_empty());
    assert!(!sim.store.manifest().is_empty());
    let refusal = sim.step(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    assert!(refusal.iter().any(|effect| {
        matches!(
            effect,
            Effect::RejectSession {
                reason: photo_sync_core::RejectReason::CommitInProgress,
                ..
            }
        )
    }));
}

#[test]
fn an_entry_whose_file_vanished_is_dropped_from_the_batch() {
    covers!("R-COMMIT-006");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));
    sim.take_log();
    sim.storage.remove(file);

    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(sim.storage.vault().is_empty());
    assert!(sim.store.device_files().is_empty());
    assert!(sim.store.manifest().is_empty());
}

#[test]
fn a_partial_left_in_staging_is_cleared_with_everything_else() {
    covers!("R-COMMIT-012");
    let mut sim = fresh();
    let (head, _) = PHOTO.split_at(10);
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, head);
    sim.take_log();

    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(sim.storage.vault().is_empty());
    assert!(sim.store.manifest().is_empty());
    assert!(!sim.storage.holds(file));
}

#[test]
fn a_commit_with_nothing_staged_still_completes() {
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    sim.take_log();

    sim.deliver(Event::FinishRequested { device: phone() });

    let log = sim.take_log();
    let finished = log.iter().any(|effect| {
        matches!(
            effect,
            Effect::NotifyUi {
                update: photo_sync_core::effect::UiUpdate::CommitFinished { .. }
            }
        )
    });
    assert!(finished);
    assert!(
        log.iter()
            .any(|effect| matches!(effect, Effect::SendCandidates { .. }))
    );
}

#[test]
fn a_second_batch_deduplicates_against_the_first() {
    covers!("R-COMMIT-003", "R-INDEX-003");
    let mut sim = fresh();
    stage_and_commit(&mut sim);
    let name = only_vault_name(&sim);
    sim.take_log();

    // The same photo arrives again under a different path, so the diff cannot skip it.
    let second = DevicePath::new("DCIM/Camera/IMG_0002.jpg");
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![CatalogEntry {
            path: second.clone(),
            size: PHOTO.len() as u64,
            mtime: Timestamp(MTIME),
        }],
        total_bytes: PHOTO.len() as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: second.clone(),
        size: PHOTO.len() as u64,
        mtime: Timestamp(MTIME),
        offset: 0,
    });
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));
    sim.deliver(Event::FinishRequested { device: phone() });

    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(sim.store.device_files().len(), 2);
    assert!(
        sim.store
            .device_files()
            .iter()
            .all(|row| row.vault_name == name)
    );
}

/// Offers a catalog of two photos and takes both of them into staging.
fn stage_two(sim: &mut Simulation, second: &[u8]) -> (FileId, FileId) {
    let other = DevicePath::new("DCIM/Camera/IMG_0002.jpg");
    sim.deliver(Event::Started {
        now: Timestamp(MTIME),
    });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![
            photo_entry(),
            CatalogEntry {
                path: other.clone(),
                size: second.len() as u64,
                mtime: Timestamp(MTIME),
            },
        ],
        total_bytes: (PHOTO.len() + second.len()) as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&sim.take_log());
    assert_eq!(to_send.len(), 2);
    let (first, last) = (to_send[0].file, to_send[1].file);

    open_upload(sim, first, 0);
    send_bytes(sim, first, 0, PHOTO);
    close_upload(sim, first, digest_of(PHOTO));
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file: last,
        path: other,
        size: second.len() as u64,
        mtime: Timestamp(MTIME),
        offset: 0,
    });
    send_bytes(sim, last, 0, second);
    close_upload(sim, last, digest_of(second));
    sim.take_log();
    (first, last)
}

#[test]
fn every_file_in_a_group_moves_before_the_group_is_marked_done() {
    covers!("R-COMMIT-008");
    let mut sim = fresh();
    let (first, last) = stage_two(&mut sim, b"a second photograph entirely");

    sim.deliver(Event::FinishRequested { device: phone() });

    let log = sim.take_log();
    let marked = position_of(&log, is_done_mark);
    let renames: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, effect)| matches!(effect, Effect::RenameIntoVault { .. }))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(renames.len(), 2);
    assert!(renames.iter().all(|rename| *rename < marked));

    // One group, so one barrier and one done-mark. A desktop that synced after each file
    // would still pass the ordering above while doing the work twice and, worse, marking a
    // group done before the rest of it had moved.
    assert_eq!(count(&log, is_vault_sync), 1);
    assert_eq!(count(&log, is_staging_sync), 1);
    assert_eq!(count(&log, is_done_mark), 1);

    assert_eq!(sim.storage.vault().len(), 2);
    assert_eq!(sim.store.done_marks(&phone()), 2);
    assert!(!sim.storage.holds(first));
    assert!(!sim.storage.holds(last));
}

#[test]
fn a_plan_is_not_sealed_until_the_last_answer_is_in() {
    covers!("R-COMMIT-006");
    let mut sim = fresh();
    let (first, _) = stage_two(&mut sim, b"a second photograph entirely");
    // The index answers before storage does, and one of the two files is gone.
    sim.faults.answer_stats_last = true;
    sim.storage.remove(first);

    sim.deliver(Event::FinishRequested { device: phone() });

    assert_eq!(sim.storage.vault().len(), 1);
    assert_eq!(sim.store.device_files().len(), 1);
    assert_eq!(
        sim.store.device_files()[0].path,
        DevicePath::new("DCIM/Camera/IMG_0002.jpg")
    );
}

#[test]
fn a_halted_commit_says_so_in_the_interface() {
    covers!("R-COMMIT-009");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));
    sim.take_log();
    sim.faults.refuse_renames = true;

    sim.deliver(Event::FinishRequested { device: phone() });

    let log = sim.take_log();
    let reported = log.iter().any(|effect| {
        matches!(
            effect,
            Effect::NotifyUi {
                update: photo_sync_core::effect::UiUpdate::Error { .. }
            }
        )
    });
    assert!(reported);
    assert!(!log.iter().any(|effect| {
        matches!(
            effect,
            Effect::NotifyUi {
                update: photo_sync_core::effect::UiUpdate::CommitFinished { .. }
            }
        )
    }));
}

// ---- nominating for deletion ---------------------------------------------------------------

fn candidates_of(log: &[Effect]) -> Vec<DeletionCandidate> {
    for effect in log {
        if let Effect::SendCandidates { candidates, .. } = effect {
            return candidates.clone();
        }
    }
    panic!("no candidates were sent");
}

fn session_summary(log: &[Effect]) -> SessionSummary {
    for effect in log {
        if let Effect::SendSessionSummary { summary, .. } = effect {
            return *summary;
        }
    }
    panic!("no session summary was sent");
}

/// An index row and a vault copy for a photo imported before this session.
fn already_imported(sim: &mut Simulation, name: &str, size: u64) {
    let vault_name = VaultName::new(name);
    sim.storage
        .put_in_vault(&vault_name, vec![0; size as usize]);
    sim.store.remember_import(DeviceFileRow {
        device: phone(),
        path: photo_path(),
        size,
        mtime: Timestamp(MTIME),
        digest: digest_of(PHOTO),
        vault_name,
        committed_at: Timestamp(MTIME),
    });
}

#[test]
fn a_photo_this_session_committed_is_offered_for_deletion() {
    covers!("R-DELETE-001", "R-DELETE-002", "R-DELETE-004");
    let mut sim = fresh();

    stage_and_commit(&mut sim);

    let candidates = candidates_of(&sim.take_log());
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].path, photo_path());
    assert_eq!(candidates[0].size, PHOTO.len() as u64);
    assert_eq!(candidates[0].mtime, Timestamp(MTIME));
    assert_eq!(candidates[0].expected, digest_of(PHOTO));
    assert_eq!(candidates[0].origin, CandidateOrigin::ThisTransfer);
}

#[test]
fn a_photo_imported_earlier_is_offered_as_such() {
    covers!("R-DELETE-002");
    let mut sim = fresh();
    already_imported(&mut sim, "2026-08-01_120000.jpg", PHOTO.len() as u64);

    offer_one_photo(&mut sim);
    sim.take_log();
    sim.deliver(Event::FinishRequested { device: phone() });

    let candidates = candidates_of(&sim.take_log());
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].origin, CandidateOrigin::Earlier);
}

#[test]
fn a_photo_the_user_curated_out_of_the_vault_is_never_offered() {
    covers!("R-DELETE-002", "R-DELETE-003");
    let mut sim = fresh();
    already_imported(&mut sim, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    // The user deleted the vault copy. The index still covers the photo.
    sim.storage.curate(&VaultName::new("2026-08-01_120000.jpg"));

    offer_one_photo(&mut sim);
    sim.take_log();
    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&sim.take_log()).is_empty());
}

#[test]
fn a_vault_copy_of_the_wrong_size_is_not_offered() {
    covers!("R-DELETE-003");
    let mut sim = fresh();
    already_imported(&mut sim, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    sim.storage
        .put_in_vault(&VaultName::new("2026-08-01_120000.jpg"), vec![0; 9]);

    offer_one_photo(&mut sim);
    sim.take_log();
    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&sim.take_log()).is_empty());
}

#[test]
fn a_vault_copy_that_cannot_be_looked_at_is_not_offered() {
    covers!("R-DELETE-010");
    let mut sim = fresh();
    already_imported(&mut sim, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    sim.faults.refuse_vault_stats = true;

    offer_one_photo(&mut sim);
    sim.take_log();
    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&sim.take_log()).is_empty());
}

#[test]
fn a_photo_outside_this_session_catalog_is_never_offered() {
    covers!("R-DELETE-001");
    let mut sim = fresh();
    let vault_name = VaultName::new("2020-01-01_000000.jpg");
    sim.storage.put_in_vault(&vault_name, vec![0; 40]);
    sim.store.remember_import(DeviceFileRow {
        device: phone(),
        path: DevicePath::new("DCIM/Camera/IMG_LONG_GONE.jpg"),
        size: 40,
        mtime: Timestamp(MTIME),
        digest: digest_of(b"something else"),
        vault_name,
        committed_at: Timestamp(MTIME),
    });

    stage_and_commit(&mut sim);

    let candidates = candidates_of(&sim.take_log());
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].path, photo_path());
}

#[test]
fn an_index_row_that_no_longer_matches_the_phone_is_not_offered() {
    covers!("R-DELETE-002");
    let mut sim = fresh();
    // The photo was edited on the phone: same path, later mtime than the index holds.
    already_imported(&mut sim, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    sim.store.remember_import(DeviceFileRow {
        device: phone(),
        path: photo_path(),
        size: PHOTO.len() as u64,
        mtime: Timestamp(MTIME - 900),
        digest: digest_of(PHOTO),
        vault_name: VaultName::new("2026-08-01_120000.jpg"),
        committed_at: Timestamp(MTIME),
    });

    offer_one_photo(&mut sim);
    sim.take_log();
    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&sim.take_log()).is_empty());
}

#[test]
fn what_the_phone_did_closes_the_session_with_a_summary() {
    let mut sim = fresh();
    stage_and_commit(&mut sim);
    sim.take_log();

    sim.deliver(Event::DeletionsReported {
        device: phone(),
        outcomes: vec![DeletionOutcome {
            path: photo_path(),
            result: DeletionResult::Deleted,
        }],
    });

    let summary = session_summary(&sim.take_log());
    assert_eq!(summary.sent, 1);
    assert_eq!(summary.deleted, 1);
    assert_eq!(summary.kept, 0);
    assert_eq!(summary.bytes_freed, PHOTO.len() as u64);
}

#[test]
fn a_photo_the_phone_kept_is_counted_as_kept() {
    let mut sim = fresh();
    stage_and_commit(&mut sim);
    sim.take_log();

    sim.deliver(Event::DeletionsReported {
        device: phone(),
        outcomes: vec![DeletionOutcome {
            path: photo_path(),
            result: DeletionResult::KeptChanged,
        }],
    });

    let summary = session_summary(&sim.take_log());
    assert_eq!(summary.kept, 1);
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.bytes_freed, 0);
}

#[test]
fn both_photos_of_a_batch_are_offered_together() {
    covers!("R-DELETE-001");
    let mut sim = fresh();
    stage_two(&mut sim, b"a second photograph entirely");

    sim.deliver(Event::FinishRequested { device: phone() });

    let candidates = candidates_of(&sim.take_log());
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.origin == CandidateOrigin::ThisTransfer)
    );
}

#[test]
fn a_photo_skipped_for_a_bad_digest_shows_in_the_summary() {
    let mut sim = fresh();
    let wrong = digest_of(b"a different photograph");
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    for _ in 0..2 {
        open_upload(&mut sim, file, 0);
        send_bytes(&mut sim, file, 0, PHOTO);
        close_upload(&mut sim, file, wrong);
    }
    sim.deliver(Event::FinishRequested { device: phone() });
    sim.take_log();

    sim.deliver(Event::DeletionsReported {
        device: phone(),
        outcomes: Vec::new(),
    });

    let summary = session_summary(&sim.take_log());
    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.sent, 0);
}

#[test]
fn a_transfer_the_disk_refused_shows_in_the_summary() {
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    sim.faults.refuse_writes = true;
    send_bytes(&mut sim, file, 0, PHOTO);
    sim.faults.refuse_writes = false;
    sim.deliver(Event::FinishRequested { device: phone() });
    sim.take_log();

    sim.deliver(Event::DeletionsReported {
        device: phone(),
        outcomes: Vec::new(),
    });

    let summary = session_summary(&sim.take_log());
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.sent, 0);
}

#[test]
fn a_deletion_the_phone_could_not_carry_out_is_counted_as_failed() {
    let mut sim = fresh();
    stage_and_commit(&mut sim);
    sim.take_log();

    sim.deliver(Event::DeletionsReported {
        device: phone(),
        outcomes: vec![DeletionOutcome {
            path: photo_path(),
            result: DeletionResult::Failed,
        }],
    });

    let summary = session_summary(&sim.take_log());
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.kept, 0);
}

#[test]
fn the_vault_copy_carries_the_time_the_photograph_had_on_the_phone() {
    covers!("R-XFER-002");
    let mut sim = fresh();

    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;
    open_upload(&mut sim, file, 0);
    send_bytes(&mut sim, file, 0, PHOTO);
    close_upload(&mut sim, file, digest_of(PHOTO));
    sim.deliver(Event::FinishRequested { device: phone() });

    // The catalog said when the photograph was last changed, and the copy in the vault says
    // the same. A copy stamped with the moment it was imported would lose that.
    let names: Vec<VaultName> = sim.storage.vault().keys().cloned().collect();
    assert_eq!(names.len(), 1);
    assert_eq!(
        sim.storage.vault_mtime(&names[0]),
        Some(Timestamp(MTIME)),
        "the vault copy does not carry the phone's modification time"
    );
}

#[test]
fn a_photograph_edited_since_the_catalog_was_taken_is_skipped() {
    covers!("R-XFER-003");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;

    // The header says the file is four bytes longer than the catalog froze it at, so it is
    // not the photograph this session agreed to take.
    open_upload_of(&mut sim, file, 0, PHOTO.len() as u64 + 4);

    let log = sim.take_log();
    assert_eq!(upload_outcome(&log), UploadOutcome::ChangedOnPhone);
    assert!(
        sim.store.staged(file).is_none(),
        "a manifest entry was kept for a photograph nobody is sending"
    );
}

#[test]
fn a_photograph_touched_since_the_catalog_was_taken_is_skipped() {
    covers!("R-XFER-003");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;

    // Same size, later moment. Two photographs of the same length are still two photographs.
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        size: PHOTO.len() as u64,
        mtime: Timestamp(MTIME + 1),
        offset: 0,
    });

    let log = sim.take_log();
    assert_eq!(upload_outcome(&log), UploadOutcome::ChangedOnPhone);
    assert!(sim.store.staged(file).is_none());
}

#[test]
fn nothing_is_kept_of_a_photograph_that_changed_while_it_was_being_sent() {
    covers!("R-XFER-003");
    let mut sim = fresh();
    offer_one_photo(&mut sim);
    let (to_send, _) = diff_of(&sim.take_log());
    let file = to_send[0].file;

    // The phone sends the whole of the new photograph and states a digest that covers it. It
    // is honest about everything except which file this is, and that is enough to refuse it.
    let edited = b"the bytes of one photograph, again".to_vec();
    open_upload_of(&mut sim, file, 0, edited.len() as u64);
    send_bytes(&mut sim, file, 0, &edited);
    close_upload(&mut sim, file, digest_of(&edited));
    sim.deliver(Event::FinishRequested { device: phone() });

    assert!(
        sim.storage.vault().is_empty(),
        "a photograph nobody catalogued reached the vault"
    );
    assert!(sim.store.device_files().is_empty());
}

#[test]
fn two_photographs_can_be_in_flight_at_once() {
    covers!("R-XFER-006");
    let second = b"a second photograph, of the dog".to_vec();
    let other = DevicePath::new("DCIM/Camera/IMG_0002.jpg");

    let mut sim = fresh();
    sim.deliver(Event::Started {
        now: Timestamp(MTIME),
    });
    sim.deliver(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    sim.deliver(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![
            photo_entry(),
            CatalogEntry {
                path: other.clone(),
                size: second.len() as u64,
                mtime: Timestamp(MTIME),
            },
        ],
        total_bytes: (PHOTO.len() + second.len()) as u64,
    });
    sim.deliver(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&sim.take_log());
    assert_eq!(to_send.len(), 2);
    let (first, last) = (to_send[0].file, to_send[1].file);

    // Both streams are open together and their chunks arrive alternately, which is what
    // several channels look like from here. Each file keeps its own digest and its own
    // watermark, so neither can be spoiled by the other's bytes.
    open_upload(&mut sim, first, 0);
    sim.deliver(Event::UploadOpened {
        device: phone(),
        file: last,
        path: other.clone(),
        size: second.len() as u64,
        mtime: Timestamp(MTIME),
        offset: 0,
    });

    let (head, tail) = PHOTO.split_at(9);
    let (other_head, other_tail) = second.split_at(11);
    send_bytes(&mut sim, first, 0, head);
    send_bytes(&mut sim, last, 0, other_head);
    send_bytes(&mut sim, first, head.len() as u64, tail);
    send_bytes(&mut sim, last, other_head.len() as u64, other_tail);

    close_upload(&mut sim, last, digest_of(&second));
    close_upload(&mut sim, first, digest_of(PHOTO));
    sim.deliver(Event::FinishRequested { device: phone() });

    assert_eq!(sim.storage.vault().len(), 2);
    let stored: Vec<Vec<u8>> = sim.storage.vault().values().cloned().collect();
    assert!(
        stored.contains(&PHOTO.to_vec()),
        "the first photograph is not in the vault"
    );
    assert!(
        stored.contains(&second),
        "the second photograph is not in the vault"
    );
    assert_eq!(sim.store.device_files().len(), 2);
}
