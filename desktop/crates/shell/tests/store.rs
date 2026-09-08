//! The index and the staging manifests, against real SQLite files.
//!
//! What matters here is that the store answers the requests the core makes, that the keys are
//! the ones `SPEC.md` chose, and that state written by one run is found by the next. The
//! ordering between these writes and the filesystem is the core's business, and the
//! simulator's.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::store::Store;
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};
use photo_sync_core::naming::CivilTime;
use photo_sync_core::store::{
    ContentRow, DeviceFileRow, PlanAction, PlanEntry, StagingEntry, StoreRequest, StoreResponse,
};

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }
        Self { path }
    }

    fn open(&self) -> Store {
        match Store::open(&self.path.join("Camera"), &self.path.join("data/index.db")) {
            Ok(store) => store,
            Err(error) => panic!("cannot open the store: {error}"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn phone() -> DeviceId {
    DeviceId::new("phone-a")
}

fn ask(store: &mut Store, request: &StoreRequest) -> StoreResponse {
    match store.run(request) {
        Ok(response) => response,
        Err(error) => panic!("the store refused {request:?}: {error}"),
    }
}

fn captured() -> CivilTime {
    CivilTime {
        year: 2026,
        month: 8,
        day: 1,
        hour: 12,
        minute: 34,
        second: 56,
    }
}

fn entry(file: u64, path: &str) -> StagingEntry {
    StagingEntry {
        file: FileId(file),
        path: DevicePath::new(path),
        size: 2400,
        mtime: Timestamp(1_756_000_000),
        durable_bytes: 0,
        digest: None,
        name_sources: Vec::new(),
    }
}

fn row(path: &str, name: &str) -> DeviceFileRow {
    DeviceFileRow {
        device: phone(),
        path: DevicePath::new(path),
        size: 2400,
        mtime: Timestamp(1_756_000_000),
        digest: Sha256([9; 32]),
        vault_name: VaultName::new(name),
        committed_at: Timestamp(1_757_000_000),
    }
}

fn staged(store: &mut Store, device: &DeviceId) -> Vec<StagingEntry> {
    match ask(
        store,
        &StoreRequest::ListStagingEntries {
            device: device.clone(),
        },
    ) {
        StoreResponse::StagingEntries(entries) => entries,
        other => panic!("the store answered with {other:?}"),
    }
}

#[test]
fn a_staged_entry_is_written_and_read_back() {
    let scratch = Scratch::new();
    let mut store = scratch.open();

    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: phone(),
            entry: entry(1, "DCIM/Camera/IMG_0001.jpg"),
        },
    );

    let entries = staged(&mut store, &phone());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, DevicePath::new("DCIM/Camera/IMG_0001.jpg"));
    assert_eq!(entries[0].durable_bytes, 0);
    assert!(entries[0].digest.is_none());
}

#[test]
fn a_watermark_and_a_digest_reach_the_entry_they_name() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: phone(),
            entry: entry(1, "DCIM/Camera/IMG_0001.jpg"),
        },
    );

    ask(
        &mut store,
        &StoreRequest::AdvanceWatermark {
            file: FileId(1),
            durable_bytes: 1600,
        },
    );
    ask(
        &mut store,
        &StoreRequest::MarkStagingEntryVerified {
            file: FileId(1),
            digest: Sha256([7; 32]),
            name_sources: vec![captured()],
        },
    );

    let entries = staged(&mut store, &phone());
    assert_eq!(entries[0].durable_bytes, 1600);
    assert_eq!(entries[0].digest, Some(Sha256([7; 32])));
    assert_eq!(entries[0].name_sources, vec![captured()]);
}

#[test]
fn manifests_are_per_device_and_do_not_see_each_other() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    let other = DeviceId::new("phone-b");

    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: phone(),
            entry: entry(1, "DCIM/Camera/IMG_0001.jpg"),
        },
    );
    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: other.clone(),
            entry: entry(2, "DCIM/Camera/IMG_0002.jpg"),
        },
    );

    assert_eq!(staged(&mut store, &phone()).len(), 1);
    assert_eq!(staged(&mut store, &other).len(), 1);
    assert_eq!(staged(&mut store, &phone())[0].file, FileId(1));
}

