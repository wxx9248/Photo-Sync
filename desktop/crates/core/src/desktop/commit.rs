//! Carrying out a commit, in the order `SPEC.md` §7.3 requires.
//!
//! The plan itself is decided in [`crate::commit`]. This module is about ordering, which is
//! where the durability claims live. Two rules run through all of it. A group's directory
//! syncs complete before any done-mark for that group is written, so a done-mark that
//! survives a power loss implies its rename survived too. And staging is cleared write-log
//! first, then the manifest, then the files, so recovery never finds a log it cannot trust.
//!
//! Every step waits for the one before it to complete rather than relying on the shell to
//! perform effects in order. The ordering is then a property of the core, which is what lets
//! the simulator crash between any two of them and check what survived.

use std::collections::{BTreeMap, BTreeSet};

use super::deletion::Nomination;
use super::{Desktop, Pending, error_log, info, warn};
use crate::commit::{self, Plan};
use crate::effect::{Directory, Effect, RejectReason, UiUpdate};
use crate::id::{DeviceId, FileId, Sha256, VaultName};
use crate::naming::{self, Moment};
use crate::store::{ContentRow, PlanAction, StagingEntry, StoreRequest, StoreResponse};

/// How many entries are executed before the directories are synced and the group is marked
/// done. Smaller groups mean less to replay after a crash and more directory syncs.
const GROUP: usize = 64;

/// Which part of a commit an outstanding operation belongs to.
#[derive(Debug)]
pub(super) enum Step {
    ReadClock,
    ListBatch,
    StatStaged { file: FileId },
    LookupContent,
    LookupNames,
    SealPlan,
    Execute { file: FileId },
    SyncVault,
    SyncStaging,
    MarkDone,
    InsertRows,
    ClearLog,
    ClearManifest,
    ClearFiles,
}

/// A commit in progress. One at a time across the whole desktop, because `SPEC.md` §7.3
/// serializes commits under a single lock while leaving transfers concurrent.
#[derive(Debug)]
pub(super) struct Running {
    pub(super) device: DeviceId,
    moment: Option<Moment>,
    entries: Option<Vec<StagingEntry>>,
    missing: BTreeSet<FileId>,

    /// Whether the questions the plan is built from have been asked. They are asked once,
    /// and every answer afterwards only counts down.
    asked: bool,

    stats_awaited: usize,
    known_content: Option<BTreeMap<Sha256, VaultName>>,
    taken: Option<BTreeSet<VaultName>>,
    plan: Plan,

    /// How far through the plan the execution has got.
    next: usize,

    /// The entries of the group being executed, and how many of them are still outstanding.
    group: Vec<FileId>,
    group_awaited: usize,
}

impl Running {
    fn new(device: DeviceId) -> Self {
        Self {
            device,
            moment: None,
            entries: None,
            missing: BTreeSet::new(),
            asked: false,
            stats_awaited: 0,
            known_content: None,
            taken: None,
            plan: Plan::default(),
            next: 0,
            group: Vec::new(),
            group_awaited: 0,
        }
    }

    /// Whether every answer the plan is built from has arrived.
    fn is_gathered(&self) -> bool {
        self.asked
            && self.stats_awaited == 0
            && self.known_content.is_some()
            && self.taken.is_some()
    }
}

impl Desktop {
    /// Whether a device may not start a session because a commit of its own is in the way.
    /// `SPEC.md` §7.3 holds the per-device lock from the finish signal until the commit
    /// completes, so the phone waits rather than adding files to a batch already frozen.
    pub(super) fn is_committing(&self, device: &DeviceId) -> bool {
        self.halted.contains(device)
            || self.queued.contains(device)
            || self
                .running
                .as_ref()
                .is_some_and(|running| running.device == *device)
    }

    /// Takes the finish signal. The batch is frozen from here, whether or not the commit can
    /// start straight away.
    pub(super) fn enqueue_commit(&mut self, device: &DeviceId) -> Vec<Effect> {
        if self.is_committing(device) {
            return vec![warn(format!("{device} asked to finish twice"))];
        }
        self.queued.insert(device.clone());
        self.order.push_back(device.clone());
        self.start_next_commit()
    }

