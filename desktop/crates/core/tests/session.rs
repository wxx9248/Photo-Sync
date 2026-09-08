//! The desktop's session behavior, driven through the same events the shell delivers.
//!
//! The desktop is answered here by a store that keeps the staging manifest in memory and by
//! storage that always succeeds. That is enough to check the decisions and their order. What
//! happens when storage lies, tears, or disappears belongs to the simulator of milestone M2.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use photo_sync_core::covers;
use photo_sync_core::effect::{
    CandidateOrigin, DeletionCandidate, DiffSummary, Effect, SessionSummary, ToSend, UploadOutcome,
};
use photo_sync_core::event::{DeletionOutcome, DeletionResult, Event, StorageOutcome};
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};
use photo_sync_core::port::StorageError;
use photo_sync_core::store::{
    ContentRow, DeviceFileRow, PlanEntry, StagingEntry, StoreError, StoreRequest, StoreResponse,
};
use photo_sync_core::{CatalogEntry, CivilTime, Desktop, Moment, RunningDigest};

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

    /// Set to refuse the next rename into the vault, which halts a commit part-way.
    refuse_renames: bool,

    /// Set to refuse to look at a vault copy at all.
    refuse_stats: bool,

    /// Set to answer every staging stat only once nothing else is outstanding. A real shell
    /// answers in whatever order its threads finish, and a commit may not seal its plan
    /// until the last of those answers is in.
    answer_stats_last: bool,

    /// Which device each staging identifier belongs to. Outlives the manifest, the way a
    /// directory outlives the rows describing it.
    owner: BTreeMap<FileId, DeviceId>,

    /// Staging files that exist on disk.
    staged_files: BTreeSet<FileId>,

    /// The vault, as names and the size of what is under them.
    vault: BTreeMap<VaultName, u64>,

    content: BTreeMap<Sha256, VaultName>,
    sealed: Option<Vec<PlanEntry>>,
    done_marks: Vec<FileId>,

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
            refuse_renames: false,
            refuse_stats: false,
            answer_stats_last: false,
            owner: BTreeMap::new(),
            staged_files: BTreeSet::new(),
            vault: BTreeMap::new(),
            content: BTreeMap::new(),
            sealed: None,
            done_marks: Vec::new(),
            log: Vec::new(),
        }
    }

    /// Feeds one event and settles every effect it leads to.
    fn run(&mut self, event: Event) {
        let mut queue: VecDeque<Effect> = self.core.handle(event).into();
        let mut held: VecDeque<Event> = VecDeque::new();
        loop {
            while let Some(effect) = queue.pop_front() {
                let completion = self.perform(&effect);
                let holding =
                    self.answer_stats_last && matches!(effect, Effect::StatStagingFile { .. });
                self.log.push(effect);
                let Some(completion) = completion else {
                    continue;
                };
                if holding {
                    held.push_back(completion);
                } else {
                    queue.extend(self.core.handle(completion));
                }
            }
            let Some(deferred) = held.pop_front() else {
                break;
            };
            queue.extend(self.core.handle(deferred));
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
            Effect::ReadClock { op } => Some(Event::ClockRead {
                op: *op,
                moment: IMPORTED_AT,
            }),
            Effect::CreateStagingFile { op, device, file } => {
                self.owner.insert(*file, device.clone());
                self.staged_files.insert(*file);
                Some(done(*op))
            }
            Effect::StatStagingFile { op, file } => Some(Event::StorageOpCompleted {
                op: *op,
                result: Ok(StorageOutcome::StagingFile {
                    present: self.staged_files.contains(file),
                }),
            }),
            Effect::RenameIntoVault { op, file, name } => {
                if self.refuse_renames {
                    return Some(Event::StorageOpCompleted {
                        op: *op,
                        result: Err(StorageError::NoSpace),
                    });
                }
                let size = self.manifest.get(file).map_or(0, |(_, entry)| entry.size);
                self.staged_files.remove(file);
                self.vault.insert(name.clone(), size);
                Some(done(*op))
            }
            Effect::RemoveStagingFile { op, file } => {
                self.staged_files.remove(file);
                Some(done(*op))
            }
            Effect::ClearStagingDirectory { op, device } => {
                self.staged_files
                    .retain(|file| self.owner.get(file) != Some(device));
                Some(done(*op))
            }
            Effect::ReadNameSources { op, .. } => Some(Event::StorageOpCompleted {
                op: *op,
                result: self.name_sources.clone().map(StorageOutcome::NameSources),
            }),
            Effect::WriteChunk { op, .. }
            | Effect::SyncFile { op, .. }
            | Effect::SyncDirectory { op, .. }
            | Effect::TruncateFile { op, .. }
            | Effect::FinalizeStagingFile { op, .. } => Some(done(*op)),
            Effect::StatVaultFile { op, name } => {
                if self.refuse_stats {
                    return Some(Event::StorageOpCompleted {
                        op: *op,
                        result: Err(StorageError::Failed("unreadable".to_string())),
                    });
                }
                let found = self.vault.get(name).copied();
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: Ok(StorageOutcome::VaultFile {
                        present: found.is_some(),
                        size: found.unwrap_or(0),
                    }),
                })
            }
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
            StoreRequest::LookupContent { digests } => StoreResponse::Content(
                digests
                    .iter()
                    .filter_map(|digest| {
                        self.content.get(digest).map(|vault_name| ContentRow {
                            digest: *digest,
                            vault_name: vault_name.clone(),
                        })
                    })
                    .collect(),
            ),
            StoreRequest::TakenVaultNames { stems } => StoreResponse::VaultNames(
                self.content
                    .values()
                    .filter(|name| stems.iter().any(|stem| name.as_str().starts_with(stem)))
                    .cloned()
                    .collect(),
            ),
            StoreRequest::SealCommitPlan { plan, .. } => {
                self.sealed = Some(plan.clone());
                StoreResponse::Done
            }
            StoreRequest::MarkPlanEntriesDone { files, .. } => {
                self.done_marks.extend(files.iter().copied());
                StoreResponse::Done
            }
            StoreRequest::InsertCommittedBatch {
                contents,
                device_files,
            } => {
                for row in contents {
                    self.content.insert(row.digest, row.vault_name.clone());
                }
                // Keyed by device and path, so a re-import replaces the row it supersedes.
                for row in device_files {
                    self.index
                        .retain(|held| held.device != row.device || held.path != row.path);
                    self.index.push(row.clone());
                }
                StoreResponse::Done
            }
            StoreRequest::ClearCommitPlan { .. } => {
                self.sealed = None;
                StoreResponse::Done
            }
            StoreRequest::ClearStagingManifest { device } => {
                self.manifest.retain(|_, (owner, _)| owner != device);
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

/// The moment every commit in these tests believes it is running at.
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

fn done(op: photo_sync_core::OpId) -> Event {
    Event::StorageOpCompleted {
        op,
        result: Ok(StorageOutcome::Done),
    }
}

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

// ---- committing ---------------------------------------------------------------------------

/// Runs one photo all the way from the handshake to a committed vault copy.
fn stage_and_commit(desk: &mut Desk) -> FileId {
    offer_one_photo(desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(desk, file, 0);
    send_bytes(desk, file, 0, PHOTO);
    close_upload(desk, file, digest_of(PHOTO));
    desk.take_log();
    desk.run(Event::FinishRequested { device: phone() });
    file
}

fn only_vault_name(desk: &Desk) -> VaultName {
    let mut names = desk.vault.keys();
    match (names.next(), names.next()) {
        (Some(name), None) => name.clone(),
        _ => panic!("the vault holds {} files", desk.vault.len()),
    }
}

#[test]
fn a_staged_photo_is_renamed_into_the_vault_and_recorded() {
    covers!("R-COMMIT-004", "R-INDEX-001", "R-INDEX-002");
    let mut desk = Desk::new();

    let file = stage_and_commit(&mut desk);

    assert_eq!(only_vault_name(&desk).as_str(), "2026-08-24_093000.jpg");
    assert_eq!(
        desk.content.get(&digest_of(PHOTO)),
        desk.vault.keys().next()
    );
    assert_eq!(desk.index.len(), 1);
    assert_eq!(desk.index[0].path, photo_path());
    assert_eq!(desk.index[0].committed_at, IMPORTED_AT.at);
    // Staging is left with nothing: no row, no file, no write-log.
    assert!(desk.manifest.is_empty());
    assert!(!desk.staged_files.contains(&file));
    assert!(desk.sealed.is_none());
}

#[test]
fn the_directories_are_made_durable_before_any_done_mark() {
    covers!("R-COMMIT-008");
    let mut desk = Desk::new();

    stage_and_commit(&mut desk);

    let log = desk.take_log();
    let marked = position_of(&log, is_done_mark);
    assert!(position_of(&log, is_vault_sync) < marked);
    assert!(position_of(&log, is_staging_sync) < marked);
    assert!(!desk.done_marks.is_empty());
}

#[test]
fn the_plan_is_sealed_before_a_single_file_moves() {
    covers!("R-COMMIT-007");
    let mut desk = Desk::new();

    stage_and_commit(&mut desk);

    let log = desk.take_log();
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
    let mut desk = Desk::new();

    stage_and_commit(&mut desk);

    let log = desk.take_log();
    assert!(position_of(&log, is_row_insert) < position_of(&log, is_log_clear));
    assert!(position_of(&log, is_log_clear) < position_of(&log, is_manifest_clear));
    assert!(position_of(&log, is_manifest_clear) < position_of(&log, is_directory_clear));
}

#[test]
fn content_the_vault_already_holds_is_deleted_from_staging_instead() {
    covers!("R-COMMIT-005");
    let mut desk = Desk::new();
    desk.content
        .insert(digest_of(PHOTO), VaultName::new("2020-01-01_000000.jpg"));

    let file = stage_and_commit(&mut desk);

    assert!(desk.vault.is_empty());
    assert!(!desk.staged_files.contains(&file));
    assert_eq!(desk.index.len(), 1);
    assert_eq!(
        desk.index[0].vault_name,
        VaultName::new("2020-01-01_000000.jpg")
    );
}

#[test]
fn a_phone_that_returns_while_its_import_finishes_is_asked_to_wait() {
    covers!("R-COMMIT-002");
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));

    // Take the finish signal without answering any of the work it started.
    desk.step(Event::FinishRequested { device: phone() });
    let refusal = desk.step(Event::PeerConnected {
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
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    desk.take_log();

    // The first device's commit is left mid-flight, then a second device finishes too.
    desk.step(Event::FinishRequested { device: phone() });
    let queued = desk.step(Event::FinishRequested {
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
    let refusal = desk.step(Event::PeerConnected {
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
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));
    desk.take_log();
    desk.refuse_renames = true;

    desk.run(Event::FinishRequested { device: phone() });

    // The sealed write-log survives, because it is what a replay would work from.
    assert!(desk.sealed.is_some());
    assert!(desk.vault.is_empty());
    assert!(!desk.manifest.is_empty());
    let refusal = desk.step(Event::PeerConnected {
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
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));
    desk.take_log();
    desk.staged_files.remove(&file);

    desk.run(Event::FinishRequested { device: phone() });

    assert!(desk.vault.is_empty());
    assert!(desk.index.is_empty());
    assert!(desk.manifest.is_empty());
}

#[test]
fn a_partial_left_in_staging_is_cleared_with_everything_else() {
    covers!("R-COMMIT-012");
    let mut desk = Desk::new();
    let (head, _) = PHOTO.split_at(10);
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, head);
    desk.take_log();

    desk.run(Event::FinishRequested { device: phone() });

    assert!(desk.vault.is_empty());
    assert!(desk.manifest.is_empty());
    assert!(!desk.staged_files.contains(&file));
}

#[test]
fn a_commit_with_nothing_staged_still_completes() {
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    desk.take_log();

    desk.run(Event::FinishRequested { device: phone() });

    let log = desk.take_log();
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
    let mut desk = Desk::new();
    stage_and_commit(&mut desk);
    let name = only_vault_name(&desk);
    desk.take_log();

    // The same photo arrives again under a different path, so the diff cannot skip it.
    let second = DevicePath::new("DCIM/Camera/IMG_0002.jpg");
    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
        device: phone(),
        entries: vec![CatalogEntry {
            path: second.clone(),
            size: PHOTO.len() as u64,
            mtime: Timestamp(MTIME),
        }],
        total_bytes: PHOTO.len() as u64,
    });
    desk.run(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    desk.run(Event::UploadOpened {
        device: phone(),
        file,
        path: second.clone(),
        offset: 0,
    });
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));
    desk.run(Event::FinishRequested { device: phone() });

    assert_eq!(desk.vault.len(), 1);
    assert_eq!(desk.index.len(), 2);
    assert!(desk.index.iter().all(|row| row.vault_name == name));
}

