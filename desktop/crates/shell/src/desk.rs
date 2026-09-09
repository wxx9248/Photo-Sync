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
use photo_sync_core::effect::{Effect, LogLevel};
use photo_sync_core::event::{Event, StorageOutcome};
use photo_sync_core::port::StorageError;

use crate::suspend::{self, Busy, Inhibition};
use crate::views::Views;

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

    /// What the window would show. Kept here because this is the one place every effect
    /// passes through, so nothing can happen that the window never hears about.
    views: Views,

    /// Which phones are mid-session, and the promise held while any of them are.
    ///
    /// `SPEC.md` §4 asks the desktop to inhibit suspend while a session is active, and this
    /// is the one place that knows when one starts: every event a phone causes arrives here.
    busy: Busy,
    inhibition: Option<Inhibition>,
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
            views: Views::new(),
            busy: Busy::new(),
            inhibition: None,
        };
        let started = clock::now();
        desk.deliver(Event::Started { now: started.at });
        Ok(desk)
    }

    /// Delivers one event and performs everything it leads to.
    /// Takes or releases the promise that the machine will stay awake.
    ///
    /// The first phone to arrive takes it and the last to leave releases it, because a second
    /// phone is already covered by the promise the first one made. A machine that will not
    /// give one is logged once and otherwise carried on with: photographs still move, the
    /// machine might go to sleep underneath them, and the next session picks up where this
    /// one stopped.
    fn mind_the_clock(&mut self, event: &Event) {
        match event {
            Event::PeerConnected { device, .. } => {
                if self.busy.started(device) {
                    match suspend::inhibit("moving photographs off a phone") {
                        Ok(held) => self.inhibition = Some(held),
                        Err(error) => tracing::warn!("{error}"),
                    }
                }
            }
            Event::PeerDisconnected { device } => {
                if self.busy.ended(device) {
                    // Dropping it is how the machine is told it may sleep again.
                    self.inhibition = None;
                }
            }
            _ => {}
        }
    }

    /// What the window should be showing now.
    #[must_use]
    pub fn views(&self) -> &Views {
        &self.views
    }

    pub fn deliver(&mut self, event: Event) {
        self.mind_the_clock(&event);
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
            Effect::ReadNameSources { op, mtime, .. } => Some(Event::StorageOpCompleted {
                op: *op,
                // The capture time of SPEC.md §7.2's first two sources needs nom-exif, which
                // is not built, so the modification time is the first reading offered. It is
                // read in UTC until a timezone database arrives with the extraction.
                result: Ok(StorageOutcome::NameSources(vec![clock::civil_from_unix(
                    mtime.0,
                )])),
            }),
            Effect::SetModifiedTime { op, file, mtime } => Some(storage_result(
                *op,
                self.storage.set_modified_time(*file, *mtime),
            )),
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
            Effect::ReadStagedRange {
                op,
                file,
                offset,
                length,
            } => {
                let read = self.storage.read_staged_range(*file, *offset, *length);
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: read.map(StorageOutcome::Bytes),
                })
            }
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
            // What the core wanted said, said. `STACK.md` §3.9 wants the commit and
            // recovery paths readable from the logs alone after the fact, and the core
            // decides what is worth saying — it cannot write it down itself.
            Effect::NotifyUi { update } => {
                self.views.observe(update);
                None
            }
            Effect::Log { level, message } => {
                match level {
                    LogLevel::Error => tracing::error!(target: "photo_sync::core", "{message}"),
                    LogLevel::Warn => tracing::warn!(target: "photo_sync::core", "{message}"),
                    LogLevel::Info => tracing::info!(target: "photo_sync::core", "{message}"),
                }
                None
            }
            // Timers, messages to a phone, and the interface are the server's and the
            // window's business. The loop records them and moves on.
            Effect::SetTimer { .. }
            | Effect::SendDiff { .. }
            | Effect::SendUploadResult { .. }
            | Effect::SendCandidates { .. }
            | Effect::SendSessionSummary { .. }
            | Effect::RejectSession { .. } => None,
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