    fn start_next_commit(&mut self) -> Vec<Effect> {
        if self.running.is_some() {
            return Vec::new();
        }
        let Some(device) = self.order.pop_front() else {
            return Vec::new();
        };
        self.queued.remove(&device);
        self.running = Some(Running::new(device.clone()));

        let clock = self.begin(Pending::Commit(Step::ReadClock));
        let list = self.begin(Pending::Commit(Step::ListBatch));
        vec![
            Effect::NotifyUi {
                update: UiUpdate::CommitStarted {
                    device: device.clone(),
                },
            },
            Effect::ReadClock { op: clock },
            Effect::Store {
                op: list,
                request: StoreRequest::ListStagingEntries { device },
            },
        ]
    }

    pub(super) fn clock_read(&mut self, moment: Moment) -> Vec<Effect> {
        let Some(running) = self.running.as_mut() else {
            return vec![warn("the clock was read for no commit".to_string())];
        };
        running.moment = Some(moment);
        self.gather()
    }

    /// Handles one completed step of the running commit.
    pub(super) fn commit_step(&mut self, step: Step, answer: Answer) -> Vec<Effect> {
        if self.running.is_none() {
            return vec![warn(format!("{step:?} completed for no commit"))];
        }
        match answer {
            Answer::Failed(reason) => self.halt(&step, &reason),
            Answer::Done => self.advance(step),
            Answer::Store(response) => self.take_answer(step, response),
            Answer::Present(present) => self.record_stat(step, present),
        }
    }

    fn take_answer(&mut self, step: Step, response: StoreResponse) -> Vec<Effect> {
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        match (&step, response) {
            (Step::ListBatch, StoreResponse::StagingEntries(entries)) => {
                running.entries = Some(entries);
                self.gather()
            }
            (Step::LookupContent, StoreResponse::Content(rows)) => {
                running.known_content = Some(
                    rows.into_iter()
                        .map(|row| (row.digest, row.vault_name))
                        .collect(),
                );
                self.gather()
            }
            (Step::LookupNames, StoreResponse::VaultNames(names)) => {
                running.taken = Some(names.into_iter().collect());
                self.gather()
            }
            _ => self.advance(step),
        }
    }

    fn record_stat(&mut self, step: Step, present: bool) -> Vec<Effect> {
        let Step::StatStaged { file } = step else {
            return Vec::new();
        };
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        if !present {
            running.missing.insert(file);
        }
        running.stats_awaited = running.stats_awaited.saturating_sub(1);
        self.gather()
    }

    /// Asks for whatever the plan still needs, and builds it once nothing is outstanding.
    fn gather(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        let (Some(moment), Some(entries)) = (running.moment, running.entries.as_ref()) else {
            return Vec::new();
        };

        // The questions are asked once, as soon as the clock and the manifest are in hand.
        if !running.asked {
            let batch: Vec<StagingEntry> = entries
                .iter()
                .filter(|entry| entry.is_verified())
                .cloned()
                .collect();
            running.asked = true;
            running.stats_awaited = batch.len();
            return self.ask_about(&batch, moment);
        }

        if !running.is_gathered() {
            return Vec::new();
        }
        self.seal()
    }

    fn ask_about(&mut self, batch: &[StagingEntry], moment: Moment) -> Vec<Effect> {
        let digests: Vec<Sha256> = batch.iter().filter_map(|entry| entry.digest).collect();
        let stems: BTreeSet<String> = batch
            .iter()
            .map(|entry| naming::stem(&entry.name_sources, moment.local))
            .collect();

        let mut effects = Vec::new();
        for entry in batch {
            let op = self.begin(Pending::Commit(Step::StatStaged { file: entry.file }));
            effects.push(Effect::StatStagingFile {
                op,
                file: entry.file,
            });
        }

        let content = self.begin(Pending::Commit(Step::LookupContent));
        effects.push(Effect::Store {
            op: content,
            request: StoreRequest::LookupContent { digests },
        });
        let names = self.begin(Pending::Commit(Step::LookupNames));
        effects.push(Effect::Store {
            op: names,
            request: StoreRequest::TakenVaultNames {
                stems: stems.into_iter().collect(),
            },
        });

        // A batch of nothing has nothing to stat, so the answers above are all that is left.
        effects
    }

