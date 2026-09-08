//! The desktop, running on the machine.
//!
//! The same loop the simulator runs, with the simulated halves replaced by the real ones.
//! Every effect the core emits is performed and answered with an event, in the order the
//! effects were emitted, and nothing here decides what should happen next.
//!
//! It is deliberately synchronous. `SPEC.md` §7.3 serializes commits under one lock and the
//! core is a single state machine, so the concurrency that matters is between connections,
//! which is the server's business rather than this loop's.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use photo_sync_core::Desktop;
use photo_sync_core::effect::Effect;
use photo_sync_core::event::{Event, StorageOutcome};
use photo_sync_core::port::StorageError;

use crate::clock;
use crate::storage::Storage;
use crate::store::Store;

/// One desktop: the logic, the files, and the databases.
pub struct Desk {
    desktop: Desktop,
    storage: Storage,
    store: Store,
    vault: PathBuf,
    log: Vec<Effect>,
}

impl Desk {
    /// Opens a vault and its index, creating either if it is not there, and starts the core.
    pub fn open(vault: &Path, index_file: &Path) -> Result<Self, StorageError> {
        let storage = Storage::open(vault)?;
        let store = Store::open(vault, index_file)
            .map_err(|error| StorageError::Failed(error.to_string()))?;

        let mut desk = Self {
            desktop: Desktop::new(),
            storage,
            store,
            vault: vault.to_path_buf(),
            log: Vec::new(),
        };
        let started = clock::now();
        desk.deliver(Event::Started { now: started.at });
        Ok(desk)
    }

    /// Delivers one event and performs everything it leads to.
    pub fn deliver(&mut self, event: Event) {
        let mut queue: VecDeque<Effect> = self.desktop.handle(event).into();
        while let Some(effect) = queue.pop_front() {
            let completion = self.perform(&effect);
            self.log.push(effect);
            if let Some(completion) = completion {
                queue.extend(self.desktop.handle(completion));
            }
        }
    }

    /// What the core has said since this was last asked. The server reads it to learn what to
    /// send each phone.
    pub fn take_log(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.log)
    }

    fn perform(&mut self, effect: &Effect) -> Option<Event> {
        match effect {
            Effect::CreateStagingFile { op, device, file } => Some(storage_result(
                *op,
                self.storage.create_staging_file(device, *file),
            )),
            Effect::WriteChunk {
                op,
                file,
                offset,
                data,
            } => Some(storage_result(
                *op,
                self.storage.write_at(*file, *offset, data),
            )),
            Effect::SyncFile { op, file } => {
                Some(storage_result(*op, self.storage.sync_file(*file)))
            }
            Effect::SyncDirectory { op, directory } => {
                Some(storage_result(*op, self.storage.sync_directory(directory)))
            }
            Effect::TruncateFile { op, file, length } => {
                Some(storage_result(*op, self.storage.truncate(*file, *length)))
            }
            Effect::FinalizeStagingFile { op, file } => Some(storage_result(
                *op,
                self.storage.finalize_staging_file(*file),
            )),
            Effect::ReadNameSources { op, .. } => Some(Event::StorageOpCompleted {
                op: *op,
                // Reading a capture time out of a photograph needs nom-exif and a timezone,
                // neither of which is built. A name falls back to the import time until then.
                result: Ok(StorageOutcome::NameSources(Vec::new())),
            }),
            Effect::RenameIntoVault { op, file, name } => Some(storage_result(
                *op,
                self.storage.rename_into_vault(*file, name),
            )),
            Effect::RemoveStagingFile { op, file } => {
                Some(storage_result(*op, self.storage.remove_staging_file(*file)))
            }
            Effect::ClearStagingDirectory { op, device } => Some(storage_result(
                *op,
                self.storage.clear_staging_directory(device),
            )),
            Effect::StatStagingFile { op, file } => {
                let present = self.storage.stat_staging_file(*file);
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: present.map(|present| StorageOutcome::StagingFile { present }),
                })
            }
            Effect::StatVaultFile { op, name } => {
                let found = self.storage.stat_vault_file(name);
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: found.map(|facts| StorageOutcome::VaultFile {
                        present: facts.is_some(),
                        size: facts.map_or(0, |facts| facts.size),
                    }),
                })
            }
            Effect::Store { op, request } => Some(Event::StoreOpCompleted {
                op: *op,
                result: self.store.run(request),
            }),
            Effect::MeasureFreeSpace { op } => Some(Event::FreeSpaceMeasured {
                op: *op,
                available_bytes: free_space(&self.vault),
            }),
            Effect::ReadClock { op } => Some(Event::ClockRead {
                op: *op,
                moment: clock::now(),
            }),
            // Timers, messages to a phone, logging, and the interface are the server's and
            // the window's business. The loop records them and moves on.
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
}

fn storage_result(op: photo_sync_core::OpId, result: Result<(), StorageError>) -> Event {
    Event::StorageOpCompleted {
        op,
        result: result.map(|()| StorageOutcome::Done),
    }
}

/// Bytes the vault's filesystem still has room for. `SPEC.md` §4 tests the diff's to-send
/// total against this before a transfer begins.
fn free_space(vault: &Path) -> u64 {
    match rustix::fs::statvfs(vault) {
        Ok(facts) => facts.f_bavail.saturating_mul(facts.f_frsize),
        // A filesystem that will not say how much room it has is not a reason to refuse a
        // transfer; the write that runs out of space reports itself.
        Err(_) => u64::MAX,
    }
}
