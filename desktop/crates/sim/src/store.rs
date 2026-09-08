//! The manifest, the index, and the commit write-log, answered from memory.
//!
//! The desktop states what it needs recorded rather than how, so this answers the same
//! requests SQLite will. Keying the device rows by device and path is not a convenience: it
//! is what makes a re-import replace the row it supersedes, as `SPEC.md` §7.5 requires.

use std::collections::{BTreeMap, BTreeSet};

use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, VaultName};
use photo_sync_core::store::{
    ContentRow, DeviceFileRow, PlanEntry, StagingEntry, StoreRequest, StoreResponse,
};

#[derive(Debug, Default)]
pub struct Store {
    manifest: BTreeMap<FileId, (DeviceId, StagingEntry)>,
    content: BTreeMap<Sha256, VaultName>,
    device_files: BTreeMap<(DeviceId, DevicePath), DeviceFileRow>,
    plans: BTreeMap<DeviceId, Vec<PlanEntry>>,
    done: BTreeMap<DeviceId, BTreeSet<FileId>>,
}

impl Store {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_lines)]
    pub fn run(&mut self, request: &StoreRequest) -> StoreResponse {
        match request {
            StoreRequest::HighestStagingFileId => {
                StoreResponse::HighestStagingFileId(self.manifest.keys().next_back().copied())
            }
            StoreRequest::ListStagingDevices => {
                let mut devices: Vec<DeviceId> = self
                    .manifest
                    .values()
                    .map(|(device, _)| device.clone())
                    .collect();
                devices.dedup();
                StoreResponse::Devices(devices)
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
            StoreRequest::SealCommitPlan { device, plan } => {
                let plan: Vec<PlanEntry> = plan
                    .iter()
                    .map(|entry| PlanEntry {
                        done: false,
                        ..entry.clone()
                    })
                    .collect();
                self.plans.insert(device.clone(), plan);
                self.done.insert(device.clone(), BTreeSet::new());
                StoreResponse::Done
            }
            StoreRequest::LoadCommitPlan { device } => {
                StoreResponse::CommitPlan(self.plans.get(device).cloned())
            }
            StoreRequest::MarkPlanEntriesDone { device, files } => {
                self.done
                    .entry(device.clone())
                    .or_default()
                    .extend(files.iter().copied());
                for entry in self.plans.entry(device.clone()).or_default() {
                    if files.contains(&entry.file) {
                        entry.done = true;
                    }
                }
                StoreResponse::Done
            }
            StoreRequest::ClearCommitPlan { device } => {
                self.plans.remove(device);
                StoreResponse::Done
            }
            StoreRequest::ClearStagingManifest { device } => {
                self.manifest.retain(|_, (owner, _)| owner != device);
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
            StoreRequest::LookupDeviceFiles { device, paths } => StoreResponse::DeviceFiles(
                paths
                    .iter()
                    .filter_map(|path| self.device_files.get(&(device.clone(), path.clone())))
                    .cloned()
                    .collect(),
            ),
            StoreRequest::InsertCommittedBatch {
                contents,
                device_files,
            } => {
                for row in contents {
                    self.content.insert(row.digest, row.vault_name.clone());
                }
                for row in device_files {
                    self.device_files
                        .insert((row.device.clone(), row.path.clone()), row.clone());
                }
                StoreResponse::Done
            }
            StoreRequest::ForgetDeviceFile { device, path } => {
                self.device_files.remove(&(device.clone(), path.clone()));
                StoreResponse::Done
            }
        }
    }

    // ---- what a test says and asks -----------------------------------------------------

    /// Records a photo as imported by an earlier session.
    pub fn remember_import(&mut self, row: DeviceFileRow) {
        self.content.insert(row.digest, row.vault_name.clone());
        self.device_files
            .insert((row.device.clone(), row.path.clone()), row);
    }

    #[must_use]
    pub fn manifest(&self) -> Vec<&StagingEntry> {
        self.manifest.values().map(|(_, entry)| entry).collect()
    }

    #[must_use]
    pub fn staged(&self, file: FileId) -> Option<&StagingEntry> {
        self.manifest.get(&file).map(|(_, entry)| entry)
    }

    #[must_use]
    pub fn content(&self) -> &BTreeMap<Sha256, VaultName> {
        &self.content
    }

    #[must_use]
    pub fn device_files(&self) -> Vec<&DeviceFileRow> {
        self.device_files.values().collect()
    }

    #[must_use]
    pub fn sealed_plan(&self, device: &DeviceId) -> Option<&Vec<PlanEntry>> {
        self.plans.get(device)
    }

    #[must_use]
    pub fn done_marks(&self, device: &DeviceId) -> usize {
        self.done.get(device).map_or(0, BTreeSet::len)
    }
}
