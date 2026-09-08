//! What the desktop knows about one phone's session.
//!
//! The session records the frozen catalog, the classification derived from it, and how far
//! each transfer has got. It decides nothing about ordering or effects; `desktop` does that.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{Catalog, CatalogEntry};
use crate::digest::RunningDigest;
use crate::effect::DiffSummary;
use crate::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp};
use crate::store::{DeviceFileRow, StagingEntry};

/// Hands out staging identifiers.
///
/// Identifiers are unique across devices rather than per device, because the manifest requests
/// that name a staged file carry the identifier alone, and so does every effect that touches
/// it. The counter therefore starts above the highest identifier surviving in the manifest,
/// which the desktop reads once at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIds {
    next: u64,
}

impl FileIds {
    pub(crate) fn starting_above(highest: Option<FileId>) -> Self {
        Self {
            next: highest.map_or(0, |FileId(value)| value + 1),
        }
    }

    pub(crate) fn allocate(&mut self) -> FileId {
        let allocated = FileId(self.next);
        self.next += 1;
        allocated
    }
}

/// How far the session has got. A phone drives every exchange, so the phase says which event
/// the desktop expects from it next.
#[derive(Debug)]
pub(crate) enum Phase {
    /// Connected. The catalog comes first, including after a reconnection.
    AwaitingCatalog,

    /// The catalog is frozen and the phone asks for the diff next.
    CatalogFrozen,

    /// The store lookups behind the diff are outstanding.
    Classifying(Box<Inputs>),

    /// The classification is done and the free-space answer is outstanding.
    MeasuringSpace,

    /// The phone holds the list and is uploading.
    Transferring,

    /// The session was turned away. Nothing further is accepted from it.
    Rejected,
}

/// The two answers the diff waits on.
#[derive(Debug, Default)]
pub(crate) struct Inputs {
    pub staged: Option<Vec<StagingEntry>>,
    pub imported: Option<Vec<DeviceFileRow>>,
}

impl Inputs {
    /// Both answers are in and the catalog can be classified.
    fn is_complete(&self) -> bool {
        self.staged.is_some() && self.imported.is_some()
    }
}

/// One file the desktop asked the phone for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedSend {
    pub file: FileId,
    pub path: DevicePath,
    pub size: u64,
    pub mtime: Timestamp,

    /// First byte the desktop still needs, which is always its own durable watermark.
    pub resume_offset: u64,

    /// A mismatched digest buys one full re-transfer before the file is skipped.
    pub attempts: u32,
}

/// The outcome of classifying a catalog against what the desktop already holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Classification {
    pub to_send: Vec<PlannedSend>,

    /// Staging entries for a path whose file has since changed on the phone. Both the manifest
    /// row and the staged file go, so a re-transfer cannot be confused with the older version.
    pub superseded: Vec<FileId>,

    pub summary: DiffSummary,
}

/// Partitions a frozen catalog into imported, already staged, and to send.
///
/// `resumable` names the partials whose digest the desktop can still continue, which is what
/// decides between resuming a partial and starting it again. A partial the desktop cannot
/// continue is superseded rather than trusted: `SPEC.md` §7.6 makes resume an optimization,
/// never an assumption, so falling back to a full transfer is always correct.
pub(crate) fn classify(
    catalog: &Catalog,
    staged: &[StagingEntry],
    imported: &[DeviceFileRow],
    resumable: &BTreeSet<FileId>,
    ids: &mut FileIds,
) -> Classification {
    let imported_by_path: BTreeMap<&DevicePath, &DeviceFileRow> =
        imported.iter().map(|row| (&row.path, row)).collect();
    let staged_by_path = group_by_path(staged);

    let mut classification = Classification::default();

    for entry in catalog.entries() {
        if is_imported(imported_by_path.get(&entry.path).copied(), entry) {
            classification.summary.already_imported += 1;
            continue;
        }

        let staged_here = staged_by_path
            .get(&entry.path)
            .map(Vec::as_slice)
            .unwrap_or_default();

        let (current, stale): (Vec<&StagingEntry>, Vec<&StagingEntry>) = staged_here
            .iter()
            .copied()
            .partition(|staged| describes(staged, entry));
        classification
            .superseded
            .extend(stale.iter().map(|staged| staged.file));

        classify_entry(
            entry,
            current.first().copied(),
            resumable,
            ids,
            &mut classification,
        );
    }

    classification
}

