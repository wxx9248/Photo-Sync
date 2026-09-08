//! The desktop, driven.
//!
//! Every effect the core emits is performed against the simulated storage and store, and the
//! outcome is handed back as an event, in the order the effects were emitted. That loop is
//! the whole simulator: the core cannot tell it apart from the shell, which is what lets a
//! test crash it, starve it, or lie to it and still be testing the real logic.

use std::collections::VecDeque;

use photo_sync_core::effect::Effect;
use photo_sync_core::event::{Event, StorageOutcome};
use photo_sync_core::id::DeviceId;
use photo_sync_core::port::StorageError;
use photo_sync_core::store::{StagingEntry, StoreError, StoreRequest};
use photo_sync_core::{Desktop, Directory, Moment};

use crate::storage::Storage;
use crate::store::Store;

/// Ways storage can refuse. Each one stands for a condition `SPEC.md` names: a full disk, a
/// vault the desktop cannot read, a shell whose threads finish in their own order.
#[derive(Clone, Copy, Debug, Default)]
pub struct Faults {
    pub refuse_writes: bool,
    pub refuse_renames: bool,
    pub refuse_vault_stats: bool,
    pub refuse_name_sources: bool,
    pub refuse_store: bool,

    /// Answers every staging stat only once nothing else is outstanding.
    pub answer_stats_last: bool,
}

/// One desktop, its storage, and the loop between them.
///
/// The three parts are public because a test sets the world up and then reads it back, and
/// accessors over each of them would say nothing their names do not.
pub struct Simulation {
    pub desktop: Desktop,
    pub storage: Storage,
    pub store: Store,
    pub faults: Faults,

    /// What the desktop is told the time is whenever it asks.
    pub now: Moment,

    /// What the desktop is told is free when it asks.
    pub free_space: u64,

    log: Vec<Effect>,
}

impl Simulation {
    #[must_use]
    pub fn new(now: Moment) -> Self {
        Self {
            desktop: Desktop::new(),
            storage: Storage::new(),
            store: Store::new(),
            faults: Faults::default(),
            now,
            free_space: u64::MAX,
            log: Vec::new(),
        }
    }

    /// Starts the desktop, as a machine coming up would.
    pub fn start(&mut self) {
        let at = self.now.at;
        self.deliver(Event::Started { now: at });
    }

    /// Delivers one event and performs everything it leads to.
    pub fn deliver(&mut self, event: Event) {
        let mut queue: VecDeque<Effect> = self.desktop.handle(event).into();
        let mut held: VecDeque<Event> = VecDeque::new();
        loop {
            while let Some(effect) = queue.pop_front() {
                let completion = self.perform(&effect);
                let hold = self.faults.answer_stats_last
                    && matches!(effect, Effect::StatStagingFile { .. });
                self.log.push(effect);
                let Some(completion) = completion else {
                    continue;
                };
                if hold {
                    held.push_back(completion);
                } else {
                    queue.extend(self.desktop.handle(completion));
                }
            }
            let Some(deferred) = held.pop_front() else {
                break;
            };
            queue.extend(self.desktop.handle(deferred));
        }
    }

    /// Puts a file in a device's staging area with the manifest row describing it, as an
    /// earlier session would have left it.
    pub fn stage(&mut self, device: &DeviceId, entry: StagingEntry, bytes: &[u8]) {
        let file = entry.file;
        self.storage.create(device, file);
        self.storage.write_at(file, 0, bytes);
        self.storage.sync_file(file);
        self.store.run(&StoreRequest::BeginStagingEntry {
            device: device.clone(),
            entry,
        });
    }

