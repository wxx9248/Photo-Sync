//! Files, as the desktop would find them, including after the power goes out.
//!
//! Two pictures are kept: what a reader sees now, and what a crash would leave behind. Every
//! write lands in the first. What moves it into the second is exactly what moves it onto a
//! disk in reality, and nothing else:
//!
//! * a file's bytes become durable when that file is synced;
//! * a directory entry becomes durable when that directory is synced, which is what makes a
//!   creation, a rename, or a deletion survive;
//! * a crash discards everything the second picture does not hold.
//!
//! This is the whole reason `SPEC.md` §7.3 syncs both directories before it writes a
//! done-mark, and §7.6 truncates a partial back to its watermark. A simulator that made a
//! write durable the moment it happened would agree with the implementation about everything
//! and prove nothing.

use std::collections::BTreeMap;

use photo_sync_core::CivilTime;
use photo_sync_core::id::{DeviceId, FileId, Timestamp, VaultName};
use photo_sync_core::naming;

/// One file in a device's staging directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedFile {
    pub device: DeviceId,
    pub bytes: Vec<u8>,

    /// False while the file still carries the `.part` suffix that marks it unverified.
    pub finalized: bool,
}

/// A whole filesystem at one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Picture {
    staging: BTreeMap<FileId, StagedFile>,
    vault: BTreeMap<VaultName, Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct Storage {
    /// What a reader sees.
    live: Picture,

    /// What a power loss would leave behind.
    durable: Picture,

    /// Bytes a completed sync has made durable, whether or not the directory entry naming
    /// them has. A file synced but never linked durably is a file a crash still loses.
    synced: BTreeMap<FileId, Vec<u8>>,

    /// The same for a file that has since been renamed into the vault.
    synced_vault: BTreeMap<VaultName, Vec<u8>>,

    /// Modification times put on staged files, and the ones they carried into the vault.
    ///
    /// These sit outside the durable picture on purpose. `SPEC.md` §6.4 asks for the phone's
    /// time on the stored copy; it is a detail of the copy, not a claim about what survives,
    /// and a crash that loses it costs nothing the specification promised.
    mtimes: BTreeMap<FileId, Timestamp>,
    vault_mtimes: BTreeMap<VaultName, Timestamp>,

    /// What the shell would read out of a file with these contents.
    name_sources: BTreeMap<Vec<u8>, Vec<CivilTime>>,

    /// What it reads out of a file nothing was said about. A real shell finds a capture time
    /// in most photographs, so answering nothing by default would make the common case the
    /// one no test covers.
    default_name_sources: Vec<CivilTime>,
}

