//! The desktop, driven by events and answered in effects.
//!
//! Every decision `SPEC.md` §6 asks of the desktop is made here, and nothing else is. The
//! module owns no files, no sockets, and no clock: it records what it has been told, decides
//! what should happen next, and hands the work out as [`Effect`] values that whatever drives
//! it performs in the order they were emitted. That ordering is the contract the durability
//! rules of §7 rest on, so a driver may not reorder them.
//!
//! Commit, deletion nomination, and startup recovery are not built yet. The desktop says so
//! when a phone asks for them rather than reporting a session it did not finish.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{Catalog, CatalogEntry};
use crate::digest::RunningDigest;
use crate::effect::{Effect, LogLevel, RejectReason, ToSend, UploadOutcome};
use crate::event::{Event, StorageOutcome};
use crate::id::{DeviceId, DevicePath, FileId, OpId, Sha256};
use crate::port::StorageError;
use crate::session::{FileIds, Input, Phase, PlannedSend, Session, Upload, UploadState, classify};
use crate::store::{StagingEntry, StoreError, StoreRequest, StoreResponse};

/// How much of a partial file may sit unsynced before the desktop makes it durable and moves
/// the watermark on. `STACK.md` §3.4 fixes the interval at 16 MiB.
const SYNC_INTERVAL_BYTES: u64 = 16 * 1024 * 1024;

/// A hash mismatch buys one full re-transfer. `SPEC.md` §6.5 skips the file after that.
const MAX_ATTEMPTS: u32 = 2;

pub struct Desktop {
    sessions: BTreeMap<DeviceId, Session>,

    /// The digest of each partial file as of its durable watermark.
    ///
    /// This is what makes a resumed transfer verifiable end to end. It outlives the session,
    /// because a phone that reconnects resumes a partial the previous connection began. It
    /// does not outlive the process, so after a restart the desktop cannot continue a digest
    /// and starts those files again. `SPEC.md` §7.6 allows that: resume is an optimization,
    /// and replaying the prefix through the digest arrives with milestone M3.
    durable_digests: BTreeMap<FileId, RunningDigest>,

    pending: BTreeMap<OpId, Pending>,

    /// Absent until the manifest has been asked for its highest identifier at startup.
    files: Option<FileIds>,

    /// Devices whose diff is waiting for that answer.
    awaiting_ids: BTreeSet<DeviceId>,

    next_op: u64,
}

/// What an outstanding effect was for. The core learns an operation's outcome by identifier,
/// so it has to remember what it asked for.
#[derive(Debug)]
enum Pending {
    SeedFileIds,
    ListStaging {
        device: DeviceId,
    },
    LookupImported {
        device: DeviceId,
    },
    FreeSpace {
        device: DeviceId,
    },
    DropSuperseded {
        file: FileId,
    },
    RemoveSuperseded {
        file: FileId,
    },
    BeginEntry {
        device: DeviceId,
        file: FileId,
    },
    CreateFile {
        device: DeviceId,
        file: FileId,
    },
    WriteChunk {
        device: DeviceId,
        file: FileId,
    },
    SyncFile(Sync),
    AdvanceWatermark(Sync),
    FinalizeFile {
        device: DeviceId,
        file: FileId,
        digest: Sha256,
    },
    MarkVerified {
        device: DeviceId,
        file: FileId,
    },
    DropMismatched {
        device: DeviceId,
        file: FileId,
    },
    RemoveMismatched {
        file: FileId,
    },
}

/// One durability step: a file sync and the watermark advance that follows it.
#[derive(Debug)]
struct Sync {
    device: DeviceId,
    file: FileId,

    /// Bytes the sync covers, which is what the watermark becomes.
    through: u64,

    /// The digest as of exactly those bytes, held so a resumed transfer continues from a
    /// digest that matches the prefix on disk rather than one that ran ahead of it.
    snapshot: RunningDigest,

    then: AfterWatermark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterWatermark {
    /// A periodic sync inside a transfer that carries on.
    Continue,

    /// The last sync before the digest decides the file's fate.
    Verify,

    /// The connection went away. The watermark is being finalized and nothing follows.
    Stop,
}

/// Why a chunk cannot be taken.
enum ChunkRefusal {
    UnknownSession,
    UnknownUpload,
    NotReceiving,
    OutOfOrder { expected: u64 },
    PastEnd { size: u64 },
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Desktop {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            durable_digests: BTreeMap::new(),
            pending: BTreeMap::new(),
            files: None,
            awaiting_ids: BTreeSet::new(),
            next_op: 0,
        }
    }