fn classify_entry(
    entry: &CatalogEntry,
    staged: Option<&StagingEntry>,
    resumable: &BTreeSet<FileId>,
    ids: &mut FileIds,
    into: &mut Classification,
) {
    let resume = match staged {
        Some(staged) if staged.is_verified() => {
            into.summary.already_staged += 1;
            return;
        }
        Some(staged) if resumable.contains(&staged.file) => Some(staged),
        Some(staged) => {
            into.superseded.push(staged.file);
            None
        }
        None => None,
    };

    let send = PlannedSend {
        file: resume.map_or_else(|| ids.allocate(), |staged| staged.file),
        path: entry.path.clone(),
        size: entry.size,
        mtime: entry.mtime,
        resume_offset: resume.map_or(0, |staged| staged.durable_bytes),
        attempts: 0,
    };

    into.summary.to_send_count += 1;
    into.summary.to_send_bytes += send.size.saturating_sub(send.resume_offset);
    into.to_send.push(send);
}

/// Groups staging entries by the path they hold, ordered by identifier so a run replays the
/// same way whatever order the store answered in.
fn group_by_path(staged: &[StagingEntry]) -> BTreeMap<&DevicePath, Vec<&StagingEntry>> {
    let mut grouped: BTreeMap<&DevicePath, Vec<&StagingEntry>> = BTreeMap::new();
    for entry in staged {
        grouped.entry(&entry.path).or_default().push(entry);
    }
    for entries in grouped.values_mut() {
        entries.sort_by_key(|entry| entry.file);
    }
    grouped
}

fn is_imported(row: Option<&DeviceFileRow>, entry: &CatalogEntry) -> bool {
    row.is_some_and(|row| row.size == entry.size && row.mtime == entry.mtime)
}

/// Whether a staging entry holds the same version of the file the catalog describes.
fn describes(staged: &StagingEntry, entry: &CatalogEntry) -> bool {
    staged.size == entry.size && staged.mtime == entry.mtime
}

/// A transfer in flight.
#[derive(Debug)]
pub(crate) struct Upload {
    pub size: u64,

    /// Kept so the readings a vault name may be built from can be taken against it.
    pub mtime: Timestamp,

    /// Absolute offset of the next byte expected from the phone.
    pub received: u64,

    /// Bytes a completed sync has made durable.
    pub durable: u64,

    /// Bytes written since the last sync, which decides when the next one falls due.
    pub unsynced: u64,

    /// The digest of everything received so far.
    pub digest: RunningDigest,

    pub state: UploadState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadState {
    Receiving,

    /// The phone stated its digest and the last sync is outstanding.
    Closing {
        claimed: Sha256,
    },

    /// Finished or abandoned. Further chunks for this file are ignored.
    Closed,
}

/// One phone, from its handshake to its last verified file.
///
/// The fields are visible to the rest of the crate because `desktop` is the only module that
/// drives a session, and accessors for each of them would say nothing the names do not.
#[derive(Debug)]
pub(crate) struct Session {
    pub device: DeviceId,
    pub phase: Phase,
    pub catalog: Catalog,
    pub summary: DiffSummary,
    pub sends: BTreeMap<FileId, PlannedSend>,

    /// The identifiers in `sends`, kept in the order the catalog listed them.
    order: Vec<FileId>,

    pub uploads: BTreeMap<FileId, Upload>,
}

impl Session {
    pub(crate) fn connected(device: DeviceId) -> Self {
        Self {
            device,
            phase: Phase::AwaitingCatalog,
            catalog: Catalog::default(),
            summary: DiffSummary::default(),
            sends: BTreeMap::new(),
            order: Vec::new(),
            uploads: BTreeMap::new(),
        }
    }

    /// Records the catalog the phone froze at session start. A reconnection sends the same
    /// list again, so replacing it wholesale is what `SPEC.md` §6 describes.
    pub(crate) fn freeze(&mut self, catalog: Catalog) {
        self.catalog = catalog;
        self.phase = Phase::CatalogFrozen;
    }

    pub(crate) fn begin_classifying(&mut self) {
        self.phase = Phase::Classifying(Box::default());
    }

    /// Stores one of the two diff answers and says whether both have now arrived.
    pub(crate) fn record_input(&mut self, input: Input) -> bool {
        let Phase::Classifying(inputs) = &mut self.phase else {
            return false;
        };
        match input {
            Input::Staged(entries) => inputs.staged = Some(entries),
            Input::Imported(rows) => inputs.imported = Some(rows),
        }
        inputs.is_complete()
    }

    /// Takes both diff answers, leaving the session waiting for the free-space measurement.
    pub(crate) fn take_inputs(&mut self) -> Option<(Vec<StagingEntry>, Vec<DeviceFileRow>)> {
        let Phase::Classifying(inputs) = &mut self.phase else {
            return None;
        };
        let (Some(staged), Some(imported)) = (inputs.staged.take(), inputs.imported.take()) else {
            return None;
        };
        self.phase = Phase::MeasuringSpace;
        Some((staged, imported))
    }