/// Offers a catalog of two photos and takes both of them into staging.
fn stage_two(desk: &mut Desk, second: &[u8]) -> (FileId, FileId) {
    let other = DevicePath::new("DCIM/Camera/IMG_0002.jpg");
    desk.run(Event::Started {
        now: Timestamp(MTIME),
    });
    desk.run(Event::PeerConnected {
        device: phone(),
        name: "Kitchen phone".to_string(),
    });
    desk.run(Event::CatalogSubmitted {
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
    desk.run(Event::DiffRequested { device: phone() });
    let (to_send, _) = diff_of(&desk.take_log());
    assert_eq!(to_send.len(), 2);
    let (first, last) = (to_send[0].file, to_send[1].file);

    open_upload(desk, first, 0);
    send_bytes(desk, first, 0, PHOTO);
    close_upload(desk, first, digest_of(PHOTO));
    desk.run(Event::UploadOpened {
        device: phone(),
        file: last,
        path: other,
        offset: 0,
    });
    send_bytes(desk, last, 0, second);
    close_upload(desk, last, digest_of(second));
    desk.take_log();
    (first, last)
}

#[test]
fn every_file_in_a_group_moves_before_the_group_is_marked_done() {
    covers!("R-COMMIT-008");
    let mut desk = Desk::new();
    let (first, last) = stage_two(&mut desk, b"a second photograph entirely");

    desk.run(Event::FinishRequested { device: phone() });

    let log = desk.take_log();
    let marked = position_of(&log, is_done_mark);
    let renames: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, effect)| matches!(effect, Effect::RenameIntoVault { .. }))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(renames.len(), 2);
    assert!(renames.iter().all(|rename| *rename < marked));
    assert_eq!(desk.vault.len(), 2);
    assert_eq!(desk.done_marks.len(), 2);
    assert!(!desk.staged_files.contains(&first));
    assert!(!desk.staged_files.contains(&last));
}

