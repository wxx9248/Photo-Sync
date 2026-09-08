//! The plan a commit follows, and the rows it will leave behind.
//!
//! `SPEC.md` §7.3 step 1 builds a name map from the verified entries in one device's staging
//! manifest and seals it before anything moves. Building it is where deduplication happens,
//! at the single point the specification allows, and it decides every name the batch will
//! use. The work is pure: it reads what the index already holds and answers with the plan,
//! so it can be checked by calling it.

use std::collections::{BTreeMap, BTreeSet};

use crate::effect::CommitSummary;
use crate::id::{DeviceId, Sha256, VaultName};
use crate::naming::{self, Moment};
use crate::store::{ContentRow, DeviceFileRow, PlanAction, PlanEntry, StagingEntry};

/// What a commit needs to know before it can decide anything.
pub struct Inputs<'a> {
    pub device: &'a DeviceId,

    /// The device's staging manifest, frozen when the finish signal was processed.
    pub entries: &'a [StagingEntry],

    /// Entries whose staged file is not there. `SPEC.md` §7.3 drops them rather than
    /// planning a rename that could not happen.
    pub missing: &'a BTreeSet<crate::id::FileId>,

    /// Content the index already holds, and the vault name it holds it under.
    pub known_content: &'a BTreeMap<Sha256, VaultName>,

    /// Vault names already spoken for, which a new name has to avoid.
    pub taken_names: &'a BTreeSet<VaultName>,

    pub imported_at: Moment,
}

/// Everything the commit will do, decided before any of it happens.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The map, in the order it is executed and replayed.
    pub entries: Vec<PlanEntry>,

    /// Rows for content the vault did not hold before this batch.
    pub contents: Vec<ContentRow>,

    /// One row for every file in the batch, duplicates included, because the phone still
    /// holds a copy of each and the index is what proves it is safe to delete.
    pub device_files: Vec<DeviceFileRow>,

    /// Manifest entries dropped for want of a file. Worth a line in the log.
    pub dropped: Vec<crate::id::FileId>,

    pub summary: CommitSummary,
}

/// Builds the name map of `SPEC.md` §7.3 step 1.
///
/// Content already in the index becomes a duplicate that inherits the vault name that content
/// already has. Content met twice inside one batch does the same, because the first of them
/// is what puts the name in the index. Everything else is an import and is given a fresh
/// name that no other file in the vault or in this batch will take.
#[must_use]
pub fn plan(inputs: &Inputs<'_>) -> Plan {
    let mut plan = Plan::default();

    // The index is the authority on content, and a batch adds to that authority as it goes.
    let mut assigned = inputs.known_content.clone();
    let mut taken = inputs.taken_names.clone();

    for entry in in_arrival_order(inputs.entries) {
        let Some(digest) = entry.digest else {
            continue;
        };
        if inputs.missing.contains(&entry.file) {
            plan.dropped.push(entry.file);
            continue;
        }

        let action = match assigned.get(&digest) {
            Some(name) => {
                plan.summary.duplicates += 1;
                PlanAction::Duplicate { name: name.clone() }
            }
            None => {
                let name = naming::assign(
                    &entry.name_sources,
                    inputs.imported_at.local,
                    &entry.path,
                    &taken,
                );
                taken.insert(name.clone());
                assigned.insert(digest, name.clone());
                plan.contents.push(ContentRow {
                    digest,
                    vault_name: name.clone(),
                });
                plan.summary.imported += 1;
                plan.summary.bytes_committed += entry.size;
                PlanAction::Import { name }
            }
        };

        plan.device_files.push(DeviceFileRow {
            device: inputs.device.clone(),
            path: entry.path.clone(),
            size: entry.size,
            mtime: entry.mtime,
            digest,
            vault_name: name_of(&action).clone(),
            committed_at: inputs.imported_at.at,
        });
        plan.entries.push(PlanEntry {
            file: entry.file,
            action,
        });
    }

    plan
}

/// The batch in the order its files arrived. Identifiers only ever count up, so this is both
/// stable across runs and the order a person would expect the vault names to follow.
fn in_arrival_order(entries: &[StagingEntry]) -> Vec<&StagingEntry> {
    let mut ordered: Vec<&StagingEntry> =
        entries.iter().filter(|entry| entry.is_verified()).collect();
    ordered.sort_by_key(|entry| entry.file);
    ordered
}