    pub(crate) fn plan(&mut self, classification: Classification) {
        self.summary = classification.summary;
        self.order = classification
            .to_send
            .iter()
            .map(|send| send.file)
            .collect();
        self.sends = classification
            .to_send
            .into_iter()
            .map(|send| (send.file, send))
            .collect();
    }

    /// The files to send, in catalog order. The map beside it answers lookups by identifier;
    /// this says what the phone is told to work through.
    pub(crate) fn in_order(&self) -> impl Iterator<Item = &PlannedSend> {
        self.order.iter().filter_map(|file| self.sends.get(file))
    }

    /// Bytes the transfer still has to deliver, which is what the free-space check tests.
    pub(crate) fn to_send_bytes(&self) -> u64 {
        self.summary.to_send_bytes
    }
}

pub(crate) enum Input {
    Staged(Vec<StagingEntry>),
    Imported(Vec<DeviceFileRow>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::covers;
    use crate::id::{Sha256, VaultName};

    fn path(name: &str) -> DevicePath {
        DevicePath::new(format!("DCIM/Camera/{name}"))
    }

    fn catalogued(name: &str, size: u64, mtime: i64) -> CatalogEntry {
        CatalogEntry {
            path: path(name),
            size,
            mtime: Timestamp(mtime),
        }
    }

    fn staged(file: u64, name: &str, size: u64, mtime: i64) -> StagingEntry {
        StagingEntry {
            file: FileId(file),
            path: path(name),
            size,
            mtime: Timestamp(mtime),
            durable_bytes: 0,
            digest: None,
            name_sources: Vec::new(),
        }
    }

    fn verified(entry: StagingEntry) -> StagingEntry {
        StagingEntry {
            digest: Some(Sha256([7; 32])),
            ..entry
        }
    }

    fn partial(entry: StagingEntry, durable_bytes: u64) -> StagingEntry {
        StagingEntry {
            durable_bytes,
            ..entry
        }
    }

    fn imported(name: &str, size: u64, mtime: i64) -> DeviceFileRow {
        DeviceFileRow {
            device: DeviceId::new("phone"),
            path: path(name),
            size,
            mtime: Timestamp(mtime),
            digest: Sha256([1; 32]),
            vault_name: VaultName::new("2026-08-01_120000.jpg"),
            committed_at: Timestamp(1_756_000_000),
        }
    }

    /// Classifies against a desktop that can continue every partial it holds.
    fn classification_of(
        entries: Vec<CatalogEntry>,
        staged: &[StagingEntry],
        rows: &[DeviceFileRow],
    ) -> Classification {
        let resumable = staged.iter().map(|entry| entry.file).collect();
        classify_with(entries, staged, rows, &resumable)
    }

    fn classify_with(
        entries: Vec<CatalogEntry>,
        staged: &[StagingEntry],
        rows: &[DeviceFileRow],
        resumable: &BTreeSet<FileId>,
    ) -> Classification {
        let total = entries.iter().map(|entry| entry.size).sum();
        let catalog = Catalog::new(entries, total);
        let mut ids = FileIds::starting_above(Some(FileId(100)));
        classify(&catalog, staged, rows, resumable, &mut ids)
    }

    #[test]
    fn a_file_the_index_already_holds_is_not_sent() {
        covers!("R-DIFF-001");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_000)];

        let result = classification_of(catalog, &[], &[imported("one.jpg", 2400, 1_756_000_000)]);

        assert!(result.to_send.is_empty());
        assert_eq!(result.summary.already_imported, 1);
    }