    /// Consumes one event and answers with the work it produced, in the order it must happen.
    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Started { now: _ } => self.started(),
            Event::TimerFired { op, now: _ } => {
                vec![warn(format!("timer {op:?} fired but none was set"))]
            }
            Event::PeerConnected { device, name } => self.peer_connected(device, name),
            Event::PeerDisconnected { device } => self.peer_disconnected(&device),
            Event::CatalogSubmitted {
                device,
                entries,
                total_bytes,
            } => self.catalog_submitted(&device, entries, total_bytes),
            Event::DiffRequested { device } => self.diff_requested(&device),
            Event::UploadOpened {
                device,
                file,
                path,
                offset,
            } => self.upload_opened(&device, file, &path, offset),
            Event::ChunkArrived {
                device,
                file,
                offset,
                data,
            } => self.chunk_arrived(&device, file, offset, data),
            Event::UploadClosed {
                device,
                file,
                digest,
            } => self.upload_closed(&device, file, digest),
            Event::UploadAborted { device, file } => self.upload_aborted(&device, file),
            Event::FinishRequested { device } => {
                Self::needs_commit_path("the finish signal", &device)
            }
            Event::DeletionsReported {
                device,
                outcomes: _,
            } => Self::needs_commit_path("the deletion results", &device),
            Event::ManualCommitRequested { device } => {
                Self::needs_commit_path("\"Commit now\"", &device)
            }
            Event::StorageOpCompleted { op, result } => self.storage_completed(op, result),
            Event::StoreOpCompleted { op, result } => self.store_completed(op, result),
            Event::FreeSpaceMeasured {
                op,
                available_bytes,
            } => self.free_space_measured(op, available_bytes),
            Event::ShutdownRequested => Vec::new(),
        }
    }

    /// Says plainly that a step is not built rather than reporting a session as finished.
    /// The commit sequence of §7.3 and the deletion protocol of §8 are the next change in
    /// milestone M1, described in `docs/ROADMAP.md`.
    fn needs_commit_path(step: &str, device: &DeviceId) -> Vec<Effect> {
        vec![warn(format!(
            "{step} from {device} needs the commit sequence, which is not built yet"
        ))]
    }

    fn started(&mut self) -> Vec<Effect> {
        let op = self.begin(Pending::SeedFileIds);
        vec![Effect::Store {
            op,
            request: StoreRequest::HighestStagingFileId,
        }]
    }

    fn begin(&mut self, pending: Pending) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.pending.insert(op, pending);
        op
    }

    // ---- the session, from connection to diff -------------------------------------------

    fn peer_connected(&mut self, device: DeviceId, name: String) -> Vec<Effect> {
        let mut effects = Vec::new();

        // A second live connection for one device supersedes the first. Whatever the previous
        // one was receiving is durable up to its watermark, so the only thing worth doing is
        // pushing that watermark as far as the bytes already in hand allow.
        if let Some(previous) = self.sessions.remove(&device) {
            effects.extend(self.finalize_partials(&previous));
            effects.push(info(format!(
                "{name} ({device}) reconnected, superseding its session"
            )));
        }

        self.sessions
            .insert(device.clone(), Session::connected(device));
        effects
    }

    fn peer_disconnected(&mut self, device: &DeviceId) -> Vec<Effect> {
        let Some(session) = self.sessions.remove(device) else {
            return vec![unknown_session(device)];
        };
        self.finalize_partials(&session)
    }

    /// Makes the bytes already received durable when a connection goes away, so the next diff
    /// resumes from as far along as the desktop can honestly claim. `SPEC.md` §7.6.
    fn finalize_partials(&mut self, session: &Session) -> Vec<Effect> {
        let unfinished: Vec<FileId> = session
            .uploads
            .iter()
            .filter(|(_, upload)| upload.state == UploadState::Receiving)
            .map(|(file, _)| *file)
            .collect();

        let mut effects = Vec::new();
        for file in unfinished {
            let Some(upload) = session.uploads.get(&file) else {
                continue;
            };
            let sync = Sync {
                device: session.device.clone(),
                file,
                through: upload.received,
                snapshot: upload.digest.clone(),
                then: AfterWatermark::Stop,
            };
            let op = self.begin(Pending::SyncFile(sync));
            effects.push(Effect::SyncFile { op, file });
        }
        effects
    }

    fn catalog_submitted(
        &mut self,
        device: &DeviceId,
        entries: Vec<CatalogEntry>,
        total_bytes: u64,
    ) -> Vec<Effect> {
        let Some(session) = self.sessions.get_mut(device) else {
            return vec![unknown_session(device)];
        };
        if !matches!(session.phase, Phase::AwaitingCatalog) {
            return vec![warn(format!(
                "{device} sent a catalog while {}",
                describe(&session.phase)
            ))];
        }
        session.freeze(Catalog::new(entries, total_bytes));
        Vec::new()
    }

    fn diff_requested(&mut self, device: &DeviceId) -> Vec<Effect> {
        let Some(session) = self.sessions.get_mut(device) else {
            return vec![unknown_session(device)];
        };
        if !matches!(session.phase, Phase::CatalogFrozen) {
            return vec![warn(format!(
                "{device} asked for a diff while {}",
                describe(&session.phase)
            ))];
        }

        let paths: Vec<DevicePath> = session
            .catalog
            .entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        session.begin_classifying();

        let staging = self.begin(Pending::ListStaging {
            device: device.clone(),
        });
        let index = self.begin(Pending::LookupImported {
            device: device.clone(),
        });

        vec![
            Effect::Store {
                op: staging,
                request: StoreRequest::ListStagingEntries {
                    device: device.clone(),
                },
            },
            Effect::Store {
                op: index,
                request: StoreRequest::LookupDeviceFiles {
                    device: device.clone(),
                    paths,
                },
            },
        ]
    }

    fn seed_file_ids(&mut self, highest: Option<FileId>) -> Vec<Effect> {
        self.files = Some(FileIds::starting_above(highest));

        let waiting: Vec<DeviceId> = std::mem::take(&mut self.awaiting_ids).into_iter().collect();
        let mut effects = Vec::new();
        for device in waiting {
            effects.extend(self.try_classify(&device));
        }
        effects
    }

    fn record_input(&mut self, device: &DeviceId, input: Input) -> Vec<Effect> {
        let Some(session) = self.sessions.get_mut(device) else {
            return vec![unknown_session(device)];
        };
        if !session.record_input(input) {
            return Vec::new();
        }
        self.try_classify(device)
    }

    /// Classifies the catalog once both store answers and the identifier seed are in hand.
    fn try_classify(&mut self, device: &DeviceId) -> Vec<Effect> {
        let Some(files) = self.files.as_mut() else {
            self.awaiting_ids.insert(device.clone());
            return Vec::new();
        };
        let Some(session) = self.sessions.get_mut(device) else {
            return vec![unknown_session(device)];
        };
        let Some((staged, imported)) = session.take_inputs() else {
            return Vec::new();
        };

        let durable_digests = &self.durable_digests;
        let resumable: BTreeSet<FileId> = staged
            .iter()
            .filter(|entry| {
                durable_digests
                    .get(&entry.file)
                    .is_some_and(|digest| digest.bytes() == entry.durable_bytes)
            })
            .map(|entry| entry.file)
            .collect();

        let classification = classify(&session.catalog, &staged, &imported, &resumable, files);
        let superseded = classification.superseded.clone();
        session.plan(classification);

        let mut effects = self.supersede(&superseded);
        let op = self.begin(Pending::FreeSpace {
            device: device.clone(),
        });
        effects.push(Effect::MeasureFreeSpace { op });
        effects
    }

    /// Drops the manifest rows and files of staged versions the phone has since replaced.
    /// The row goes first, so a crash can never leave a row pointing at a file that is gone.
    fn supersede(&mut self, files: &[FileId]) -> Vec<Effect> {
        let mut effects = Vec::new();
        for file in files {
            self.durable_digests.remove(file);
            let op = self.begin(Pending::DropSuperseded { file: *file });
            effects.push(Effect::Store {
                op,
                request: StoreRequest::DropStagingEntry { file: *file },
            });
        }
        effects
    }

    fn free_space_measured(&mut self, op: OpId, available_bytes: u64) -> Vec<Effect> {
        if !matches!(self.pending.get(&op), Some(Pending::FreeSpace { .. })) {
            return vec![warn(format!(
                "free space was reported for {op:?}, which did not ask"
            ))];
        }
        let Some(Pending::FreeSpace { device }) = self.pending.remove(&op) else {
            return vec![warn(format!(
                "free space was reported for {op:?}, which did not ask"
            ))];
        };
        let Some(session) = self.sessions.get_mut(&device) else {
            return vec![unknown_session(&device)];
        };

        let required_bytes = session.to_send_bytes();
        if available_bytes < required_bytes {
            session.phase = Phase::Rejected;
            return vec![Effect::RejectSession {
                device,
                reason: RejectReason::NotEnoughSpace {
                    required_bytes,
                    available_bytes,
                },
            }];
        }

        session.phase = Phase::Transferring;
        let to_send = session
            .in_order()
            .map(|send| ToSend {
                file: send.file,
                path: send.path.clone(),
                resume_offset: send.resume_offset,
            })
            .collect();

        vec![Effect::SendDiff {
            device,
            to_send,
            summary: session.summary,
        }]
    }

    // ---- receiving a file ----------------------------------------------------------------

    fn upload_opened(
        &mut self,
        device: &DeviceId,
        file: FileId,
        path: &DevicePath,
        offset: u64,
    ) -> Vec<Effect> {
        let Some(session) = self.sessions.get(device) else {
            return vec![unknown_session(device)];
        };
        if !matches!(session.phase, Phase::Transferring) {
            return vec![warn(format!(
                "{device} opened an upload while {}",
                describe(&session.phase)
            ))];
        }
        let Some(planned) = session.sends.get(&file).cloned() else {
            return self.refuse_upload(device, file, format!("{file:?} was not on the diff"));
        };
        if planned.path != *path {
            return self.refuse_upload(
                device,
                file,
                format!(
                    "{file:?} was assigned to {} and not to {path}",
                    planned.path
                ),
            );
        }
        // The watermark is the desktop's own record of what durably arrived, so a phone that
        // offers a different offset is refused rather than believed. SPEC.md §6.
        if offset != planned.resume_offset {
            return self.refuse_upload(
                device,
                file,
                format!(
                    "{file:?} was offered from {} but the phone sent from {offset}",
                    planned.resume_offset
                ),
            );
        }

        let digest = match self.prefix_digest(file, offset) {
            Some(digest) => digest,
            None => {
                return self.refuse_upload(
                    device,
                    file,
                    format!("{file:?} has no digest covering its first {offset} bytes"),
                );
            }
        };

        self.open_upload(device, file, &planned, offset, digest)
    }

    /// The digest covering the prefix a resumed transfer starts after. A transfer from zero
    /// needs nothing; one that resumes needs the accumulation the desktop kept.
    fn prefix_digest(&self, file: FileId, offset: u64) -> Option<RunningDigest> {
        if offset == 0 {
            return Some(RunningDigest::new());
        }
        self.durable_digests
            .get(&file)
            .filter(|digest| digest.bytes() == offset)
            .cloned()
    }

    fn open_upload(
        &mut self,
        device: &DeviceId,
        file: FileId,
        planned: &PlannedSend,
        offset: u64,
        digest: RunningDigest,
    ) -> Vec<Effect> {
        let upload = Upload {
            size: planned.size,
            received: offset,
            durable: offset,
            unsynced: 0,
            digest,
            state: UploadState::Receiving,
        };
        if let Some(session) = self.sessions.get_mut(device) {
            session.uploads.insert(file, upload);
        }

        // A resumed transfer already has its manifest row and its file.
        if offset > 0 {
            return Vec::new();
        }

        // The row is created before the file, so a crash can leave a row without a file but
        // never a file without a row. Commit drops the first; nothing would find the second.
        let entry = StagingEntry {
            file,
            path: planned.path.clone(),
            size: planned.size,
            mtime: planned.mtime,
            durable_bytes: 0,
            digest: None,
        };
        let begin = self.begin(Pending::BeginEntry {
            device: device.clone(),
            file,
        });
        let create = self.begin(Pending::CreateFile {
            device: device.clone(),
            file,
        });
        vec![
            Effect::Store {
                op: begin,
                request: StoreRequest::BeginStagingEntry {
                    device: device.clone(),
                    entry,
                },
            },
            Effect::CreateStagingFile {
                op: create,
                device: device.clone(),
                file,
            },
        ]
    }

    fn refuse_upload(&mut self, device: &DeviceId, file: FileId, reason: String) -> Vec<Effect> {
        vec![
            Effect::Log {
                level: LogLevel::Warn,
                message: reason,
            },
            Effect::SendUploadResult {
                device: device.clone(),
                file,
                outcome: UploadOutcome::WriteFailed,
            },
        ]
    }

    fn chunk_arrived(
        &mut self,
        device: &DeviceId,
        file: FileId,
        offset: u64,
        data: Vec<u8>,
    ) -> Vec<Effect> {
        match self.inspect_chunk(device, file, offset, data.len() as u64) {
            Ok(()) => self.accept_chunk(device, file, offset, data),
            Err(ChunkRefusal::UnknownSession) => vec![unknown_session(device)],
            Err(ChunkRefusal::UnknownUpload) => {
                vec![warn(format!("a chunk arrived for unopened {file:?}"))]
            }
            Err(ChunkRefusal::NotReceiving) => {
                vec![warn(format!("a chunk arrived for closed {file:?}"))]
            }
            Err(ChunkRefusal::OutOfOrder { expected }) => self.fail_upload(
                device,
                file,
                format!("{file:?} expected byte {expected} but the phone sent {offset}"),
            ),
            Err(ChunkRefusal::PastEnd { size }) => self.fail_upload(
                device,
                file,
                format!("{file:?} was declared as {size} bytes and the phone sent more"),
            ),
        }
    }

    fn inspect_chunk(
        &self,
        device: &DeviceId,
        file: FileId,
        offset: u64,
        length: u64,
    ) -> Result<(), ChunkRefusal> {
        let session = self
            .sessions
            .get(device)
            .ok_or(ChunkRefusal::UnknownSession)?;
        let upload = session
            .uploads
            .get(&file)
            .ok_or(ChunkRefusal::UnknownUpload)?;

        if upload.state != UploadState::Receiving {
            return Err(ChunkRefusal::NotReceiving);
        }
        if offset != upload.received {
            return Err(ChunkRefusal::OutOfOrder {
                expected: upload.received,
            });
        }
        // A peer is not trusted to stop at the size it declared. SPEC.md §12 aside, an
        // unbounded stream is an unbounded staging file.
        if offset.saturating_add(length) > upload.size {
            return Err(ChunkRefusal::PastEnd { size: upload.size });
        }
        Ok(())
    }

    fn accept_chunk(
        &mut self,
        device: &DeviceId,
        file: FileId,
        offset: u64,
        data: Vec<u8>,
    ) -> Vec<Effect> {
        let length = data.len() as u64;
        let due = {
            let Some(upload) = self.upload_mut(device, file) else {
                return Vec::new();
            };
            upload.digest.update(&data);
            upload.received += length;
            upload.unsynced += length;
            upload.unsynced >= SYNC_INTERVAL_BYTES
        };

        let op = self.begin(Pending::WriteChunk {
            device: device.clone(),
            file,
        });
        let mut effects = vec![Effect::WriteChunk {
            op,
            file,
            offset,
            data,
        }];
        if due {
            effects.extend(self.sync_partial(device, file, AfterWatermark::Continue));
        }
        effects
    }

    fn upload_closed(&mut self, device: &DeviceId, file: FileId, digest: Sha256) -> Vec<Effect> {
        let Some(upload) = self.upload_mut(device, file) else {
            return vec![warn(format!("{file:?} was closed but never opened"))];
        };
        if upload.state != UploadState::Receiving {
            return vec![warn(format!("{file:?} was closed twice"))];
        }
        upload.state = UploadState::Closing { claimed: digest };

        // SPEC.md §6.5 fsyncs before it verifies, so a file that passes is already durable.
        self.sync_partial(device, file, AfterWatermark::Verify)
    }

    fn upload_aborted(&mut self, device: &DeviceId, file: FileId) -> Vec<Effect> {
        let Some(upload) = self.upload_mut(device, file) else {
            return vec![warn(format!("{file:?} was aborted but never opened"))];
        };
        if upload.state != UploadState::Receiving {
            return Vec::new();
        }
        upload.state = UploadState::Closed;
        self.sync_partial(device, file, AfterWatermark::Stop)
    }

    fn sync_partial(
        &mut self,
        device: &DeviceId,
        file: FileId,
        then: AfterWatermark,
    ) -> Vec<Effect> {
        let Some(upload) = self.upload_mut(device, file) else {
            return Vec::new();
        };
        upload.unsynced = 0;
        let sync = Sync {
            device: device.clone(),
            file,
            through: upload.received,
            snapshot: upload.digest.clone(),
            then,
        };
        let op = self.begin(Pending::SyncFile(sync));
        vec![Effect::SyncFile { op, file }]
    }

    /// The watermark may only move once the sync behind it has completed. `SPEC.md` §7.6.
    fn file_synced(&mut self, sync: Sync) -> Vec<Effect> {
        let file = sync.file;
        let durable_bytes = sync.through;
        let op = self.begin(Pending::AdvanceWatermark(sync));
        vec![Effect::Store {
            op,
            request: StoreRequest::AdvanceWatermark {
                file,
                durable_bytes,
            },
        }]
    }

    fn watermark_advanced(&mut self, sync: Sync) -> Vec<Effect> {
        self.durable_digests.insert(sync.file, sync.snapshot);
        if let Some(upload) = self.upload_mut(&sync.device, sync.file) {
            upload.durable = sync.through;
        }
        match sync.then {
            AfterWatermark::Continue | AfterWatermark::Stop => Vec::new(),
            AfterWatermark::Verify => self.verify(&sync.device, sync.file),
        }
    }

    /// Compares what arrived against what the phone said it sent. `SPEC.md` §6.5.
    fn verify(&mut self, device: &DeviceId, file: FileId) -> Vec<Effect> {
        let Some(upload) = self.upload(device, file) else {
            return Vec::new();
        };
        let UploadState::Closing { claimed } = upload.state else {
            return vec![warn(format!("{file:?} was verified twice"))];
        };
        let computed = upload.digest.peek();

        if computed != claimed {
            return self.reject_content(device, file);
        }

        let op = self.begin(Pending::FinalizeFile {
            device: device.clone(),
            file,
            digest: computed,
        });
        vec![Effect::FinalizeStagingFile { op, file }]
    }

    /// A digest that did not match costs the partial and buys one full re-transfer.
    /// `SPEC.md` §6.5.
    fn reject_content(&mut self, device: &DeviceId, file: FileId) -> Vec<Effect> {
        self.durable_digests.remove(&file);
        if let Some(upload) = self.upload_mut(device, file) {
            upload.state = UploadState::Closed;
        }

        let mut retrying = false;
        if let Some(session) = self.sessions.get_mut(device)
            && let Some(planned) = session.sends.get_mut(&file)
        {
            planned.attempts += 1;
            planned.resume_offset = 0;
            retrying = planned.attempts < MAX_ATTEMPTS;
        }
        if !retrying && let Some(session) = self.sessions.get_mut(device) {
            session.sends.remove(&file);
        }

        let op = self.begin(Pending::DropMismatched {
            device: device.clone(),
            file,
        });
        vec![
            Effect::Log {
                level: LogLevel::Warn,
                message: format!(
                    "{file:?} did not match the digest the phone stated{}",
                    if retrying {
                        ", retransferring once"
                    } else {
                        ", skipping it"
                    }
                ),
            },
            Effect::Store {
                op,
                request: StoreRequest::DropStagingEntry { file },
            },
        ]
    }

    fn staging_file_finalized(
        &mut self,
        device: &DeviceId,
        file: FileId,
        digest: Sha256,
    ) -> Vec<Effect> {
        let op = self.begin(Pending::MarkVerified {
            device: device.clone(),
            file,
        });
        vec![Effect::Store {
            op,
            request: StoreRequest::MarkStagingEntryVerified { file, digest },
        }]
    }

    fn upload_verified(&mut self, device: &DeviceId, file: FileId) -> Vec<Effect> {
        self.durable_digests.remove(&file);

        // A file can finish verifying after its phone has gone. The staging entry is what
        // carries that forward to the commit, so the only thing lost is the message.
        let Some(upload) = self.upload_mut(device, file) else {
            return vec![info(format!("{file:?} verified after {device} went away"))];
        };
        upload.state = UploadState::Closed;
        let durable_bytes = upload.durable;

        if let Some(session) = self.sessions.get_mut(device) {
            session.sends.remove(&file);
        }

        vec![Effect::SendUploadResult {
            device: device.clone(),
            file,
            outcome: UploadOutcome::Verified { durable_bytes },
        }]
    }

    /// Ends a transfer the desktop could not take. The partial and its manifest row stay, so
    /// the next diff resumes rather than starting again. `SPEC.md` §4.
    fn fail_upload(&mut self, device: &DeviceId, file: FileId, reason: String) -> Vec<Effect> {
        if let Some(upload) = self.upload_mut(device, file) {
            upload.state = UploadState::Closed;
        }
        vec![
            Effect::Log {
                level: LogLevel::Error,
                message: reason,
            },
            Effect::SendUploadResult {
                device: device.clone(),
                file,
                outcome: UploadOutcome::WriteFailed,
            },
        ]
    }

    fn upload(&self, device: &DeviceId, file: FileId) -> Option<&Upload> {
        self.sessions.get(device)?.uploads.get(&file)
    }

    fn upload_mut(&mut self, device: &DeviceId, file: FileId) -> Option<&mut Upload> {
        self.sessions.get_mut(device)?.uploads.get_mut(&file)
    }

    // ---- completions ---------------------------------------------------------------------

    fn storage_completed(
        &mut self,
        op: OpId,
        result: Result<StorageOutcome, StorageError>,
    ) -> Vec<Effect> {
        let Some(pending) = self.pending.remove(&op) else {
            return vec![warn(format!("{op:?} completed but was never asked for"))];
        };
        match result {
            Ok(_) => self.storage_succeeded(pending),
            Err(error) => self.storage_failed(pending, &error),
        }
    }

    fn storage_succeeded(&mut self, pending: Pending) -> Vec<Effect> {
        match pending {
            Pending::CreateFile { .. } | Pending::WriteChunk { .. } => Vec::new(),
            Pending::SyncFile(sync) => self.file_synced(sync),
            Pending::RemoveSuperseded { .. } | Pending::RemoveMismatched { .. } => Vec::new(),
            Pending::FinalizeFile {
                device,
                file,
                digest,
            } => self.staging_file_finalized(&device, file, digest),
            other @ (Pending::SeedFileIds
            | Pending::ListStaging { .. }
            | Pending::LookupImported { .. }
            | Pending::FreeSpace { .. }
            | Pending::DropSuperseded { .. }
            | Pending::BeginEntry { .. }
            | Pending::AdvanceWatermark(_)
            | Pending::MarkVerified { .. }
            | Pending::DropMismatched { .. }) => {
                vec![warn(format!("{other:?} was answered by storage"))]
            }
        }
    }

    fn storage_failed(&mut self, pending: Pending, error: &StorageError) -> Vec<Effect> {
        match pending {
            Pending::CreateFile { device, file }
            | Pending::WriteChunk { device, file }
            | Pending::FinalizeFile { device, file, .. } => self.fail_upload(
                &device,
                file,
                format!("{file:?} could not be stored: {error}"),
            ),
            Pending::SyncFile(sync) => self.fail_upload(
                &sync.device,
                sync.file,
                format!("{:?} could not be made durable: {error}", sync.file),
            ),
            Pending::RemoveSuperseded { file } | Pending::RemoveMismatched { file } => {
                vec![warn(format!(
                    "the staged file for {file:?} was not removed: {error}"
                ))]
            }
            other @ (Pending::SeedFileIds
            | Pending::ListStaging { .. }
            | Pending::LookupImported { .. }
            | Pending::FreeSpace { .. }
            | Pending::DropSuperseded { .. }
            | Pending::BeginEntry { .. }
            | Pending::AdvanceWatermark(_)
            | Pending::MarkVerified { .. }
            | Pending::DropMismatched { .. }) => {
                vec![error_log(format!("{other:?} failed in storage: {error}"))]
            }
        }
    }

    fn store_completed(
        &mut self,
        op: OpId,
        result: Result<StoreResponse, StoreError>,
    ) -> Vec<Effect> {
        let Some(pending) = self.pending.remove(&op) else {
            return vec![warn(format!("{op:?} completed but was never asked for"))];
        };
        match result {
            Ok(response) => self.store_succeeded(pending, response),
            Err(error) => self.store_failed(pending, &error),
        }
    }

    fn store_succeeded(&mut self, pending: Pending, response: StoreResponse) -> Vec<Effect> {
        match pending {
            Pending::SeedFileIds => match response {
                StoreResponse::HighestStagingFileId(highest) => self.seed_file_ids(highest),
                other => vec![mismatched(&other)],
            },
            Pending::ListStaging { device } => match response {
                StoreResponse::StagingEntries(entries) => {
                    self.record_input(&device, Input::Staged(entries))
                }
                other => vec![mismatched(&other)],
            },
            Pending::LookupImported { device } => match response {
                StoreResponse::DeviceFiles(rows) => {
                    self.record_input(&device, Input::Imported(rows))
                }
                other => vec![mismatched(&other)],
            },
            Pending::DropSuperseded { file } => {
                let op = self.begin(Pending::RemoveSuperseded { file });
                vec![Effect::RemoveStagingFile { op, file }]
            }
            Pending::DropMismatched { device, file } => {
                let op = self.begin(Pending::RemoveMismatched { file });
                vec![
                    Effect::RemoveStagingFile { op, file },
                    Effect::SendUploadResult {
                        device,
                        file,
                        outcome: UploadOutcome::HashMismatch,
                    },
                ]
            }
            Pending::AdvanceWatermark(sync) => self.watermark_advanced(sync),
            Pending::MarkVerified { device, file } => self.upload_verified(&device, file),
            Pending::BeginEntry { .. } => Vec::new(),
            other @ (Pending::CreateFile { .. }
            | Pending::WriteChunk { .. }
            | Pending::SyncFile(_)
            | Pending::FinalizeFile { .. }
            | Pending::RemoveSuperseded { .. }
            | Pending::RemoveMismatched { .. }
            | Pending::FreeSpace { .. }) => {
                vec![warn(format!("{other:?} was answered by the store"))]
            }
        }
    }

    fn store_failed(&mut self, pending: Pending, error: &StoreError) -> Vec<Effect> {
        match pending {
            Pending::BeginEntry { device, file } | Pending::MarkVerified { device, file } => self
                .fail_upload(
                    &device,
                    file,
                    format!("the manifest would not record {file:?}: {error}"),
                ),
            Pending::AdvanceWatermark(sync) => self.fail_upload(
                &sync.device,
                sync.file,
                format!("the watermark for {:?} would not move: {error}", sync.file),
            ),
            other @ (Pending::CreateFile { .. }
            | Pending::WriteChunk { .. }
            | Pending::SyncFile(_)
            | Pending::FinalizeFile { .. }
            | Pending::RemoveSuperseded { .. }
            | Pending::RemoveMismatched { .. }
            | Pending::FreeSpace { .. }
            | Pending::SeedFileIds
            | Pending::ListStaging { .. }
            | Pending::LookupImported { .. }
            | Pending::DropSuperseded { .. }
            | Pending::DropMismatched { .. }) => {
                vec![error_log(format!("{other:?} failed in the store: {error}"))]
            }
        }
    }
}

fn describe(phase: &Phase) -> &'static str {
    match phase {
        Phase::AwaitingCatalog => "waiting for its catalog",
        Phase::CatalogFrozen => "holding a frozen catalog",
        Phase::Classifying(_) => "being classified",
        Phase::MeasuringSpace => "waiting on the free-space check",
        Phase::Transferring => "transferring",
        Phase::Rejected => "rejected",
    }
}

fn unknown_session(device: &DeviceId) -> Effect {
    warn(format!("{device} has no session"))
}

fn mismatched(response: &StoreResponse) -> Effect {
    error_log(format!(
        "the store answered with {response:?}, which does not fit the request"
    ))
}

fn warn(message: String) -> Effect {
    Effect::Log {
        level: LogLevel::Warn,
        message,
    }
}

fn info(message: String) -> Effect {
    Effect::Log {
        level: LogLevel::Info,
        message,
    }
}

fn error_log(message: String) -> Effect {
    Effect::Log {
        level: LogLevel::Error,
        message,
    }
}