#[test]
fn a_plan_is_not_sealed_until_the_last_answer_is_in() {
    covers!("R-COMMIT-006");
    let mut desk = Desk::new();
    let (first, _) = stage_two(&mut desk, b"a second photograph entirely");
    // The index answers before storage does, and one of the two files is gone.
    desk.answer_stats_last = true;
    desk.staged_files.remove(&first);

    desk.run(Event::FinishRequested { device: phone() });

    assert_eq!(desk.vault.len(), 1);
    assert_eq!(desk.index.len(), 1);
    assert_eq!(
        desk.index[0].path,
        DevicePath::new("DCIM/Camera/IMG_0002.jpg")
    );
}

#[test]
fn a_halted_commit_says_so_in_the_interface() {
    covers!("R-COMMIT-009");
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    send_bytes(&mut desk, file, 0, PHOTO);
    close_upload(&mut desk, file, digest_of(PHOTO));
    desk.take_log();
    desk.refuse_renames = true;

    desk.run(Event::FinishRequested { device: phone() });

    let log = desk.take_log();
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
fn already_imported(desk: &mut Desk, name: &str, size: u64) {
    let vault_name = VaultName::new(name);
    desk.vault.insert(vault_name.clone(), size);
    desk.content.insert(digest_of(PHOTO), vault_name.clone());
    desk.index.push(DeviceFileRow {
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
    let mut desk = Desk::new();

    stage_and_commit(&mut desk);

    let candidates = candidates_of(&desk.take_log());
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
    let mut desk = Desk::new();
    already_imported(&mut desk, "2026-08-01_120000.jpg", PHOTO.len() as u64);

    offer_one_photo(&mut desk);
    desk.take_log();
    desk.run(Event::FinishRequested { device: phone() });

    let candidates = candidates_of(&desk.take_log());
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].origin, CandidateOrigin::Earlier);
}

#[test]
fn a_photo_the_user_curated_out_of_the_vault_is_never_offered() {
    covers!("R-DELETE-002", "R-DELETE-003");
    let mut desk = Desk::new();
    already_imported(&mut desk, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    // The user deleted the vault copy. The index still covers the photo.
    desk.vault.clear();

    offer_one_photo(&mut desk);
    desk.take_log();
    desk.run(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&desk.take_log()).is_empty());
}

#[test]
fn a_vault_copy_of_the_wrong_size_is_not_offered() {
    covers!("R-DELETE-003");
    let mut desk = Desk::new();
    already_imported(&mut desk, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    desk.vault
        .insert(VaultName::new("2026-08-01_120000.jpg"), 9);

    offer_one_photo(&mut desk);
    desk.take_log();
    desk.run(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&desk.take_log()).is_empty());
}

#[test]
fn a_vault_copy_that_cannot_be_looked_at_is_not_offered() {
    covers!("R-DELETE-010");
    let mut desk = Desk::new();
    already_imported(&mut desk, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    desk.refuse_stats = true;

    offer_one_photo(&mut desk);
    desk.take_log();
    desk.run(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&desk.take_log()).is_empty());
}

#[test]
fn a_photo_outside_this_session_catalog_is_never_offered() {
    covers!("R-DELETE-001");
    let mut desk = Desk::new();
    let vault_name = VaultName::new("2020-01-01_000000.jpg");
    desk.vault.insert(vault_name.clone(), 40);
    desk.index.push(DeviceFileRow {
        device: phone(),
        path: DevicePath::new("DCIM/Camera/IMG_LONG_GONE.jpg"),
        size: 40,
        mtime: Timestamp(MTIME),
        digest: digest_of(b"something else"),
        vault_name,
        committed_at: Timestamp(MTIME),
    });

    stage_and_commit(&mut desk);

    let candidates = candidates_of(&desk.take_log());
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].path, photo_path());
}