    /// `SPEC.md` §7.3 step 1: write the map, flush it, seal it with an end marker. The store
    /// does all three in one transaction, so a half-written plan cannot outlive a crash.
    fn seal(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        let (Some(moment), Some(entries), Some(known_content), Some(taken)) = (
            running.moment,
            running.entries.as_ref(),
            running.known_content.as_ref(),
            running.taken.as_ref(),
        ) else {
            return Vec::new();
        };

        running.plan = commit::plan(&commit::Inputs {
            device: &running.device,
            entries,
            missing: &running.missing,
            known_content,
            taken_names: taken,
            imported_at: moment,
        });

        let device = running.device.clone();
        let dropped = running.plan.dropped.clone();
        let entries = running.plan.entries.clone();

        let mut effects: Vec<Effect> = dropped
            .iter()
            .map(|file| {
                warn(format!(
                    "{file:?} is in the manifest with no file, dropping it"
                ))
            })
            .collect();
        let op = self.begin(Pending::Commit(Step::SealPlan));
        effects.push(Effect::Store {
            op,
            request: StoreRequest::SealCommitPlan {
                device,
                plan: entries,
            },
        });
        effects
    }

    fn advance(&mut self, step: Step) -> Vec<Effect> {
        match step {
            Step::SealPlan => self.execute_group(),
            Step::Execute { .. } => self.entry_executed(),
            Step::SyncVault => self.sync_staging(),
            Step::SyncStaging => self.mark_group_done(),
            Step::MarkDone => self.execute_group(),
            Step::InsertRows => self.clear(Step::ClearLog),
            Step::ClearLog => self.clear(Step::ClearManifest),
            Step::ClearManifest => self.clear(Step::ClearFiles),
            Step::ClearFiles => self.finish(),
            Step::ReadClock
            | Step::ListBatch
            | Step::StatStaged { .. }
            | Step::LookupContent
            | Step::LookupNames => Vec::new(),
        }
    }

    /// `SPEC.md` §7.3 step 2: imports are renamed into the vault and duplicates have their
    /// staged file deleted, a group at a time.
    fn execute_group(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        let start = running.next;
        let end = (start + GROUP).min(running.plan.entries.len());
        if start >= end {
            return self.insert_rows();
        }

        let group: Vec<(FileId, PlanAction)> = running.plan.entries[start..end]
            .iter()
            .map(|entry| (entry.file, entry.action.clone()))
            .collect();
        running.next = end;
        running.group = group.iter().map(|(file, _)| *file).collect();
        running.group_awaited = group.len();

        group
            .into_iter()
            .map(|(file, action)| {
                let op = self.begin(Pending::Commit(Step::Execute { file }));
                match action {
                    PlanAction::Import { name } => Effect::RenameIntoVault { op, file, name },
                    PlanAction::Duplicate { .. } => Effect::RemoveStagingFile { op, file },
                }
            })
            .collect()
    }

    fn entry_executed(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        running.group_awaited = running.group_awaited.saturating_sub(1);
        if running.group_awaited > 0 {
            return Vec::new();
        }
        self.sync_vault()
    }

    /// The barrier of `SPEC.md` §7.3 step 2. Both directories are made durable before a
    /// single done-mark is written, because a done-mark that outlives its rename would tell
    /// recovery a file is in the vault when it is not.
    fn sync_vault(&mut self) -> Vec<Effect> {
        let op = self.begin(Pending::Commit(Step::SyncVault));
        vec![Effect::SyncDirectory {
            op,
            directory: Directory::Vault,
        }]
    }

    fn sync_staging(&mut self) -> Vec<Effect> {
        let Some(device) = self.committing_device() else {
            return Vec::new();
        };
        let op = self.begin(Pending::Commit(Step::SyncStaging));
        vec![Effect::SyncDirectory {
            op,
            directory: Directory::Staging { device },
        }]
    }

    fn mark_group_done(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.as_ref() else {
            return Vec::new();
        };
        let device = running.device.clone();
        let files = running.group.clone();
        let op = self.begin(Pending::Commit(Step::MarkDone));
        vec![Effect::Store {
            op,
            request: StoreRequest::MarkPlanEntriesDone { device, files },
        }]
    }

    /// `SPEC.md` §7.3 step 3: the index rows go in first and are durable before staging is
    /// touched, so the next commit's map building sees this batch's content.
    fn insert_rows(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.as_ref() else {
            return Vec::new();
        };
        let contents: Vec<ContentRow> = running.plan.contents.clone();
        let device_files = running.plan.device_files.clone();
        let op = self.begin(Pending::Commit(Step::InsertRows));
        vec![Effect::Store {
            op,
            request: StoreRequest::InsertCommittedBatch {
                contents,
                device_files,
            },
        }]
    }

