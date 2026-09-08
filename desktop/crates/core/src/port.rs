//! Interfaces the shell implements and the simulator replaces.
//!
//! The core does not call these. It emits effects, and whatever drives the core turns those
//! effects into port calls. Both implementations answer the same way, which is what lets a
//! crash be simulated instead of staged.

use crate::id::{DeviceId, FileId, OpId, Sha256, Timestamp, VaultName};
use crate::store::{StoreError, StoreRequest, StoreResponse};

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum StorageError {
    #[error("file not found")]
    NotFound,

    #[error("no space left")]
    NoSpace,

    #[error("input or output failed: {0}")]
    Failed(String),
}

/// Facts about a vault file, read when deciding whether a photo may be deleted from a phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VaultFileFacts {
    pub size: u64,
}

pub trait FileOps {
    fn create_staging_file(&self, device: &DeviceId, file: FileId) -> Result<(), StorageError>;

    fn write_at(&self, file: FileId, offset: u64, data: &[u8]) -> Result<(), StorageError>;

    /// Makes this file's bytes and length durable.
    fn sync_file(&self, file: FileId) -> Result<(), StorageError>;

    /// Makes the directory's entries durable, so a rename survives a power loss.
    fn sync_directory(&self, directory: &crate::effect::Directory) -> Result<(), StorageError>;

    fn truncate(&self, file: FileId, length: u64) -> Result<(), StorageError>;

    fn rename_into_vault(&self, file: FileId, name: &VaultName) -> Result<(), StorageError>;

    fn remove_staging_file(&self, file: FileId) -> Result<(), StorageError>;

    fn clear_staging_directory(&self, device: &DeviceId) -> Result<(), StorageError>;

    fn stat_vault_file(&self, name: &VaultName) -> Result<Option<VaultFileFacts>, StorageError>;
}

pub trait Store {
    fn run(&self, request: StoreRequest) -> Result<StoreResponse, StoreError>;
}

pub trait Clock {
    fn now(&self) -> Timestamp;
}

/// Source of the staging identifiers and collision suffixes. Seeded in the simulator so a
/// failing run replays exactly.
pub trait Rng {
    fn next_u64(&self) -> u64;
}

pub trait Env {
    fn free_space(&self) -> Result<u64, StorageError>;
}

/// Computes digests of content the desktop already holds. Used during recovery, where a
/// streamed digest is not available.
pub trait Digest {
    fn digest_staging_file(&self, file: FileId) -> Result<Sha256, StorageError>;
}

/// Delivers the outcome of an effect back to whatever drives the core.
pub trait Completions {
    fn storage_completed(&self, op: OpId, result: Result<(), StorageError>);
    fn store_completed(&self, op: OpId, result: Result<StoreResponse, StoreError>);
}