#[test]
fn an_index_row_that_no_longer_matches_the_phone_is_not_offered() {
    covers!("R-DELETE-002");
    let mut desk = Desk::new();
    // The photo was edited on the phone: same path, later mtime than the index holds.
    already_imported(&mut desk, "2026-08-01_120000.jpg", PHOTO.len() as u64);
    if let Some(row) = desk.index.last_mut() {
        row.mtime = Timestamp(MTIME - 900);
    }

    offer_one_photo(&mut desk);
    desk.take_log();
    desk.run(Event::FinishRequested { device: phone() });

    assert!(candidates_of(&desk.take_log()).is_empty());
}

#[test]
fn what_the_phone_did_closes_the_session_with_a_summary() {
    let mut desk = Desk::new();
    stage_and_commit(&mut desk);
    desk.take_log();

    desk.run(Event::DeletionsReported {
        device: phone(),
        outcomes: vec![DeletionOutcome {
            path: photo_path(),
            result: DeletionResult::Deleted,
        }],
    });

    let summary = session_summary(&desk.take_log());
    assert_eq!(summary.sent, 1);
    assert_eq!(summary.deleted, 1);
    assert_eq!(summary.kept, 0);
    assert_eq!(summary.bytes_freed, PHOTO.len() as u64);
}

#[test]
fn a_photo_the_phone_kept_is_counted_as_kept() {
    let mut desk = Desk::new();
    stage_and_commit(&mut desk);
    desk.take_log();

    desk.run(Event::DeletionsReported {
        device: phone(),
        outcomes: vec![DeletionOutcome {
            path: photo_path(),
            result: DeletionResult::KeptChanged,
        }],
    });

    let summary = session_summary(&desk.take_log());
    assert_eq!(summary.kept, 1);
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.bytes_freed, 0);
}