    /// Clearing order is the write-log, then the manifest, then the files. Recovery reads
    /// them in the opposite direction, so removing the log first is what makes a half-cleared
    /// staging directory look like a fresh one rather than a commit to replay.
    fn clear(&mut self, step: Step) -> Vec<Effect> {
        let Some(device) = self.committing_device() else {
            return Vec::new();
        };
        let request = match step {
            Step::ClearLog => Some(StoreRequest::ClearCommitPlan {
                device: device.clone(),
            }),
            Step::ClearManifest => Some(StoreRequest::ClearStagingManifest {
                device: device.clone(),
            }),
            _ => None,
        };

        match request {
            Some(request) => {
                let op = self.begin(Pending::Commit(step));
                vec![Effect::Store { op, request }]
            }
            None => {
                let op = self.begin(Pending::Commit(Step::ClearFiles));
                vec![Effect::ClearStagingDirectory { op, device }]
            }
        }
    }

    fn finish(&mut self) -> Vec<Effect> {
        let Some(running) = self.running.take() else {
            return Vec::new();
        };
        let device = running.device;
        let summary = running.plan.summary;

        for file in running.plan.entries.iter().map(|entry| entry.file) {
            self.durable_digests.remove(&file);
        }

        let mut effects = vec![Effect::NotifyUi {
            update: UiUpdate::CommitFinished {
                device: device.clone(),
                summary,
            },
        }];

        // SPEC.md §6.6: the commit is followed by the candidates it makes possible.
        let committed = running
            .plan
            .device_files
            .iter()
            .map(|row| row.path.clone())
            .collect();
        effects.extend(self.nominate(
            &device,
            Nomination {
                committed,
                commit: summary,
                awaited: 0,
                candidates: Vec::new(),
            },
        ));
        effects.extend(self.start_next_commit());
        effects
    }

    /// A rename or a manifest write that failed stops the commit where it stands. The sealed
    /// write-log is left alone: it is what recovery replays once the condition clears, and
    /// the device stays locked until then so nothing writes over it.
    fn halt(&mut self, step: &Step, reason: &str) -> Vec<Effect> {
        let Some(running) = self.running.take() else {
            return Vec::new();
        };
        let device = running.device;
        self.halted.insert(device.clone());

        let mut effects = vec![
            error_log(format!(
                "the commit for {device} stopped at {}: {reason}",
                describe_step(step)
            )),
            Effect::NotifyUi {
                update: UiUpdate::Error {
                    device: device.clone(),
                    message: format!("the import stopped part-way: {reason}"),
                },
            },
            info(format!(
                "{device} stays locked until its sealed write-log is replayed"
            )),
        ];
        effects.extend(self.start_next_commit());
        effects
    }

    fn committing_device(&self) -> Option<DeviceId> {
        self.running.as_ref().map(|running| running.device.clone())
    }

    pub(super) fn refuse_while_committing(device: &DeviceId) -> Vec<Effect> {
        vec![
            info(format!("{device} connected while its import was finishing")),
            Effect::RejectSession {
                device: device.clone(),
                reason: RejectReason::CommitInProgress,
            },
        ]
    }
}

/// What a completed commit step answered with.
pub(super) enum Answer {
    Done,
    Present(bool),
    Store(StoreResponse),
    Failed(String),
}

/// Where a commit stopped, said in terms a person can act on.
fn describe_step(step: &Step) -> String {
    match step {
        Step::ReadClock => "reading the clock".to_string(),
        Step::ListBatch => "reading the manifest".to_string(),
        Step::StatStaged { file } => format!("looking for the staged {file:?}"),
        Step::LookupContent => "looking content up in the index".to_string(),
        Step::LookupNames => "looking up the names already taken".to_string(),
        Step::SealPlan => "sealing the write-log".to_string(),
        Step::Execute { file } => format!("moving {file:?} into the vault"),
        Step::SyncVault => "making the vault durable".to_string(),
        Step::SyncStaging => "making staging durable".to_string(),
        Step::MarkDone => "recording a group as done".to_string(),
        Step::InsertRows => "writing the index rows".to_string(),
        Step::ClearLog => "clearing the write-log".to_string(),
        Step::ClearManifest => "clearing the manifest".to_string(),
        Step::ClearFiles => "clearing the staging directory".to_string(),
    }
}
