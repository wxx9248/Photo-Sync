//! Everything the core can ask the shell to do.
//!
//! This enum is the port surface. A side effect that is not a variant here is invisible to the
//! simulator, the model, and the fault injector, so new kinds of work belong in this file
//! before they belong anywhere else.

use crate::id::{DeviceId, DevicePath, FileId, OpId, Sha256, Timestamp, VaultName};
use crate::store::StoreRequest;

#[derive(Clone, Debug)]
pub enum Effect {
    CreateStagingFile {
        op: OpId,
        device: DeviceId,
        file: FileId,
    },

    WriteChunk {
        op: OpId,
        file: FileId,
        offset: u64,
        data: Vec<u8>,
    },

    /// Makes the written bytes durable. The watermark may only advance after this completes.
    SyncFile {
        op: OpId,
        file: FileId,
    },

    /// Makes recent directory entries durable. Required before any done-mark that depends on a
    /// rename, because a done-mark surviving a power loss must imply its rename survived too.
    SyncDirectory {
        op: OpId,
        directory: Directory,
    },

    /// Cuts a partial file back to the last durable byte during startup recovery.
    TruncateFile {
        op: OpId,
        file: FileId,
        length: u64,
    },

    /// Reads the wall-clock readings a vault name may be built from, in the order
    /// `SPEC.md` §7.2 lists them. The shell extracts the capture time and localizes both it
    /// and the modification time handed to it, because converting an instant into a wall
    /// clock needs a timezone and the core may not read one.
    ///
    /// Asked while the file is still in staging, so a commit never re-reads file bytes.
    ReadNameSources {
        op: OpId,
        file: FileId,
        mtime: Timestamp,
    },

    /// Renames a verified staging file from `<id>.part` to `<id>`. The suffix is what startup
    /// recovery uses to tell a partial apart from a file whose digest already matched, so
    /// stripping it is the moment a transfer stops being resumable and becomes committable.
    FinalizeStagingFile {
        op: OpId,
        file: FileId,
    },

    RenameIntoVault {
        op: OpId,
        file: FileId,
        name: VaultName,
    },

    RemoveStagingFile {
        op: OpId,
        file: FileId,
    },

    ClearStagingDirectory {
        op: OpId,
        device: DeviceId,
    },

    /// Checks that a vault copy is still where the index says it is. A file the user curated
    /// away is never offered for deletion.
    StatVaultFile {
        op: OpId,
        name: VaultName,
    },

    Store {
        op: OpId,
        request: StoreRequest,
    },

    MeasureFreeSpace {
        op: OpId,
    },

    SetTimer {
        op: OpId,
        at: Timestamp,
    },

    SendDiff {
        device: DeviceId,
        to_send: Vec<ToSend>,
        summary: DiffSummary,
    },

    SendUploadResult {
        device: DeviceId,
        file: FileId,
        outcome: UploadOutcome,
    },

    SendCandidates {
        device: DeviceId,
        candidates: Vec<DeletionCandidate>,
        commit: CommitSummary,
    },

    SendSessionSummary {
        device: DeviceId,
        summary: SessionSummary,
    },

    RejectSession {
        device: DeviceId,
        reason: RejectReason,
    },

    Log {
        level: LogLevel,
        message: String,
    },

    NotifyUi {
        update: UiUpdate,
    },
}

/// Directories whose entries the core needs made durable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Directory {
    Vault,
    Staging { device: DeviceId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToSend {
    pub file: FileId,
    pub path: DevicePath,
    pub resume_offset: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub to_send_count: u64,
    pub to_send_bytes: u64,
    pub already_imported: u64,
    pub already_staged: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadOutcome {
    Verified { durable_bytes: u64 },
    HashMismatch,
    ChangedOnPhone,
    WriteFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletionCandidate {
    pub path: DevicePath,
    pub size: u64,
    pub mtime: Timestamp,
    pub expected: Sha256,
    pub origin: CandidateOrigin,
}

/// Decides which check the phone runs before deleting. Content committed during this session
/// was verified against a digest minutes ago, so size and mtime carry that guarantee. Anything
/// older is re-hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateOrigin {
    ThisTransfer,
    Earlier,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitSummary {
    pub imported: u64,
    pub duplicates: u64,
    pub bytes_committed: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionSummary {
    pub sent: u64,
    pub skipped: u64,
    pub failed: u64,
    pub deleted: u64,
    pub kept: u64,
    pub bytes_freed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    UnsupportedProtocolVersion {
        supported: u32,
    },
    NotEnoughSpace {
        required_bytes: u64,
        available_bytes: u64,
    },
    CommitInProgress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiUpdate {
    DeviceProgress {
        device: DeviceId,
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
    },
    CommitStarted {
        device: DeviceId,
    },
    CommitFinished {
        device: DeviceId,
        summary: CommitSummary,
    },
    Error {
        device: DeviceId,
        message: String,
    },
}