#[test]
fn the_highest_identifier_is_taken_across_every_device() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: DeviceId::new("phone-b"),
            entry: entry(41, "DCIM/Camera/IMG_9999.jpg"),
        },
    );
    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: phone(),
            entry: entry(3, "DCIM/Camera/IMG_0001.jpg"),
        },
    );

    let highest = ask(&mut store, &StoreRequest::HighestStagingFileId);

    assert_eq!(
        highest,
        StoreResponse::HighestStagingFileId(Some(FileId(41)))
    );
}

#[test]
fn a_manifest_written_by_one_run_is_found_by_the_next() {
    let scratch = Scratch::new();
    {
        let mut store = scratch.open();
        ask(
            &mut store,
            &StoreRequest::BeginStagingEntry {
                device: phone(),
                entry: entry(7, "DCIM/Camera/IMG_0001.jpg"),
            },
        );
    }

    let mut restarted = scratch.open();

    assert_eq!(staged(&mut restarted, &phone()).len(), 1);
    // A file named by identifier alone still finds the manifest that holds it.
    ask(
        &mut restarted,
        &StoreRequest::AdvanceWatermark {
            file: FileId(7),
            durable_bytes: 900,
        },
    );
    assert_eq!(staged(&mut restarted, &phone())[0].durable_bytes, 900);
    assert_eq!(
        ask(&mut restarted, &StoreRequest::HighestStagingFileId),
        StoreResponse::HighestStagingFileId(Some(FileId(7)))
    );
}

#[test]
fn a_sealed_plan_comes_back_in_the_order_it_was_written() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    let plan = vec![
        PlanEntry {
            file: FileId(2),
            action: PlanAction::Import {
                name: VaultName::new("2026-08-01_123456.jpg"),
            },
        },
        PlanEntry {
            file: FileId(1),
            action: PlanAction::Duplicate {
                name: VaultName::new("2020-01-01_000000.jpg"),
            },
        },
    ];

    ask(
        &mut store,
        &StoreRequest::SealCommitPlan {
            device: phone(),
            plan: plan.clone(),
        },
    );

    assert_eq!(
        ask(
            &mut store,
            &StoreRequest::LoadCommitPlan { device: phone() }
        ),
        StoreResponse::CommitPlan(Some(plan))
    );
}

#[test]
fn a_staging_area_with_no_plan_says_so() {
    let scratch = Scratch::new();
    let mut store = scratch.open();

    let loaded = ask(
        &mut store,
        &StoreRequest::LoadCommitPlan { device: phone() },
    );

    assert_eq!(loaded, StoreResponse::CommitPlan(None));
}

#[test]
fn clearing_the_plan_leaves_the_manifest_alone() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    ask(
        &mut store,
        &StoreRequest::BeginStagingEntry {
            device: phone(),
            entry: entry(1, "DCIM/Camera/IMG_0001.jpg"),
        },
    );
    ask(
        &mut store,
        &StoreRequest::SealCommitPlan {
            device: phone(),
            plan: vec![PlanEntry {
                file: FileId(1),
                action: PlanAction::Import {
                    name: VaultName::new("2026-08-01_123456.jpg"),
                },
            }],
        },
    );

    ask(
        &mut store,
        &StoreRequest::ClearCommitPlan { device: phone() },
    );

    assert_eq!(
        ask(
            &mut store,
            &StoreRequest::LoadCommitPlan { device: phone() }
        ),
        StoreResponse::CommitPlan(None)
    );
    assert_eq!(staged(&mut store, &phone()).len(), 1);
}

