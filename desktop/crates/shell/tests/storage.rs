//! The filesystem adapter, against a real filesystem.
//!
//! These check that the calls land where the layout of `SPEC.md` §7.1 says they should and
//! that each one means what the core assumes. Whether the ordering between them survives a
//! power loss is a different question, and one the simulator answers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use photo_sync::storage::Storage;
use photo_sync_core::covers;
use photo_sync_core::effect::Directory;
use photo_sync_core::id::{DeviceId, FileId, VaultName};
use photo_sync_core::port::StorageError;

/// A directory of its own for each test, removed when the test ends.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "photo-sync-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("cannot make a scratch directory: {error}");
        }
        Self { path }
    }

    fn vault(&self) -> PathBuf {
        self.path.join("Camera")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn storage(scratch: &Scratch) -> Storage {
    match Storage::open(&scratch.vault()) {
        Ok(storage) => storage,
        Err(error) => panic!("cannot open the vault: {error}"),
    }
}

fn phone() -> DeviceId {
    DeviceId::new("phone-a")
}

fn staging_of(vault: &Path, device: &DeviceId) -> PathBuf {
    vault.join(".staging").join(device.as_str())
}

fn contents(path: &Path) -> Vec<u8> {
    match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    }
}

fn expect(result: Result<(), StorageError>) {
    if let Err(error) = result {
        panic!("the operation failed: {error}");
    }
}

#[test]
fn staging_sits_inside_the_vault_so_a_commit_is_a_rename() {
    covers!("R-STAGE-010");
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);

    expect(storage.create_staging_file(&phone(), FileId(1)));

    let partial = staging_of(&scratch.vault(), &phone()).join("1.part");
    assert!(partial.is_file());
}

#[test]
fn bytes_written_are_the_bytes_read_back() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));

    expect(storage.write_at(FileId(1), 0, b"one "));
    expect(storage.write_at(FileId(1), 4, b"photograph"));
    expect(storage.sync_file(FileId(1)));

    let partial = staging_of(&scratch.vault(), &phone()).join("1.part");
    assert_eq!(contents(&partial), b"one photograph".to_vec());
}

#[test]
fn a_verified_file_loses_its_suffix() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));
    expect(storage.write_at(FileId(1), 0, b"one photograph"));

    expect(storage.finalize_staging_file(FileId(1)));

    let staging = staging_of(&scratch.vault(), &phone());
    assert!(!staging.join("1.part").exists());
    assert_eq!(contents(&staging.join("1")), b"one photograph".to_vec());
}

#[test]
fn a_commit_moves_the_file_into_the_vault_under_its_new_name() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));
    expect(storage.write_at(FileId(1), 0, b"one photograph"));
    expect(storage.finalize_staging_file(FileId(1)));

    expect(storage.rename_into_vault(FileId(1), &VaultName::new("2026-08-01_123456.jpg")));

    let vault_copy = scratch.vault().join("2026-08-01_123456.jpg");
    assert_eq!(contents(&vault_copy), b"one photograph".to_vec());
    assert!(!staging_of(&scratch.vault(), &phone()).join("1").exists());
}

#[test]
fn a_partial_is_cut_back_to_the_length_it_is_given() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));
    expect(storage.write_at(FileId(1), 0, b"one photograph"));

    expect(storage.truncate(FileId(1), 3));

    let partial = staging_of(&scratch.vault(), &phone()).join("1.part");
    assert_eq!(contents(&partial), b"one".to_vec());
}

#[test]
fn clearing_staging_takes_the_partials_with_it() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));
    expect(storage.write_at(FileId(1), 0, b"half a"));
    expect(storage.create_staging_file(&phone(), FileId(2)));
    expect(storage.finalize_staging_file(FileId(2)));

    expect(storage.clear_staging_directory(&phone()));

    let staging = staging_of(&scratch.vault(), &phone());
    assert!(staging.is_dir());
    assert!(!staging.join("1.part").exists());
    assert!(!staging.join("2").exists());
}

#[test]
fn a_vault_copy_reports_its_size_and_a_missing_one_reports_nothing() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));
    expect(storage.write_at(FileId(1), 0, b"one photograph"));
    expect(storage.finalize_staging_file(FileId(1)));
    let name = VaultName::new("2026-08-01_123456.jpg");
    expect(storage.rename_into_vault(FileId(1), &name));

    let found = storage.stat_vault_file(&name);
    let missing = storage.stat_vault_file(&VaultName::new("nothing.jpg"));

    assert_eq!(
        found.map(|facts| facts.map(|facts| facts.size)),
        Ok(Some(14))
    );
    assert_eq!(missing.map(|facts| facts.is_none()), Ok(true));
}

#[test]
fn a_staged_file_is_found_again_after_a_restart() {
    let scratch = Scratch::new();
    {
        let mut storage = storage(&scratch);
        expect(storage.create_staging_file(&phone(), FileId(7)));
        expect(storage.write_at(FileId(7), 0, b"one photograph"));
        expect(storage.finalize_staging_file(FileId(7)));
    }

    // A commit for a device that never came back has to move files this process never made.
    let mut restarted = storage(&scratch);

    assert_eq!(restarted.stat_staging_file(FileId(7)), Ok(true));
    expect(restarted.rename_into_vault(FileId(7), &VaultName::new("2026-08-01_123456.jpg")));
    assert_eq!(
        contents(&scratch.vault().join("2026-08-01_123456.jpg")),
        b"one photograph".to_vec()
    );
}

#[test]
fn a_partial_left_by_an_earlier_run_is_still_a_partial() {
    let scratch = Scratch::new();
    {
        let mut storage = storage(&scratch);
        expect(storage.create_staging_file(&phone(), FileId(7)));
        expect(storage.write_at(FileId(7), 0, b"half a photograph"));
    }

    let restarted = storage(&scratch);

    assert_eq!(restarted.stat_staging_file(FileId(7)), Ok(true));
    assert!(
        staging_of(&scratch.vault(), &phone())
            .join("7.part")
            .is_file()
    );
}

#[test]
fn a_file_nobody_staged_is_simply_absent() {
    let scratch = Scratch::new();
    let storage = storage(&scratch);

    assert_eq!(storage.stat_staging_file(FileId(99)), Ok(false));
}

#[test]
fn removing_a_staged_file_twice_is_not_an_error() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));

    expect(storage.remove_staging_file(FileId(1)));

    assert_eq!(storage.stat_staging_file(FileId(1)), Ok(false));
}

#[test]
fn both_directories_a_commit_syncs_can_be_synced() {
    let scratch = Scratch::new();
    let mut storage = storage(&scratch);
    expect(storage.create_staging_file(&phone(), FileId(1)));

    expect(storage.sync_directory(&Directory::Vault));
    expect(storage.sync_directory(&Directory::Staging { device: phone() }));
}