fn name_of(action: &PlanAction) -> &VaultName {
    match action {
        PlanAction::Import { name } | PlanAction::Duplicate { name } => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::covers;
    use crate::id::{DevicePath, FileId, Timestamp};
    use crate::naming::CivilTime;

    const MTIME: i64 = 1_756_000_000;

    fn device() -> DeviceId {
        DeviceId::new("phone-a")
    }

    fn imported_at() -> Moment {
        Moment {
            at: Timestamp(1_757_000_000),
            local: CivilTime {
                year: 2026,
                month: 9,
                day: 7,
                hour: 23,
                minute: 0,
                second: 0,
            },
        }
    }

    fn captured_at(day: u8, second: u8) -> CivilTime {
        CivilTime {
            year: 2026,
            month: 8,
            day,
            hour: 12,
            minute: 0,
            second,
        }
    }

    fn verified(file: u64, name: &str, digest: u8, captured: CivilTime) -> StagingEntry {
        StagingEntry {
            file: FileId(file),
            path: DevicePath::new(format!("DCIM/Camera/{name}")),
            size: 100 * u64::from(digest),
            mtime: Timestamp(MTIME),
            durable_bytes: 100 * u64::from(digest),
            digest: Some(Sha256([digest; 32])),
            name_sources: vec![captured],
        }
    }

    fn unverified(file: u64, name: &str) -> StagingEntry {
        StagingEntry {
            digest: None,
            ..verified(file, name, 1, captured_at(1, 0))
        }
    }

    fn plan_of(entries: &[StagingEntry]) -> Plan {
        plan_with(
            entries,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
    }

    fn plan_with(
        entries: &[StagingEntry],
        known_content: &BTreeMap<Sha256, VaultName>,
        taken_names: &BTreeSet<VaultName>,
        missing: &BTreeSet<FileId>,
    ) -> Plan {
        plan(&Inputs {
            device: &device(),
            entries,
            missing,
            known_content,
            taken_names,
            imported_at: imported_at(),
        })
    }

    fn actions(plan: &Plan) -> Vec<(FileId, PlanAction)> {
        plan.entries
            .iter()
            .map(|entry| (entry.file, entry.action.clone()))
            .collect()
    }

    #[test]
    fn a_photo_the_vault_has_never_held_is_imported_under_a_fresh_name() {
        covers!("R-COMMIT-004");
        let batch = [verified(1, "one.jpg", 0xaa, captured_at(1, 0))];

        let plan = plan_of(&batch);

        assert_eq!(
            actions(&plan),
            vec![(
                FileId(1),
                PlanAction::Import {
                    name: VaultName::new("2026-08-01_120000.jpg")
                }
            )]
        );
        assert_eq!(plan.summary.imported, 1);
        assert_eq!(plan.summary.duplicates, 0);
    }

    #[test]
    fn content_the_index_already_holds_becomes_a_duplicate() {
        covers!("R-COMMIT-005");
        let batch = [verified(1, "one.jpg", 0xaa, captured_at(1, 0))];
        let known = BTreeMap::from([(Sha256([0xaa; 32]), VaultName::new("2020-01-01_000000.jpg"))]);

        let plan = plan_with(&batch, &known, &BTreeSet::new(), &BTreeSet::new());

        assert_eq!(
            actions(&plan),
            vec![(
                FileId(1),
                PlanAction::Duplicate {
                    name: VaultName::new("2020-01-01_000000.jpg")
                }
            )]
        );
        assert!(plan.contents.is_empty());
        assert_eq!(plan.summary.duplicates, 1);
    }

    #[test]
    fn a_duplicate_still_earns_the_phone_an_index_row() {
        covers!("R-COMMIT-005", "R-INDEX-002");
        let batch = [verified(1, "one.jpg", 0xaa, captured_at(1, 0))];
        let known = BTreeMap::from([(Sha256([0xaa; 32]), VaultName::new("2020-01-01_000000.jpg"))]);

        let plan = plan_with(&batch, &known, &BTreeSet::new(), &BTreeSet::new());

        assert_eq!(plan.device_files.len(), 1);
        assert_eq!(
            plan.device_files[0].vault_name,
            VaultName::new("2020-01-01_000000.jpg")
        );
        assert_eq!(
            plan.device_files[0].path,
            DevicePath::new("DCIM/Camera/one.jpg")
        );
    }

    #[test]
    fn the_same_photo_twice_in_one_batch_is_imported_once() {
        covers!("R-COMMIT-005");
        let batch = [
            verified(1, "one.jpg", 0xaa, captured_at(1, 0)),
            verified(2, "copy.jpg", 0xaa, captured_at(2, 0)),
        ];

        let plan = plan_of(&batch);

        assert_eq!(
            actions(&plan),
            vec![
                (
                    FileId(1),
                    PlanAction::Import {
                        name: VaultName::new("2026-08-01_120000.jpg")
                    }
                ),
                (
                    FileId(2),
                    PlanAction::Duplicate {
                        name: VaultName::new("2026-08-01_120000.jpg")
                    }
                ),
            ]
        );
        assert_eq!(plan.contents.len(), 1);
        assert_eq!(plan.device_files.len(), 2);
    }

    #[test]
    fn an_entry_whose_file_is_gone_is_dropped_rather_than_planned() {
        covers!("R-COMMIT-006");
        let batch = [
            verified(1, "one.jpg", 0xaa, captured_at(1, 0)),
            verified(2, "two.jpg", 0xbb, captured_at(2, 0)),
        ];
        let missing = BTreeSet::from([FileId(1)]);

        let plan = plan_with(&batch, &BTreeMap::new(), &BTreeSet::new(), &missing);

        assert_eq!(plan.dropped, vec![FileId(1)]);
        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.entries[0].file, FileId(2));
        assert_eq!(plan.device_files.len(), 1);
    }

    #[test]
    fn a_partial_transfer_is_not_part_of_the_batch() {
        covers!("R-COMMIT-004");
        let batch = [
            unverified(1, "half.jpg"),
            verified(2, "two.jpg", 0xbb, captured_at(2, 0)),
        ];

        let plan = plan_of(&batch);

        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.entries[0].file, FileId(2));
    }

    #[test]
    fn two_photos_captured_in_the_same_second_take_different_names() {
        covers!("R-NAME-002");
        let batch = [
            verified(1, "one.jpg", 0xaa, captured_at(1, 0)),
            verified(2, "two.jpg", 0xbb, captured_at(1, 0)),
        ];

        let plan = plan_of(&batch);

        assert_eq!(
            actions(&plan),
            vec![
                (
                    FileId(1),
                    PlanAction::Import {
                        name: VaultName::new("2026-08-01_120000.jpg")
                    }
                ),
                (
                    FileId(2),
                    PlanAction::Import {
                        name: VaultName::new("2026-08-01_120000_1.jpg")
                    }
                ),
            ]
        );
    }

    #[test]
    fn a_name_the_vault_already_uses_is_not_taken_again() {
        covers!("R-NAME-002");
        let batch = [verified(1, "one.jpg", 0xaa, captured_at(1, 0))];
        let taken = BTreeSet::from([VaultName::new("2026-08-01_120000.jpg")]);

        let plan = plan_with(&batch, &BTreeMap::new(), &taken, &BTreeSet::new());

        assert_eq!(
            actions(&plan),
            vec![(
                FileId(1),
                PlanAction::Import {
                    name: VaultName::new("2026-08-01_120000_1.jpg")
                }
            )]
        );
    }

    #[test]
    fn the_plan_follows_the_order_the_files_arrived_in() {
        let batch = [
            verified(9, "late.jpg", 0xcc, captured_at(3, 0)),
            verified(2, "early.jpg", 0xaa, captured_at(1, 0)),
            verified(5, "middle.jpg", 0xbb, captured_at(2, 0)),
        ];

        let plan = plan_of(&batch);

        let order: Vec<FileId> = plan.entries.iter().map(|entry| entry.file).collect();
        assert_eq!(order, vec![FileId(2), FileId(5), FileId(9)]);
    }

    #[test]
    fn a_row_records_the_moment_the_batch_committed() {
        covers!("R-INDEX-002");
        let batch = [verified(1, "one.jpg", 0xaa, captured_at(1, 0))];

        let plan = plan_of(&batch);

        assert_eq!(plan.device_files[0].committed_at, imported_at().at);
        assert_eq!(plan.device_files[0].digest, Sha256([0xaa; 32]));
    }

    #[test]
    fn an_empty_manifest_plans_nothing() {
        let plan = plan_of(&[]);

        assert_eq!(plan, Plan::default());
    }

    #[test]
    fn only_imported_bytes_count_as_committed() {
        let batch = [
            verified(1, "one.jpg", 0x02, captured_at(1, 0)),
            verified(2, "copy.jpg", 0x02, captured_at(2, 0)),
        ];

        let plan = plan_of(&batch);

        assert_eq!(plan.summary.bytes_committed, 200);
    }
}