#[test]
fn a_committed_batch_is_found_by_content_and_by_path() {
    let scratch = Scratch::new();
    let mut store = scratch.open();

    ask(
        &mut store,
        &StoreRequest::InsertCommittedBatch {
            contents: vec![ContentRow {
                digest: Sha256([9; 32]),
                vault_name: VaultName::new("2026-08-01_123456.jpg"),
            }],
            device_files: vec![row("DCIM/Camera/IMG_0001.jpg", "2026-08-01_123456.jpg")],
        },
    );

    let content = ask(
        &mut store,
        &StoreRequest::LookupContent {
            digests: vec![Sha256([9; 32]), Sha256([1; 32])],
        },
    );
    let files = ask(
        &mut store,
        &StoreRequest::LookupDeviceFiles {
            device: phone(),
            paths: vec![DevicePath::new("DCIM/Camera/IMG_0001.jpg")],
        },
    );

    match content {
        StoreResponse::Content(rows) => assert_eq!(rows.len(), 1),
        other => panic!("the store answered with {other:?}"),
    }
    match files {
        StoreResponse::DeviceFiles(rows) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].digest, Sha256([9; 32]));
            assert_eq!(rows[0].vault_name, VaultName::new("2026-08-01_123456.jpg"));
        }
        other => panic!("the store answered with {other:?}"),
    }
}

#[test]
fn re_importing_a_changed_file_replaces_the_row_it_supersedes() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    ask(
        &mut store,
        &StoreRequest::InsertCommittedBatch {
            contents: Vec::new(),
            device_files: vec![row("DCIM/Camera/IMG_0001.jpg", "first.jpg")],
        },
    );

    ask(
        &mut store,
        &StoreRequest::InsertCommittedBatch {
            contents: Vec::new(),
            device_files: vec![row("DCIM/Camera/IMG_0001.jpg", "second.jpg")],
        },
    );

    match ask(
        &mut store,
        &StoreRequest::LookupDeviceFiles {
            device: phone(),
            paths: vec![DevicePath::new("DCIM/Camera/IMG_0001.jpg")],
        },
    ) {
        StoreResponse::DeviceFiles(rows) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].vault_name, VaultName::new("second.jpg"));
        }
        other => panic!("the store answered with {other:?}"),
    }
}

#[test]
fn names_are_asked_for_by_stem_and_the_stem_is_a_literal() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    ask(
        &mut store,
        &StoreRequest::InsertCommittedBatch {
            contents: vec![
                ContentRow {
                    digest: Sha256([1; 32]),
                    vault_name: VaultName::new("2026-08-01_123456.jpg"),
                },
                ContentRow {
                    digest: Sha256([2; 32]),
                    vault_name: VaultName::new("2026-08-01_123456_1.jpg"),
                },
                ContentRow {
                    digest: Sha256([3; 32]),
                    vault_name: VaultName::new("2026-09-09_000000.jpg"),
                },
            ],
            device_files: Vec::new(),
        },
    );

    match ask(
        &mut store,
        &StoreRequest::TakenVaultNames {
            stems: vec!["2026-08-01_123456".to_string()],
        },
    ) {
        StoreResponse::VaultNames(names) => assert_eq!(names.len(), 2),
        other => panic!("the store answered with {other:?}"),
    }
}

#[test]
fn a_stale_row_can_be_forgotten_so_the_next_diff_sends_it() {
    let scratch = Scratch::new();
    let mut store = scratch.open();
    ask(
        &mut store,
        &StoreRequest::InsertCommittedBatch {
            contents: Vec::new(),
            device_files: vec![row("DCIM/Camera/IMG_0001.jpg", "first.jpg")],
        },
    );

    ask(
        &mut store,
        &StoreRequest::ForgetDeviceFile {
            device: phone(),
            path: DevicePath::new("DCIM/Camera/IMG_0001.jpg"),
        },
    );

    match ask(
        &mut store,
        &StoreRequest::LookupDeviceFiles {
            device: phone(),
            paths: vec![DevicePath::new("DCIM/Camera/IMG_0001.jpg")],
        },
    ) {
        StoreResponse::DeviceFiles(rows) => assert!(rows.is_empty()),
        other => panic!("the store answered with {other:?}"),
    }
}
