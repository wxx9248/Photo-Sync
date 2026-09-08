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
use photo_sync_core::store::{DeviceFileRow, StagingEntry, StoreError, StoreRequest};
use photo_sync_core::{Desktop, Moment};

use crate::storage::Storage;
use crate::store::Store;
use photo_sync_core::RunningDigest;
use photo_sync_core::id::{DevicePath, Sha256};
use photo_sync_model::{Model, Recorded};
use std::collections::{BTreeMap, BTreeSet};

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

    /// An account of what the desktop should be holding, kept beside it and told the same
    /// story. Nothing consults it until something asks the two to agree.
    pub model: Model,

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
            model: Model::new(),
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

    /// The power goes out, and the machine comes back.
    ///
    /// Everything the filesystem had not been told to write down is lost. The databases are
    /// not: `STACK.md` §3.5 runs both in WAL mode with `synchronous=FULL`, so a transaction
    /// that returned survived, and one that did not never existed. Modelling a crash inside
    /// SQLite's own writing needs the virtual file system that milestone M2 also calls for,
    /// and until it lands this is the more forgiving assumption of the two.
    ///
    /// Everything the core held in memory is gone, which is the point: what the desktop knows
    /// afterwards is only what it wrote down.
    pub fn restart(&mut self) {
        self.storage.crash();
        self.desktop = Desktop::new();
        self.log.clear();
        self.start();
    }

    /// Asks the desktop and the model whether they agree, and says how they differ if not.
    ///
    /// `docs/VERIFICATION.md` §L2 asks this after every commit, every recovery, and at the
    /// end of a session. A difference is reported as a difference in state, which is
    /// something to act on, rather than as an assertion that failed.
    ///
    /// # Errors
    /// Lists every way the two accounts disagree.
    pub fn agree(&self) -> Result<(), Vec<String>> {
        let expected = self.model.expected();
        let mut differences = Vec::new();

        let held: BTreeSet<Sha256> = self
            .storage
            .vault()
            .values()
            .map(|bytes| digest(bytes))
            .collect();
        compare("the vault holds", &expected.vault, &held, &mut differences);

        let known: BTreeSet<Sha256> = self.store.content().keys().copied().collect();
        compare(
            "the index knows",
            &expected.content,
            &known,
            &mut differences,
        );

        let recorded: BTreeMap<(DeviceId, DevicePath), Recorded> = self
            .store
            .device_files()
            .into_iter()
            .map(|row| {
                (
                    (row.device.clone(), row.path.clone()),
                    Recorded {
                        digest: row.digest,
                        size: row.size,
                        mtime: row.mtime,
                    },
                )
            })
            .collect();
        for (key, expected_row) in &expected.device_files {
            match recorded.get(key) {
                Some(found) if found == expected_row => {}
                Some(found) => differences.push(format!(
                    "{} on {} was recorded as {found:?}, expected {expected_row:?}",
                    key.1, key.0
                )),
                None => differences.push(format!(
                    "{} on {} has no row, expected {expected_row:?}",
                    key.1, key.0
                )),
            }
        }
        for key in recorded.keys() {
            if !expected.device_files.contains_key(key) {
                differences.push(format!("{} on {} has a row nothing earned", key.1, key.0));
            }
        }

        if differences.is_empty() {
            Ok(())
        } else {
            Err(differences)
        }
    }

    /// Records a photograph an earlier session imported, in every account of the world at
    /// once: the index, the vault if its copy is still there, and the model.
    ///
    /// Seeding them separately is how they drift, and a model told a different history than
    /// the desktop reports differences that are the test's fault rather than the code's.
    pub fn remember_import(&mut self, row: DeviceFileRow, vault_copy: Option<&[u8]>) {
        self.model
            .remember_import(&row.device, &row.path, row.digest, row.size, row.mtime);
        if let Some(bytes) = vault_copy {
            self.model.remember_vault_copy(row.digest);
            self.storage.put_in_vault(&row.vault_name, bytes.to_vec());
        }
        self.store.remember_import(row);
    }

    /// Delivers one event and performs everything it leads to.
    pub fn deliver(&mut self, event: Event) {
        self.model.observe(&event);
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

    /// Delivers one event and performs what it leads to, up to but not including the first
    /// effect the test names.
    ///
    /// This is how a crash is placed. The machine stops with that effect never having
    /// happened, and everything queued behind it never having been asked for, which is what a
    /// power loss looks like from the outside.
    pub fn deliver_until(&mut self, event: Event, stop: fn(&Effect) -> bool) {
        self.model.observe(&event);
        let mut queue: VecDeque<Effect> = self.desktop.handle(event).into();
        while let Some(effect) = queue.pop_front() {
            if stop(&effect) {
                return;
            }
            let completion = self.perform(&effect);
            self.log.push(effect);
            if let Some(completion) = completion {
                queue.extend(self.desktop.handle(completion));
            }
        }
    }

    /// Delivers one event and answers none of it. Used where the property is that something
    /// has *not* happened yet.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        self.model.observe(&event);
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
                self.storage.sync_directory(directory);
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
            Effect::ReadNameSources { op, file, mtime } => {
                if self.faults.refuse_name_sources {
                    return Some(failed(*op, StorageError::Failed("unreadable".into())));
                }
                Some(Event::StorageOpCompleted {
                    op: *op,
                    result: Ok(StorageOutcome::NameSources(
                        self.storage.name_sources(*file, *mtime),
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

fn digest(bytes: &[u8]) -> Sha256 {
    let mut running = RunningDigest::new();
    running.update(bytes);
    running.peek()
}

/// Says what one side has that the other does not, rather than that they differ.
fn compare(
    what: &str,
    expected: &BTreeSet<Sha256>,
    found: &BTreeSet<Sha256>,
    into: &mut Vec<String>,
) {
    for missing in expected.difference(found) {
        into.push(format!("{what} nothing for {missing:?}, which it should"));
    }
    for extra in found.difference(expected) {
        into.push(format!("{what} {extra:?}, which nothing earned"));
    }
}