impl Storage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ---- what the desktop asks for ----------------------------------------------------

    pub fn create(&mut self, device: &DeviceId, file: FileId) {
        self.live.staging.insert(
            file,
            StagedFile {
                device: device.clone(),
                bytes: Vec::new(),
                finalized: false,
            },
        );
        self.synced.insert(file, Vec::new());
    }

    pub fn write_at(&mut self, file: FileId, offset: u64, data: &[u8]) {
        let Some(staged) = self.live.staging.get_mut(&file) else {
            return;
        };
        let at = offset as usize;
        if staged.bytes.len() < at {
            staged.bytes.resize(at, 0);
        }
        staged.bytes.truncate(at);
        staged.bytes.extend_from_slice(data);
    }

    /// Makes this file's bytes durable. Its name is a separate question.
    pub fn sync_file(&mut self, file: FileId) {
        let Some(staged) = self.live.staging.get(&file) else {
            return;
        };
        let bytes = staged.bytes.clone();
        self.synced.insert(file, bytes.clone());
        if let Some(kept) = self.durable.staging.get_mut(&file) {
            kept.bytes = bytes;
        }
    }

    /// Makes a directory's entries durable: which files it holds, and under which names.
    pub fn sync_directory(&mut self, directory: &photo_sync_core::Directory) {
        match directory {
            photo_sync_core::Directory::Vault => self.sync_vault_directory(),
            photo_sync_core::Directory::Staging { device } => self.sync_staging_directory(device),
        }
    }

    fn sync_staging_directory(&mut self, device: &DeviceId) {
        self.durable
            .staging
            .retain(|file, kept| kept.device != *device || self.live.staging.contains_key(file));

        for (file, staged) in &self.live.staging {
            if staged.device != *device {
                continue;
            }
            self.durable.staging.insert(
                *file,
                StagedFile {
                    device: staged.device.clone(),
                    bytes: self.synced.get(file).cloned().unwrap_or_default(),
                    finalized: staged.finalized,
                },
            );
        }
    }

    fn sync_vault_directory(&mut self) {
        self.durable
            .vault
            .retain(|name, _| self.live.vault.contains_key(name));
        for name in self.live.vault.keys() {
            let bytes = self.synced_vault.get(name).cloned().unwrap_or_default();
            self.durable.vault.insert(name.clone(), bytes);
        }
    }

    /// Cuts a file back, durably. The shell follows `set_len` with a sync, since a truncation
    /// that a crash could undo would leave the torn tail §7.6 exists to remove.
    pub fn truncate(&mut self, file: FileId, length: u64) {
        if let Some(staged) = self.live.staging.get_mut(&file) {
            staged.bytes.truncate(length as usize);
        }
        if let Some(synced) = self.synced.get_mut(&file) {
            synced.truncate(length as usize);
        }
        if let Some(kept) = self.durable.staging.get_mut(&file) {
            kept.bytes.truncate(length as usize);
        }
    }

    /// Strips the `.part` suffix. A rename inside one directory, durable when it is synced.
    pub fn finalize(&mut self, file: FileId) {
        if let Some(staged) = self.live.staging.get_mut(&file) {
            staged.finalized = true;
        }
    }

    /// Puts the phone's own modification time on a staged file. `SPEC.md` §6.4.
    pub fn set_modified_time(&mut self, file: FileId, mtime: Timestamp) {
        if self.live.staging.contains_key(&file) {
            self.mtimes.insert(file, mtime);
        }
    }

    /// The time the vault copy carries, which is the one its photograph had on the phone.
    #[must_use]
    pub fn vault_mtime(&self, name: &VaultName) -> Option<Timestamp> {
        self.vault_mtimes.get(name).copied()
    }

    pub fn rename_into_vault(&mut self, file: FileId, name: &VaultName) {
        let Some(staged) = self.live.staging.remove(&file) else {
            return;
        };
        // The time travels with the file, the way a rename carries it on a real filesystem.
        if let Some(mtime) = self.mtimes.remove(&file) {
            self.vault_mtimes.insert(name.clone(), mtime);
        }
        self.live.vault.insert(name.clone(), staged.bytes);
        // The bytes were made durable while the file was in staging; the rename moves that
        // fact along with the name. Both directories still have to be synced for either.
        let synced = self.synced.remove(&file).unwrap_or_default();
        self.synced_vault.insert(name.clone(), synced);
    }

    pub fn remove(&mut self, file: FileId) {
        self.live.staging.remove(&file);
        self.mtimes.remove(&file);
        self.synced.remove(&file);
    }

    pub fn clear_staging(&mut self, device: &DeviceId) {
        self.live.staging.retain(|file, staged| {
            let mine = staged.device == *device;
            if mine {
                self.synced.remove(file);
                self.mtimes.remove(file);
            }
            !mine
        });
    }

    /// Everything the machine did not get around to writing down is gone.
    ///
    /// A file keeps only the bytes a sync had proven, but not necessarily its length: a
    /// buffered write that never reached the disk can still have moved the file's size, and
    /// what lies past the durable data is then whatever the disk held. That is the trap
    /// `SPEC.md` §7.6 exists to close, and a crash that quietly cut every file back to its
    /// durable data would close it for free and prove nothing. So the tail comes back as
    /// zeroes, which is the shape a resumed transfer would silently accept.
    pub fn crash(&mut self) {
        let lengths: BTreeMap<FileId, usize> = self
            .live
            .staging
            .iter()
            .map(|(file, staged)| (*file, staged.bytes.len()))
            .collect();

        self.live = self.durable.clone();
        for (file, staged) in &mut self.live.staging {
            if let Some(length) = lengths.get(file)
                && *length > staged.bytes.len()
            {
                staged.bytes.resize(*length, 0);
            }
        }

        self.synced
            .retain(|file, _| self.durable.staging.contains_key(file));
        for (file, staged) in &self.durable.staging {
            self.synced.insert(*file, staged.bytes.clone());
        }
        self.synced_vault
            .retain(|name, _| self.durable.vault.contains_key(name));
    }

    #[must_use]
    pub fn stat_vault(&self, name: &VaultName) -> Option<u64> {
        self.live.vault.get(name).map(|bytes| bytes.len() as u64)
    }

    /// Reads part of a staged file back, as the shell would.
    #[must_use]
    pub fn read_range(&self, file: FileId, offset: u64, length: u64) -> Vec<u8> {
        let Some(staged) = self.live.staging.get(&file) else {
            return Vec::new();
        };
        let from = (offset as usize).min(staged.bytes.len());
        let to = (from + length as usize).min(staged.bytes.len());
        staged.bytes[from..to].to_vec()
    }

    #[must_use]
    pub fn holds(&self, file: FileId) -> bool {
        self.live.staging.contains_key(&file)
    }

    /// The readings a vault name may be built from, in the order `SPEC.md` §7.2 tries them:
    /// what the photograph says about itself, and then when it was last modified.
    #[must_use]
    pub fn name_sources(&self, file: FileId, mtime: Timestamp) -> Vec<CivilTime> {
        let Some(staged) = self.live.staging.get(&file) else {
            return Vec::new();
        };
        let mut sources = self
            .name_sources
            .get(&staged.bytes)
            .cloned()
            .unwrap_or_else(|| self.default_name_sources.clone());
        sources.push(naming::civil_from_unix(mtime.0));
        sources
    }

    // ---- what a test says and asks -----------------------------------------------------

    /// Says what the shell would read out of a photograph with these contents.
    pub fn set_capture_time(&mut self, content: &[u8], captured: CivilTime) {
        self.name_sources.insert(content.to_vec(), vec![captured]);
    }

    /// Says what it reads out of every file nothing was said about.
    pub fn set_default_name_sources(&mut self, sources: Vec<CivilTime>) {
        self.default_name_sources = sources;
    }

    /// Puts a file in the vault, durably, as an earlier session would have left it.
    pub fn put_in_vault(&mut self, name: &VaultName, bytes: Vec<u8>) {
        self.live.vault.insert(name.clone(), bytes.clone());
        self.synced_vault.insert(name.clone(), bytes.clone());
        self.durable.vault.insert(name.clone(), bytes);
    }

    /// Takes a file out of the vault, as a person curating it would.
    pub fn curate(&mut self, name: &VaultName) {
        self.live.vault.remove(name);
        self.durable.vault.remove(name);
        self.synced_vault.remove(name);
    }

    #[must_use]
    pub fn vault(&self) -> &BTreeMap<VaultName, Vec<u8>> {
        &self.live.vault
    }

    #[must_use]
    pub fn staging(&self) -> &BTreeMap<FileId, StagedFile> {
        &self.live.staging
    }

    /// What a crash right now would leave in the vault. Only a test looks at this.
    #[must_use]
    pub fn durable_vault(&self) -> &BTreeMap<VaultName, Vec<u8>> {
        &self.durable.vault
    }

    /// What a crash right now would leave in staging.
    #[must_use]
    pub fn durable_staging(&self) -> &BTreeMap<FileId, StagedFile> {
        &self.durable.staging
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_sync_core::Directory;

    fn phone() -> DeviceId {
        DeviceId::new("phone-a")
    }

    fn staging() -> Directory {
        Directory::Staging { device: phone() }
    }

    fn written(storage: &mut Storage, file: FileId, bytes: &[u8]) {
        storage.create(&phone(), file);
        storage.write_at(file, 0, bytes);
    }

    #[test]
    fn a_write_alone_does_not_survive() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");

        storage.crash();

        assert!(!storage.holds(FileId(1)));
    }

    #[test]
    fn a_file_synced_but_never_linked_does_not_survive() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");

        // Syncing a file makes its bytes durable and says nothing about its name.
        storage.sync_file(FileId(1));
        storage.crash();

        assert!(!storage.holds(FileId(1)));
    }

    #[test]
    fn a_file_survives_once_both_it_and_its_directory_are_synced() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");

        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());
        storage.crash();

        assert_eq!(
            storage
                .staging()
                .get(&FileId(1))
                .map(|file| file.bytes.clone()),
            Some(b"one photograph".to_vec())
        );
    }

    #[test]
    fn bytes_written_after_the_last_sync_come_back_as_nothing() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"half a");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());

        storage.write_at(FileId(1), 6, b" photograph");
        storage.crash();

        // The length moved even though the bytes did not, so the file is longer than
        // anything that was proven and its tail is not the photograph. Reading it back as
        // though it were is exactly what §7.6's watermark stops.
        let mut expected = b"half a".to_vec();
        expected.resize(17, 0);
        assert_eq!(
            storage
                .staging()
                .get(&FileId(1))
                .map(|file| file.bytes.clone()),
            Some(expected)
        );
    }

    #[test]
    fn what_a_crash_kept_is_only_what_was_proven() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"half a");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());

        storage.write_at(FileId(1), 6, b" photograph");
        storage.crash();

        assert_eq!(
            storage
                .durable_staging()
                .get(&FileId(1))
                .map(|file| file.bytes.clone()),
            Some(b"half a".to_vec())
        );
    }

    #[test]
    fn a_directory_synced_before_the_file_leaves_the_name_without_the_bytes() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");

        // The trap SPEC.md §7.6 is written against: a name can outlive its contents, and the
        // file is the right length while holding none of them.
        storage.sync_directory(&staging());
        storage.crash();

        assert_eq!(
            storage
                .durable_staging()
                .get(&FileId(1))
                .map(|file| file.bytes.clone()),
            Some(Vec::new())
        );
        assert_eq!(
            storage
                .staging()
                .get(&FileId(1))
                .map(|file| file.bytes.clone()),
            Some(vec![0; 14])
        );
    }

    #[test]
    fn a_rename_into_the_vault_needs_both_directories_synced() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());
        let name = VaultName::new("2026-08-01_123456.jpg");

        storage.rename_into_vault(FileId(1), &name);
        storage.crash();

        // Neither directory was synced after the rename, so it did not happen.
        assert!(storage.vault().is_empty());
        assert!(storage.holds(FileId(1)));
    }

    #[test]
    fn a_rename_survives_once_both_directories_are_synced() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());
        let name = VaultName::new("2026-08-01_123456.jpg");

        storage.rename_into_vault(FileId(1), &name);
        storage.sync_directory(&Directory::Vault);
        storage.sync_directory(&staging());
        storage.crash();

        assert_eq!(
            storage.vault().get(&name),
            Some(&b"one photograph".to_vec())
        );
        assert!(!storage.holds(FileId(1)));
    }

    #[test]
    fn a_deletion_that_was_not_synced_comes_back() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());

        storage.remove(FileId(1));
        storage.crash();

        assert!(storage.holds(FileId(1)));
    }

    #[test]
    fn a_truncation_is_durable_because_the_shell_syncs_it() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());

        storage.truncate(FileId(1), 3);
        storage.crash();

        assert_eq!(
            storage
                .staging()
                .get(&FileId(1))
                .map(|file| file.bytes.clone()),
            Some(b"one".to_vec())
        );
    }

    #[test]
    fn stripping_the_suffix_needs_its_directory_synced_too() {
        let mut storage = Storage::new();
        written(&mut storage, FileId(1), b"one photograph");
        storage.sync_file(FileId(1));
        storage.sync_directory(&staging());

        storage.finalize(FileId(1));
        storage.crash();

        assert_eq!(
            storage.staging().get(&FileId(1)).map(|file| file.finalized),
            Some(false)
        );
    }

    #[test]
    fn a_vault_a_test_puts_files_in_is_already_durable() {
        let mut storage = Storage::new();
        let name = VaultName::new("2020-01-01_000000.jpg");
        storage.put_in_vault(&name, b"an older photograph".to_vec());

        storage.crash();

        assert_eq!(storage.stat_vault(&name), Some(19));
    }
}