    /// Delivers one event and answers none of it. Used where the property is that something
    /// has *not* happened yet.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        let effects = self.desktop.handle(event);
        self.log.extend(effects.iter().cloned());
        effects
    }

    #[must_use]
    pub fn log(&self) -> &[Effect] {
        &self.log
    }

    pub fn take_log(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.log)
    }

    fn perform(&mut self, effect: &Effect) -> Option<Event> {
        match effect {
            Effect::CreateStagingFile { op, device, file } => {
                self.storage.create(device, *file);
                Some(done(*op))
            }
            Effect::WriteChunk {
                op,
                file,
                offset,
                data,
            } => {
                if self.faults.refuse_writes {
                    return Some(failed(*op, StorageError::NoSpace));
                }
                self.storage.write_at(*file, *offset, data);
                Some(done(*op))
            }
            Effect::SyncFile { op, file } => {
                self.storage.sync_file(*file);
                Some(done(*op))
            }
            Effect::SyncDirectory { op, directory } => {
                // Directory entries are durable the moment they are made here. Modelling the
                // gap between a rename and its parent's sync is milestone M2.
                let _ = directory_of(directory);
                Some(done(*op))
            }
            Effect::TruncateFile { op, file, length } => {
                self.storage.truncate(*file, *length);
                Some(done(*op))
            }
            Effect::FinalizeStagingFile { op, file } => {
                self.storage.finalize(*file);
                Some(done(*op))
            }
            Effect::ReadNameSources { op, file, .. } => {
                if self.faults.refuse_name_sources {
                    return Some(failed(*op, StorageError::Failed("unreadable".into())));
                }
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: Ok(StorageOutcome::NameSources(
                        self.storage.name_sources(*file),
                    )),
                })
            }
            Effect::RenameIntoVault { op, file, name } => {
                if self.faults.refuse_renames {
                    return Some(failed(*op, StorageError::NoSpace));
                }
                self.storage.rename_into_vault(*file, name);
                Some(done(*op))
            }
            Effect::RemoveStagingFile { op, file } => {
                self.storage.remove(*file);
                Some(done(*op))
            }
            Effect::ClearStagingDirectory { op, device } => {
                self.storage.clear_staging(device);
                Some(done(*op))
            }
            Effect::StatStagingFile { op, file } => Some(Event::StorageOpCompleted {
                op: *op,
                result: Ok(StorageOutcome::StagingFile {
                    present: self.storage.holds(*file),
                }),
            }),
            Effect::StatVaultFile { op, name } => {
                if self.faults.refuse_vault_stats {
                    return Some(failed(*op, StorageError::Failed("unreadable".into())));
                }
                let found = self.storage.stat_vault(name);
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: Ok(StorageOutcome::VaultFile {
                        present: found.is_some(),
                        size: found.unwrap_or(0),
                    }),
                })
            }
            Effect::Store { op, request } => {
                let result = if self.faults.refuse_store {
                    Err(StoreError::NoSpace)
                } else {
                    Ok(self.answer(request))
                };
                Some(Event::StoreOpCompleted { op: *op, result })
            }
            Effect::MeasureFreeSpace { op } => Some(Event::FreeSpaceMeasured {
                op: *op,
                available_bytes: self.free_space,
            }),
            Effect::ReadClock { op } => Some(Event::ClockRead {
                op: *op,
                moment: self.now,
            }),
            Effect::SetTimer { op, at } => Some(Event::TimerFired { op: *op, now: *at }),
            Effect::SendDiff { .. }
            | Effect::SendUploadResult { .. }
            | Effect::SendCandidates { .. }
            | Effect::SendSessionSummary { .. }
            | Effect::RejectSession { .. }
            | Effect::Log { .. }
            | Effect::NotifyUi { .. } => None,
        }
    }

    fn answer(&mut self, request: &StoreRequest) -> photo_sync_core::store::StoreResponse {
        self.store.run(request)
    }
}

fn directory_of(directory: &Directory) -> &'static str {
    match directory {
        Directory::Vault => "vault",
        Directory::Staging { .. } => "staging",
    }
}

fn done(op: photo_sync_core::OpId) -> Event {
    Event::StorageOpCompleted {
        op,
        result: Ok(StorageOutcome::Done),
    }
}

fn failed(op: photo_sync_core::OpId, error: StorageError) -> Event {
    Event::StorageOpCompleted {
        op,
        result: Err(error),
    }
}

/// A moment built from a plain instant, for tests that do not care what the clock reads.
#[must_use]
pub fn moment(at: i64, local: photo_sync_core::CivilTime) -> Moment {
    Moment {
        at: photo_sync_core::Timestamp(at),
        local,
    }
}

impl crate::phone::Driver for Simulation {
    fn deliver(&mut self, event: Event) {
        Simulation::deliver(self, event);
    }

    fn take_log(&mut self) -> Vec<Effect> {
        Simulation::take_log(self)
    }
}
