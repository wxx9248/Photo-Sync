//! The desktop's session behavior, driven through the same events the shell delivers.
//!
//! The desktop is answered here by a store that keeps the staging manifest in memory and by
//! storage that always succeeds. That is enough to check the decisions and their order. What
//! happens when storage lies, tears, or disappears belongs to the simulator of milestone M2.

use std::collections::{BTreeMap, VecDeque};

use photo_sync_core::covers;
use photo_sync_core::effect::{DiffSummary, Effect, ToSend, UploadOutcome};
use photo_sync_core::event::{Event, StorageOutcome};
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};
use photo_sync_core::port::StorageError;
use photo_sync_core::store::{
    DeviceFileRow, StagingEntry, StoreError, StoreRequest, StoreResponse,
};
use photo_sync_core::{CatalogEntry, CivilTime, Desktop, RunningDigest};

// ---- the desktop under test, with somewhere to put things ------------------------------

/// A desktop plus the storage that answers it. Every effect carrying an operation identifier
/// is completed successfully, in the order the effects were emitted.
struct Desk {
    core: Desktop,
    manifest: BTreeMap<FileId, (DeviceId, StagingEntry)>,
    index: Vec<DeviceFileRow>,
    free_space: u64,

    /// Set to refuse the next write, which is how a full disk reaches the session.
    refuse_writes: bool,

    /// Set to refuse the next manifest change, which a full disk causes just as readily.
    refuse_manifest: bool,

    /// What the shell reports a verified file's vault name may be built from.
    name_sources: Result<Vec<CivilTime>, StorageError>,

    log: Vec<Effect>,
}

impl Desk {
    fn new() -> Self {
        Self {
            core: Desktop::new(),
            manifest: BTreeMap::new(),
            index: Vec::new(),
            free_space: u64::MAX,
            refuse_writes: false,
            refuse_manifest: false,
            name_sources: Ok(vec![CAPTURED]),
            log: Vec::new(),
        }
    }

    /// Feeds one event and settles every effect it leads to.
    fn run(&mut self, event: Event) {
        let mut queue: VecDeque<Effect> = self.core.handle(event).into();
        while let Some(effect) = queue.pop_front() {
            if let Some(completion) = self.perform(&effect) {
                queue.extend(self.core.handle(completion));
            }
            self.log.push(effect);
        }
    }

    /// Feeds one event and returns what it produced, without answering any of it. Used where
    /// the property is that something has *not* happened yet.
    fn step(&mut self, event: Event) -> Vec<Effect> {
        self.core.handle(event)
    }

    fn perform(&mut self, effect: &Effect) -> Option<Event> {
        match effect {
            Effect::Store { op, request } => {
                let result = if self.refuse_manifest {
                    Err(StoreError::NoSpace)
                } else {
                    Ok(self.query(request))
                };
                Some(Event::StoreOpCompleted { op: *op, result })
            }
            Effect::MeasureFreeSpace { op } => Some(Event::FreeSpaceMeasured {
                op: *op,
                available_bytes: self.free_space,
            }),
            Effect::WriteChunk { op, .. } if self.refuse_writes => {
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: Err(StorageError::NoSpace),
                })
            }
            Effect::ReadNameSources { op, .. } => Some(Event::StorageOpCompleted {
                op: *op,
                result: self.name_sources.clone().map(StorageOutcome::NameSources),
            }),
            Effect::CreateStagingFile { op, .. }
            | Effect::WriteChunk { op, .. }
            | Effect::SyncFile { op, .. }
            | Effect::SyncDirectory { op, .. }
            | Effect::TruncateFile { op, .. }
            | Effect::FinalizeStagingFile { op, .. }
            | Effect::RenameIntoVault { op, .. }
            | Effect::RemoveStagingFile { op, .. }
            | Effect::ClearStagingDirectory { op, .. } => Some(Event::StorageOpCompleted {
                op: *op,
                result: Ok(StorageOutcome::Done),
            }),
            Effect::StatVaultFile { op, .. } => Some(Event::StorageOpCompleted {
                op: *op,
                result: Ok(StorageOutcome::VaultFile {
                    present: false,
                    size: 0,
                }),
            }),
            Effect::SetTimer { .. }
            | Effect::SendDiff { .. }
            | Effect::SendUploadResult { .. }
            | Effect::SendCandidates { .. }
            | Effect::SendSessionSummary { .. }
            | Effect::RejectSession { .. }
            | Effect::Log { .. }
            | Effect::NotifyUi { .. } => None,
        }
    }

    fn query(&mut self, request: &StoreRequest) -> StoreResponse {
        match request {
            StoreRequest::HighestStagingFileId => {
                StoreResponse::HighestStagingFileId(self.manifest.keys().next_back().copied())
            }
            StoreRequest::ListStagingEntries { device } => StoreResponse::StagingEntries(
                self.manifest
                    .values()
                    .filter(|(owner, _)| owner == device)
                    .map(|(_, entry)| entry.clone())
                    .collect(),
            ),
            StoreRequest::BeginStagingEntry { device, entry } => {
                self.manifest
                    .insert(entry.file, (device.clone(), entry.clone()));
                StoreResponse::Done
            }
            StoreRequest::AdvanceWatermark {
                file,
                durable_bytes,
            } => {
                if let Some((_, entry)) = self.manifest.get_mut(file) {
                    entry.durable_bytes = *durable_bytes;
                }
                StoreResponse::Done
            }
            StoreRequest::MarkStagingEntryVerified {
                file,
                digest,
                name_sources,
            } => {
                if let Some((_, entry)) = self.manifest.get_mut(file) {
                    entry.digest = Some(*digest);
                    entry.name_sources.clone_from(name_sources);
                }
                StoreResponse::Done
            }
            StoreRequest::DropStagingEntry { file } => {
                self.manifest.remove(file);
                StoreResponse::Done
            }
            StoreRequest::LookupDeviceFiles { device, paths } => StoreResponse::DeviceFiles(
                self.index
                    .iter()
                    .filter(|row| row.device == *device && paths.contains(&row.path))
                    .cloned()
                    .collect(),
            ),
            other => panic!("the session should not have asked for {other:?}"),
        }
    }

    fn take_log(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.log)
    }

    fn staged(&self, file: FileId) -> Option<&StagingEntry> {
        self.manifest.get(&file).map(|(_, entry)| entry)
    }
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
fn offer_one_photo(desk: &mut Desk) {
    desk.run(Event::Started {
        now: Timestamp(MTIME),
    });
    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![photo_entry()],
        total_bytes: PHOTO.len() as u64,
    });
    desk.run(Event::DiffRequested { device: phone() });
}

