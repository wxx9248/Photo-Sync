//! An independent account of what the desktop should be holding.
//!
//! The model consumes the same events as the core and computes the expected vault, index, and
//! staging state on its own. The simulator compares the two after every commit and every
//! recovery, so a disagreement is reported as a difference in state rather than as a failed
//! assertion.
//!
//! Scope today is the state itself. The transition function arrives with the simulator in
//! milestone M2, described in `docs/ROADMAP.md`.

use std::collections::{BTreeMap, BTreeSet};

use photo_sync_core::id::{DeviceId, DevicePath, Sha256, VaultName};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpectedState {
    pub vault: BTreeMap<VaultName, Sha256>,
    pub content: BTreeSet<Sha256>,
    pub device_files: BTreeMap<(DeviceId, DevicePath), ExpectedDeviceFile>,
    pub staging: BTreeMap<DeviceId, Vec<ExpectedStagedFile>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedDeviceFile {
    pub digest: Sha256,
    pub vault_name: VaultName,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedStagedFile {
    pub path: DevicePath,
    pub durable_bytes: u64,
    pub digest: Option<Sha256>,
}
