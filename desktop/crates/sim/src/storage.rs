//! Files, as the desktop would find them.
//!
//! Staging and the vault are held in memory, contents and all, so a digest can be taken from
//! what actually landed rather than from what was meant to. Nothing here fails on its own:
//! faults are injected by the caller, which keeps this a model of storage rather than a model
//! of storage going wrong.
//!
//! Durability is recorded but not yet enforced. `durable` says how much of a file a sync has
//! covered; discarding the rest at a crash is milestone M2, described in `docs/ROADMAP.md`.

use std::collections::BTreeMap;

use photo_sync_core::CivilTime;
use photo_sync_core::id::{DeviceId, FileId, VaultName};

/// One file in a device's staging directory.
#[derive(Clone, Debug)]
pub struct StagedFile {
    pub device: DeviceId,
    pub bytes: Vec<u8>,

    /// Bytes a completed sync has made durable.
    pub durable: u64,

    /// False while the file still carries the `.part` suffix that marks it unverified.
    pub finalized: bool,
}

#[derive(Debug, Default)]
pub struct Storage {
    staging: BTreeMap<FileId, StagedFile>,
    vault: BTreeMap<VaultName, Vec<u8>>,

    /// What the shell would read out of each staged file as its capture time.
    name_sources: BTreeMap<FileId, Vec<CivilTime>>,

    /// What it reads out of a file nothing was said about. A real shell finds a capture time
    /// in most photographs, so answering nothing by default would make the common case the
    /// one no test covers.
    default_name_sources: Vec<CivilTime>,
}

impl Storage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ---- what the desktop asks for ----------------------------------------------------

    pub fn create(&mut self, device: &DeviceId, file: FileId) {
        self.staging.insert(
            file,
            StagedFile {
                device: device.clone(),
                bytes: Vec::new(),
                durable: 0,
                finalized: false,
            },
        );
    }

    pub fn write_at(&mut self, file: FileId, offset: u64, data: &[u8]) {
        let Some(staged) = self.staging.get_mut(&file) else {
            return;
        };
        let at = offset as usize;
        if staged.bytes.len() < at {
            staged.bytes.resize(at, 0);
        }
        staged.bytes.truncate(at);
        staged.bytes.extend_from_slice(data);
    }

    pub fn sync_file(&mut self, file: FileId) {
        if let Some(staged) = self.staging.get_mut(&file) {
            staged.durable = staged.bytes.len() as u64;
        }
    }

    pub fn truncate(&mut self, file: FileId, length: u64) {
        if let Some(staged) = self.staging.get_mut(&file) {
            staged.bytes.truncate(length as usize);
            staged.durable = staged.durable.min(length);
        }
    }

    pub fn finalize(&mut self, file: FileId) {
        if let Some(staged) = self.staging.get_mut(&file) {
            staged.finalized = true;
        }
    }

    pub fn rename_into_vault(&mut self, file: FileId, name: &VaultName) {
        if let Some(staged) = self.staging.remove(&file) {
            self.vault.insert(name.clone(), staged.bytes);
        }
    }

    pub fn remove(&mut self, file: FileId) {
        self.staging.remove(&file);
    }

    pub fn clear_staging(&mut self, device: &DeviceId) {
        self.staging.retain(|_, staged| staged.device != *device);
    }

    #[must_use]
    pub fn stat_vault(&self, name: &VaultName) -> Option<u64> {
        self.vault.get(name).map(|bytes| bytes.len() as u64)
    }

    #[must_use]
    pub fn holds(&self, file: FileId) -> bool {
        self.staging.contains_key(&file)
    }

    #[must_use]
    pub fn name_sources(&self, file: FileId) -> Vec<CivilTime> {
        self.name_sources
            .get(&file)
            .cloned()
            .unwrap_or_else(|| self.default_name_sources.clone())
    }

    // ---- what a test says and asks -----------------------------------------------------

    /// Says what the shell would read out of a file as its capture time.
    pub fn set_name_sources(&mut self, file: FileId, sources: Vec<CivilTime>) {
        self.name_sources.insert(file, sources);
    }

    /// Says what it reads out of every file nothing was said about.
    pub fn set_default_name_sources(&mut self, sources: Vec<CivilTime>) {
        self.default_name_sources = sources;
    }

    /// Puts a file in the vault, as an earlier session would have left it.
    pub fn put_in_vault(&mut self, name: &VaultName, bytes: Vec<u8>) {
        self.vault.insert(name.clone(), bytes);
    }

    /// Takes a file out of the vault, as a person curating it would.
    pub fn curate(&mut self, name: &VaultName) {
        self.vault.remove(name);
    }

    #[must_use]
    pub fn vault(&self) -> &BTreeMap<VaultName, Vec<u8>> {
        &self.vault
    }

    #[must_use]
    pub fn staging(&self) -> &BTreeMap<FileId, StagedFile> {
        &self.staging
    }
}