    #[test]
    fn an_index_row_with_a_different_mtime_does_not_count_as_imported() {
        covers!("R-DIFF-001");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_009)];

        let result = classification_of(catalog, &[], &[imported("one.jpg", 2400, 1_756_000_000)]);

        assert_eq!(result.summary.already_imported, 0);
        assert_eq!(result.to_send.len(), 1);
    }

    #[test]
    fn a_verified_staging_entry_is_not_sent_again() {
        covers!("R-DIFF-002");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_000)];
        let staging = [verified(staged(1, "one.jpg", 2400, 1_756_000_000))];

        let result = classification_of(catalog, &staging, &[]);

        assert!(result.to_send.is_empty());
        assert_eq!(result.summary.already_staged, 1);
    }

    #[test]
    fn a_partial_the_desktop_can_continue_is_offered_from_its_watermark() {
        covers!("R-DIFF-003", "R-STAGE-008");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_000)];
        let staging = [partial(staged(1, "one.jpg", 2400, 1_756_000_000), 1600)];

        let result = classification_of(catalog, &staging, &[]);

        assert_eq!(result.to_send.len(), 1);
        assert_eq!(result.to_send[0].file, FileId(1));
        assert_eq!(result.to_send[0].resume_offset, 1600);
    }

    #[test]
    fn a_partial_the_desktop_cannot_continue_restarts_from_zero() {
        covers!("R-DIFF-003");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_000)];
        let staging = [partial(staged(1, "one.jpg", 2400, 1_756_000_000), 1600)];

        let result = classify_with(catalog, &staging, &[], &BTreeSet::new());

        assert_eq!(result.superseded, vec![FileId(1)]);
        assert_eq!(result.to_send[0].resume_offset, 0);
        assert_ne!(result.to_send[0].file, FileId(1));
    }

    #[test]
    fn a_partial_whose_file_changed_on_the_phone_is_superseded_and_restarted() {
        covers!("R-DIFF-004", "R-STAGE-002");
        let catalog = vec![catalogued("one.jpg", 3000, 1_756_000_500)];
        let staging = [partial(staged(1, "one.jpg", 2400, 1_756_000_000), 1600)];

        let result = classification_of(catalog, &staging, &[]);

        assert_eq!(result.superseded, vec![FileId(1)]);
        assert_eq!(result.to_send.len(), 1);
        assert_eq!(result.to_send[0].resume_offset, 0);
        assert_eq!(result.to_send[0].size, 3000);
    }

    #[test]
    fn a_staged_entry_whose_photo_kept_its_size_but_changed_is_superseded() {
        covers!("R-DIFF-004");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_500)];
        let staging = [partial(staged(1, "one.jpg", 2400, 1_756_000_000), 1600)];

        let result = classification_of(catalog, &staging, &[]);

        assert_eq!(result.superseded, vec![FileId(1)]);
        assert_eq!(result.to_send[0].resume_offset, 0);
    }

    #[test]
    fn a_staged_entry_whose_photo_kept_its_time_but_changed_is_superseded() {
        covers!("R-DIFF-004");
        let catalog = vec![catalogued("one.jpg", 3000, 1_756_000_000)];
        let staging = [partial(staged(1, "one.jpg", 2400, 1_756_000_000), 1600)];

        let result = classification_of(catalog, &staging, &[]);

        assert_eq!(result.superseded, vec![FileId(1)]);
        assert_eq!(result.to_send[0].resume_offset, 0);
    }

    #[test]
    fn a_verified_staging_entry_for_an_older_version_is_superseded() {
        covers!("R-STAGE-002");
        let catalog = vec![catalogued("one.jpg", 3000, 1_756_000_500)];
        let staging = [verified(staged(1, "one.jpg", 2400, 1_756_000_000))];

        let result = classification_of(catalog, &staging, &[]);

        assert_eq!(result.superseded, vec![FileId(1)]);
        assert_eq!(result.summary.already_staged, 0);
        assert_eq!(result.to_send.len(), 1);
    }

    #[test]
    fn two_paths_holding_the_same_photo_are_both_sent() {
        covers!("R-STAGE-004");
        let catalog = vec![
            catalogued("one.jpg", 2400, 1_756_000_000),
            catalogued("copy.jpg", 2400, 1_756_000_000),
        ];

        let result = classification_of(catalog, &[], &[]);

        assert_eq!(result.to_send.len(), 2);
        assert_eq!(result.summary.to_send_count, 2);
    }

    #[test]
    fn the_byte_total_counts_only_what_still_has_to_arrive() {
        covers!("R-DIFF-005");
        let catalog = vec![
            catalogued("one.jpg", 2400, 1_756_000_000),
            catalogued("two.jpg", 1000, 1_756_000_000),
        ];
        let staging = [partial(staged(1, "one.jpg", 2400, 1_756_000_000), 1600)];

        let result = classification_of(catalog, &staging, &[]);

        assert_eq!(result.summary.to_send_bytes, 800 + 1000);
    }

    #[test]
    fn a_staged_file_the_catalog_no_longer_lists_is_left_alone() {
        covers!("R-STAGE-002");
        let catalog = vec![catalogued("one.jpg", 2400, 1_756_000_000)];
        let staging = [verified(staged(1, "gone.jpg", 900, 1_755_000_000))];

        let result = classification_of(catalog, &staging, &[]);

        assert!(result.superseded.is_empty());
    }

    #[test]
    fn fresh_identifiers_continue_above_the_highest_the_manifest_holds() {
        covers!("R-STAGE-001");
        let mut ids = FileIds::starting_above(Some(FileId(41)));

        assert_eq!(ids.allocate(), FileId(42));
        assert_eq!(ids.allocate(), FileId(43));
    }

    #[test]
    fn an_empty_manifest_starts_identifiers_at_zero() {
        covers!("R-STAGE-001");
        let mut ids = FileIds::starting_above(None);

        assert_eq!(ids.allocate(), FileId(0));
    }
}
