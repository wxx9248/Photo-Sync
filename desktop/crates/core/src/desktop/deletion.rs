//! Offering the phone the photos it may safely delete.
//!
//! `SPEC.md` §8 puts the whole safety argument in one sentence: the vault, not the index, is
//! the source of truth for "safely stored". A photo is offered only when the index covers it
//! and a look at the recorded vault name finds the file still there, at the size the index
//! recorded. A photo the user curated out of the vault is silently kept on the phone.
//!
//! Every way this can go wrong under-deletes. A stale row, a corrupt row, a forged row: each
//! either fails the vault check here or fails the phone's own check afterwards. Nothing here
//! can turn into a deletion of something the desktop does not hold.

use std::collections::BTreeSet;

use super::{Desktop, Pending, warn};
use crate::catalog::CatalogEntry;
use crate::effect::{CandidateOrigin, CommitSummary, DeletionCandidate, Effect, SessionSummary};
use crate::event::{DeletionOutcome, DeletionResult};
use crate::id::{DeviceId, DevicePath};
use crate::session::Phase;
use crate::store::{DeviceFileRow, StoreRequest};

/// What the desktop has worked out about the deletions it will offer.
#[derive(Debug, Default)]
pub(crate) struct Nomination {
    /// Paths this session's own batch committed. They pass the phone's cheap check, because
    /// their content was verified against a digest minutes ago.
    pub(crate) committed: BTreeSet<DevicePath>,

    pub(crate) commit: CommitSummary,

    /// Vault copies still being looked for.
    pub(crate) awaited: usize,

    pub(crate) candidates: Vec<DeletionCandidate>,
}

impl Desktop {
    /// Starts nomination once the commit for this device has finished.
    pub(super) fn nominate(&mut self, device: &DeviceId, nomination: Nomination) -> Vec<Effect> {
        let Some(session) = self.sessions.get_mut(device) else {
            return Vec::new();
        };
        session.nomination = nomination;
        session.phase = Phase::Nominating;

        // Only the catalog this session froze, so the list can never name a photo the phone
        // has stopped knowing about. SPEC.md §8.
        let paths: Vec<DevicePath> = session
            .catalog
            .entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        if paths.is_empty() {
            return self.send_candidates(device);
        }

        let op = self.begin(Pending::NominationRows {
            device: device.clone(),
        });
        vec![Effect::Store {
            op,
            request: StoreRequest::LookupDeviceFiles {
                device: device.clone(),
                paths,
            },
        }]
    }

    /// Turns the index rows for this catalog into vault checks.
    pub(super) fn nomination_rows(
        &mut self,
        device: &DeviceId,
        rows: Vec<DeviceFileRow>,
    ) -> Vec<Effect> {
        let Some(session) = self.sessions.get(device) else {
            return Vec::new();
        };
        if !matches!(session.phase, Phase::Nominating) {
            return vec![warn(format!("{device} is not nominating anything"))];
        }

        let covered: Vec<DeviceFileRow> = rows
            .into_iter()
            .filter(|row| {
                session
                    .catalog
                    .entries()
                    .iter()
                    .any(|entry| covers(entry, row))
            })
            .collect();

        if let Some(session) = self.sessions.get_mut(device) {
            session.nomination.awaited = covered.len();
        }
        if covered.is_empty() {
            return self.send_candidates(device);
        }

        covered
            .into_iter()
            .map(|row| {
                let name = row.vault_name.clone();
                let op = self.begin(Pending::NominationStat {
                    device: device.clone(),
                    row: Box::new(row),
                });
                Effect::StatVaultFile { op, name }
            })
            .collect()
    }

    /// Takes the answer about one vault copy. `SPEC.md` §8 wants both existence and size,
    /// because a file replaced by something else is not the photo the index recorded.
    pub(super) fn vault_copy_checked(
        &mut self,
        device: &DeviceId,
        row: &DeviceFileRow,
        found: Option<u64>,
    ) -> Vec<Effect> {
        let Some(session) = self.sessions.get_mut(device) else {
            return Vec::new();
        };

        if found == Some(row.size) {
            let origin = if session.nomination.committed.contains(&row.path) {
                CandidateOrigin::ThisTransfer
            } else {
                CandidateOrigin::Earlier
            };
            session.nomination.candidates.push(DeletionCandidate {
                path: row.path.clone(),
                size: row.size,
                mtime: row.mtime,
                expected: row.digest,
                origin,
            });
        }

        session.nomination.awaited = session.nomination.awaited.saturating_sub(1);
        if session.nomination.awaited > 0 {
            return Vec::new();
        }
        self.send_candidates(device)
    }

    fn send_candidates(&mut self, device: &DeviceId) -> Vec<Effect> {
        let Some(session) = self.sessions.get_mut(device) else {
            return Vec::new();
        };
        session.phase = Phase::Deleting;
        vec![Effect::SendCandidates {
            device: device.clone(),
            candidates: session.nomination.candidates.clone(),
            commit: session.nomination.commit,
        }]
    }

    /// The phone has said what it deleted and what it kept, so the session can be summed up
    /// and closed. `SPEC.md` §6.6.
    pub(super) fn deletions_reported(
        &mut self,
        device: &DeviceId,
        outcomes: &[DeletionOutcome],
    ) -> Vec<Effect> {
        let Some(session) = self.sessions.remove(device) else {
            return vec![warn(format!("{device} reported deletions with no session"))];
        };

        let mut summary = SessionSummary {
            sent: session.sent,
            skipped: session.skipped,
            failed: session.failed,
            ..SessionSummary::default()
        };
        for outcome in outcomes {
            match outcome.result {
                DeletionResult::Deleted => {
                    summary.deleted += 1;
                    summary.bytes_freed += size_of(&session.nomination.candidates, &outcome.path);
                }
                DeletionResult::KeptChanged | DeletionResult::KeptUser => summary.kept += 1,
                DeletionResult::Failed => summary.failed += 1,
            }
        }

        vec![Effect::SendSessionSummary {
            device: device.clone(),
            summary,
        }]
    }
}

/// Whether an index row describes the very file the catalog listed. All three fields, because
/// that triple is what identifies a file everywhere else in the specification.
fn covers(entry: &CatalogEntry, row: &DeviceFileRow) -> bool {
    entry.path == row.path && entry.size == row.size && entry.mtime == row.mtime
}

fn size_of(candidates: &[DeletionCandidate], path: &DevicePath) -> u64 {
    candidates
        .iter()
        .find(|candidate| candidate.path == *path)
        .map_or(0, |candidate| candidate.size)
}
