//! Requests the core makes of durable storage, expressed as intent rather than as SQL.
//!
//! The core states what it needs recorded or looked up. The shell decides which database and
//! which statements answer that, which keeps schema decisions out of the logic and lets the
//! simulator answer the same requests from memory.

use crate::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreRequest {
    /// Everything staging holds for a device, used by the diff and by startup recovery.
    ListStagingEntries {
        device: DeviceId,
    },

    /// The largest staging identifier the manifest currently holds, over every device.
    ///
    /// Read once at startup. Identifiers have to be unique across devices, because the
    /// requests that name a staged file, such as `AdvanceWatermark`, carry the identifier and
    /// not the device, and so does every effect that touches the file. Continuing above the
    /// highest surviving row is what keeps a fresh identifier from colliding with a partial
    /// left behind by a device that has not connected yet.
    HighestStagingFileId,

    BeginStagingEntry {
        device: DeviceId,
        entry: StagingEntry,
    },

    /// Records how much of a partial file is durable. Only ever called after the file's own
    /// sync completed.
    AdvanceWatermark {
        file: FileId,
        durable_bytes: u64,
    },

    MarkStagingEntryVerified {
        file: FileId,
        digest: Sha256,
    },

    DropStagingEntry {
        file: FileId,
    },

    /// Writes the commit plan and seals it in one transaction, so a partly written plan cannot
    /// exist after a crash.
    SealCommitPlan {
        device: DeviceId,
        plan: Vec<PlanEntry>,
    },

    LoadCommitPlan {
        device: DeviceId,
    },

    /// Marks a group of plan entries as performed. Ordered after the directory syncs for those
    /// entries, never before.
    MarkPlanEntriesDone {
        device: DeviceId,
        files: Vec<FileId>,
    },

    ClearCommitPlan {
        device: DeviceId,
    },

    ClearStagingManifest {
        device: DeviceId,
    },

    /// Looks up which digests the index already holds, and under which vault name.
    LookupContent {
        digests: Vec<Sha256>,
    },

    /// Rows for the paths in a catalog, used to classify entries as already imported.
    LookupDeviceFiles {
        device: DeviceId,
        paths: Vec<DevicePath>,
    },

    /// Inserts the rows for a committed batch. Runs before the commit lock is released so the
    /// next commit sees this batch's content.
    InsertCommittedBatch {
        contents: Vec<ContentRow>,
        device_files: Vec<DeviceFileRow>,
    },

    /// Drops a row whose file the phone keeps reporting as changed, so the next diff sends it.
    ForgetDeviceFile {
        device: DeviceId,
        path: DevicePath,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreResponse {
    Done,
    HighestStagingFileId(Option<FileId>),
    StagingEntries(Vec<StagingEntry>),
    CommitPlan(Option<Vec<PlanEntry>>),
    Content(Vec<ContentRow>),
    DeviceFiles(Vec<DeviceFileRow>),
}

/// One file in a device's staging area. An entry exists from the moment a transfer starts, so
/// a partial file is always described by a row even after a crash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagingEntry {
    pub file: FileId,
    pub path: DevicePath,
    pub size: u64,
    pub mtime: Timestamp,

    /// Bytes proven durable by a completed sync. Recovery truncates the partial file to this
    /// length, because the file's own size can run ahead of what survived a power loss.
    pub durable_bytes: u64,

    /// Set once the streamed digest matched what the phone stated.
    pub digest: Option<Sha256>,
}

impl StagingEntry {
    pub fn is_verified(&self) -> bool {
        self.digest.is_some()
    }
}

/// One decision in a sealed commit plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEntry {
    pub file: FileId,
    pub action: PlanAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanAction {
    /// Move the staged file into the vault under this name.
    Import { name: VaultName },

    /// The index already holds this content, so the staged file is removed and the device row
    /// inherits the vault name the content already has.
    Duplicate { name: VaultName },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRow {
    pub digest: Sha256,
    pub vault_name: VaultName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceFileRow {
    pub device: DeviceId,
    pub path: DevicePath,
    pub size: u64,
    pub mtime: Timestamp,
    pub digest: Sha256,
    pub vault_name: VaultName,
    pub committed_at: Timestamp,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("storage is full")]
    NoSpace,

    #[error("database failed: {0}")]
    Failed(String),
}