fn open_upload(desk: &mut Desk, file: FileId, offset: u64) {
    desk.run(Event::UploadOpened {
        device: phone(),
        file,
        path: photo_path(),
        offset,
    });
}

fn send_bytes(desk: &mut Desk, file: FileId, offset: u64, data: &[u8]) {
    desk.run(Event::ChunkArrived {
        device: phone(),
        file,
        offset,
        data: data.to_vec(),
    });
}

fn close_upload(desk: &mut Desk, file: FileId, digest: Sha256) {
    desk.run(Event::UploadClosed {
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
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, summary) = diff_of(&desk.take_log());
    assert_eq!(to_send.len(), 1);
    assert_eq!(summary.to_send_bytes, PHOTO.len() as u64);
    let file = to_send[0].file;

    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));

    let log = desk.take_log();
    assert_eq!(
        upload_outcome(&log),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
    let entry = match desk.staged(file) {
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
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));

    let log = desk.take_log();
    assert!(position_of(&log, is_sync) < position_of(&log, is_finalize));
    assert!(position_of(&log, is_finalize) < position_of(&log, is_marked_verified));
}

#[test]
fn the_watermark_waits_for_the_sync_that_justifies_it() {
    covers!("R-STAGE-006");
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);

    let closing = desk.step(Event::UploadClosed {
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
    let mut desk = Desk::new();
    desk.index.push(indexed_row(PHOTO.len() as u64, MTIME));

    offer_one_photo(&mut desk);

    let (to_send, summary) = diff_of(&desk.take_log());
    assert!(to_send.is_empty());
    assert_eq!(summary.already_imported, 1);
}

#[test]
fn a_dropped_connection_leaves_a_partial_the_next_diff_resumes() {
    covers!("R-SESSION-004", "R-STAGE-008", "R-DIFF-003");
    let mut desk = Desk::new();
    let (head, tail) = PHOTO.split_at(10);

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, head);
    desk.run(Event::PeerDisconnected { device: phone() });

    assert_eq!(desk.staged(file).map(|entry| entry.durable_bytes), Some(10));

    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![photo_entry()],
        total_bytes: PHOTO.len() as u64,
    });
    desk.run(Event::DiffRequested { device: phone() });

    let (resumed, summary) = diff_of(&desk.take_log());
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].file, file);
    assert_eq!(resumed[0].resume_offset, 10);
    assert_eq!(summary.to_send_bytes, tail.len() as u64);

    open_upload(&mut desk, file, 10);
    send_bytes(&mut desk, file, 10, tail);
    close_upload(&mut desk, file, digest_of(PHOTO));

    let log = desk.take_log();
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
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;

    open_upload(&mut desk, file, 12);

    assert_eq!(upload_outcome(&desk.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn more_bytes_than_the_file_declared_end_the_transfer() {
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);

    send_bytes(
        &mut desk,
        file,
        0,
        b"far more bytes than the catalog ever declared for it",
    );

    assert_eq!(upload_outcome(&desk.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn a_digest_that_does_not_match_costs_the_partial_and_buys_one_more_try() {
    covers!("R-XFER-004");
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(b"a different photograph"));

    let log = desk.take_log();
    assert_eq!(upload_outcome(&log), UploadOutcome::HashMismatch);
    assert!(log.iter().any(is_removal));
    assert!(desk.staged(file).is_none());

    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));

    assert_eq!(
        upload_outcome(&desk.take_log()),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
}

#[test]
fn a_second_mismatch_skips_the_photo() {
    covers!("R-XFER-004");
    let mut desk = Desk::new();
    let wrong = digest_of(b"a different photograph");

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;

    for _ in 0..2 {
        open_upload(&mut desk, file, 0);
        send_bytes(&mut desk, file, 0, PHOTO);
        close_upload(&mut desk, file, wrong);
        assert_eq!(
            upload_outcome(&desk.take_log()),
            UploadOutcome::HashMismatch
        );
    }

    open_upload(&mut desk, file, 0);

    assert_eq!(upload_outcome(&desk.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn a_transfer_that_does_not_fit_is_turned_away() {
    covers!("R-DIFF-005");
    let mut desk = Desk::new();
    desk.free_space = (PHOTO.len() - 1) as u64;

    offer_one_photo(&mut desk);

    let log = desk.take_log();
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
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    desk.take_log();

    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    // The superseded session is gone, so its diff cannot be asked for again until the new
    // connection has sent the frozen catalog itself.
    desk.run(Event::DiffRequested { device: phone() });

    let log = desk.take_log();
    assert!(
        !log.iter()
            .any(|effect| matches!(effect, Effect::SendDiff { .. }))
    );
}

#[test]
fn a_fresh_identifier_never_collides_with_another_device_partial() {
    covers!("R-STAGE-001");
    let mut desk = Desk::new();
    let other = DeviceId::new("phone-b");
    desk.manifest.insert(
        FileId(41),
        (
            other,
            StagingEntry {
                file: FileId(41),
                path: DevicePath::new("DCIM/Camera/IMG_9999.jpg"),
                size: 10,
                mtime: Timestamp(MTIME),
                durable_bytes: 0,
                digest: None,
                name_sources: Vec::new(),
            },
        ),
    );

    offer_one_photo(&mut desk);

    let (to_send, _) = diff_of(&desk.take_log());
    assert_eq!(to_send[0].file, FileId(42));
}

#[test]
fn a_diff_is_answered_once_for_each_connection() {
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (first, _) = diff_of(&desk.take_log());
    assert_eq!(first.len(), 1);

    desk.run(Event::DiffRequested { device: phone() });

    let log = desk.take_log();
    assert!(
        !log.iter()
            .any(|effect| matches!(effect, Effect::SendDiff { .. }))
    );
}

#[test]
fn a_catalog_of_nothing_produces_a_diff_of_nothing() {
    let mut desk = Desk::new();

    desk.run(Event::Started {
        now: Timestamp(MTIME),
    });
    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
        device: phone(),
        entries: Vec::new(),
        total_bytes: 0,
    });
    desk.run(Event::DiffRequested { device: phone() });

    let (to_send, summary) = diff_of(&desk.take_log());
    assert!(to_send.is_empty());
    assert_eq!(summary, DiffSummary::default());
}

#[test]
fn a_long_file_is_made_durable_while_it_arrives() {
    covers!("R-STAGE-006");
    // Two chunks either side of the 16 MiB the desktop lets sit unsynced.
    let half = vec![b'p'; 9 << 20];
    let whole: Vec<u8> = half.iter().chain(half.iter()).copied().collect();
    let mut desk = Desk::new();

    desk.run(Event::Started {
        now: Timestamp(MTIME),
    });
    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![CatalogEntry {
            path: photo_path(),
            size: whole.len() as u64,
            mtime: Timestamp(MTIME),
        }],
        total_bytes: whole.len() as u64,
    });
    desk.run(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;

    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, &half);
    let first = desk.take_log();
    send_bytes(&mut desk, file, half.len() as u64, &half);
    let second = desk.take_log();

    // The first chunk is under the interval, the second crosses it.
    assert!(!first.iter().any(is_sync));
    assert!(second.iter().any(is_sync));
    assert_eq!(
        desk.staged(file).map(|entry| entry.durable_bytes),
        Some(whole.len() as u64)
    );

    close_upload(&mut desk, file, digest_of(&whole));
    assert_eq!(
        upload_outcome(&desk.take_log()),
        UploadOutcome::Verified {
            durable_bytes: whole.len() as u64
        }
    );
}

#[test]
fn a_partial_of_a_photo_that_has_since_changed_is_dropped_and_removed() {
    covers!("R-STAGE-002", "R-DIFF-004");
    let mut desk = Desk::new();
    let stale = FileId(3);
    desk.manifest.insert(
        stale,
        (
            phone(),
            StagingEntry {
                file: stale,
                path: photo_path(),
                size: 11,
                mtime: Timestamp(MTIME - 500),
                durable_bytes: 4,
                digest: None,
                name_sources: Vec::new(),
            },
        ),
    );

    offer_one_photo(&mut desk);

    let log = desk.take_log();
    assert!(log.iter().any(is_entry_drop));
    assert!(log.iter().any(is_removal));
    assert!(desk.staged(stale).is_none());
    let (to_send, _) = diff_of(&log);
    assert_ne!(to_send[0].file, stale);
    assert_eq!(to_send[0].resume_offset, 0);
}

#[test]
fn a_transfer_that_exactly_fits_is_allowed() {
    covers!("R-DIFF-005");
    let mut desk = Desk::new();
    desk.free_space = PHOTO.len() as u64;

    offer_one_photo(&mut desk);

    let (to_send, _) = diff_of(&desk.take_log());
    assert_eq!(to_send.len(), 1);
}

#[test]
fn a_resumed_transfer_reuses_the_staging_file_it_already_has() {
    covers!("R-STAGE-008");
    let mut desk = Desk::new();
    let (head, tail) = PHOTO.split_at(10);

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, head);
    desk.run(Event::PeerDisconnected { device: phone() });
    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![photo_entry()],
        total_bytes: PHOTO.len() as u64,
    });
    desk.run(Event::DiffRequested { device: phone() });
    desk.take_log();

    open_upload(&mut desk, file, 10);
    let opening = desk.take_log();
    send_bytes(&mut desk, file, 10, tail);

    assert!(!opening.iter().any(is_staging_creation));
    assert!(!opening.iter().any(is_entry_drop));
}