#[test]
fn both_photos_of_a_batch_are_offered_together() {
    covers!("R-DELETE-001");
    let mut desk = Desk::new();
    stage_two(&mut desk, b"a second photograph entirely");

    desk.run(Event::FinishRequested { device: phone() });

    let candidates = candidates_of(&desk.take_log());
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.origin == CandidateOrigin::ThisTransfer)
    );
}

#[test]
fn a_photo_skipped_for_a_bad_digest_shows_in_the_summary() {
    let mut desk = Desk::new();
    let wrong = digest_of(b"a different photograph");
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    for _ in 0..2 {
        open_upload(&mut desk, file, 0);
        send_bytes(&mut desk, file, 0, PHOTO);
        close_upload(&mut desk, file, wrong);
    }
    desk.run(Event::FinishRequested { device: phone() });
    desk.take_log();

    desk.run(Event::DeletionsReported {
        device: phone(),
        outcomes: Vec::new(),
    });

    let summary = session_summary(&desk.take_log());
    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.sent, 0);
}

#[test]
fn a_transfer_the_disk_refused_shows_in_the_summary() {
    let mut desk = Desk::new();
    offer_one_photo(&mut desk);
    let (to_send, _) = diff_of(&desk.take_log());
    let file = to_send[0].file;
    open_upload(&mut desk, file, 0);
    desk.refuse_writes = true;
    send_bytes(&mut desk, file, 0, PHOTO);
    desk.refuse_writes = false;
    desk.run(Event::FinishRequested { device: phone() });
    desk.take_log();

    desk.run(Event::DeletionsReported {
        device: phone(),
        outcomes: Vec::new(),
    });

    let summary = session_summary(&desk.take_log());
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.sent, 0);
}

#[test]
fn a_deletion_the_phone_could_not_carry_out_is_counted_as_failed() {
    let mut desk = Desk::new();
    stage_and_commit(&mut desk);
    desk.take_log();

    desk.run(Event::DeletionsReported {
        device: phone(),
        outcomes: vec![DeletionOutcome {
            path: photo_path(),
            result: DeletionResult::Failed,
        }],
    });

    let summary = session_summary(&desk.take_log());
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.kept, 0);
}
