//! Everything that can happen to the core.
//!
//! Events arrive from two places: a phone talking to the shell, and the shell reporting the
//! outcome of an effect it was asked to perform. The core never observes anything else.

use crate::catalog::CatalogEntry;
use crate::id::{DeviceId, DevicePath, FileId, OpId, Sha256, Timestamp};
use crate::naming::{CivilTime, Moment};
use crate::port::StorageError;
use crate::store::{StoreError, StoreResponse};

#[derive(Clone, Debug)]
pub enum Event {
    /// The desktop started. Recovery runs before anything else is accepted.
    Started {
        now: Timestamp,
    },

    /// Time passed. Timers requested by the core are reported this way.
    TimerFired {
        op: OpId,
        now: Timestamp,
    },

    PeerConnected {
        device: DeviceId,
        name: String,
    },
    PeerDisconnected {
        device: DeviceId,
    },

    /// The complete frozen catalog for this session.
    CatalogSubmitted {
        device: DeviceId,
        entries: Vec<CatalogEntry>,
        total_bytes: u64,
    },

    DiffRequested {
        device: DeviceId,
    },

    /// A file transfer began. The offset is what the phone claims to be sending from, and the
    /// core checks it against the watermark rather than trusting it.
    UploadOpened {
        device: DeviceId,
        file: FileId,
        path: DevicePath,
        offset: u64,
    },

    ChunkArrived {
        device: DeviceId,
        file: FileId,
        offset: u64,
        data: Vec<u8>,
    },

    /// The phone finished sending and stated the digest of the whole file.
    UploadClosed {
        device: DeviceId,
        file: FileId,
        digest: Sha256,
    },

    /// The connection carrying a file went away before the trailer arrived.
    UploadAborted {
        device: DeviceId,
        file: FileId,
    },

    FinishRequested {
        device: DeviceId,
    },

    DeletionsReported {
        device: DeviceId,
        outcomes: Vec<DeletionOutcome>,
    },

    /// A person pressed "Commit now" for a device whose phone never came back.
    ManualCommitRequested {
        device: DeviceId,
    },

    StorageOpCompleted {
        op: OpId,
        result: Result<StorageOutcome, StorageError>,
    },

    StoreOpCompleted {
        op: OpId,
        result: Result<StoreResponse, StoreError>,
    },

    FreeSpaceMeasured {
        op: OpId,
        available_bytes: u64,
    },

    ClockRead {
        op: OpId,
        moment: Moment,
    },

    ShutdownRequested,
}

/// What a storage effect produced. Most effects only report success, so this stays small.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageOutcome {
    Done,

    /// Result of checking whether a vault copy is still present, used when nominating a file
    /// for deletion.
    VaultFile {
        present: bool,
        size: u64,
    },

    /// Wall-clock readings a vault name may be built from, in the order of `SPEC.md` §7.2.
    NameSources(Vec<CivilTime>),

    /// Whether a staged file is still where the manifest says it is.
    StagingFile {
        present: bool,
    },

    /// Bytes read back out of a staged file.
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletionOutcome {
    pub path: DevicePath,
    pub result: DeletionResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletionResult {
    Deleted,

    /// The file no longer matched what the desktop verified, so the phone kept it. Repeated
    /// across sessions without a re-import, this points at a stale index row.
    KeptChanged,

    KeptUser,
    Failed,
}