#[test]
fn an_aborted_upload_keeps_its_partial_and_finalizes_the_watermark() {
    covers!("R-STAGE-006");
    let mut desk = Desk::new();
    let (head, _) = PHOTO.split_at(10);

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, head);
    desk.take_log();

    desk.run(Event::UploadAborted {
        device: phone(),
        file,
    });

    let log = desk.take_log();
    assert!(log.iter().any(is_sync));
    assert!(log.iter().any(is_watermark_advance));
    assert_eq!(desk.staged(file).map(|entry| entry.durable_bytes), Some(10));
}

#[test]
fn a_write_the_disk_refuses_ends_that_transfer_and_no_other() {
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    desk.refuse_writes = true;

    send_bytes(&mut desk, file, 0, PHOTO);

    let log = desk.take_log();
    assert_eq!(upload_outcome(&log), UploadOutcome::WriteFailed);
    // The partial and its row stay, so the next diff resumes rather than starting again.
    assert!(desk.staged(file).is_some());
}

#[test]
fn a_manifest_the_store_refuses_ends_that_transfer() {
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    desk.refuse_manifest = true;

    open_upload(&mut desk, file, 0);

    assert_eq!(upload_outcome(&desk.take_log()), UploadOutcome::WriteFailed);
}

#[test]
fn a_verified_photo_records_where_its_name_may_come_from() {
    let mut desk = Desk::new();

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));

    let log = desk.take_log();
    let sources_read = position_of(&log, |effect| {
        matches!(effect, Effect::ReadNameSources { .. })
    });
    // Taken after the suffix is stripped and before the manifest records the file.
    assert!(position_of(&log, is_finalize) < sources_read);
    assert!(sources_read < position_of(&log, is_marked_verified));
    assert_eq!(
        desk.staged(file).map(|entry| entry.name_sources.as_slice()),
        Some([CAPTURED].as_slice())
    );
}

#[test]
fn a_photo_that_will_not_say_when_it_was_taken_is_still_kept() {
    let mut desk = Desk::new();
    desk.name_sources = Err(StorageError::Failed("unreadable".to_string()));

    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));

    assert_eq!(
        upload_outcome(&desk.take_log()),
        UploadOutcome::Verified {
            durable_bytes: PHOTO.len() as u64
        }
    );
    let entry = match desk.staged(file) {
        Some(entry) => entry,
        None => panic!("the manifest lost the entry"),
    };
    assert_eq!(entry.digest, Some(digest_of(PHOTO)));
    assert!(entry.name_sources.is_empty());
}
