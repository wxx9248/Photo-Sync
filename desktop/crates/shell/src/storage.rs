//! Staging and the vault, on a real filesystem.
//!
//! This is one half of the boundary the whole project rests on: the core decides what must
//! happen and in what order, and this makes each of those things happen with the call
//! `STACK.md` §3.4 names for it. Nothing here decides anything. If a rule about ordering
//! appears in this file, it is in the wrong place.
//!
//! Staging lives at `<vault>/.staging/<device>/` so that a commit is a rename within one
//! filesystem, which is what makes it atomic and free of a transient second copy (§7.1).

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use photo_sync_core::effect::Directory;
use photo_sync_core::id::{DeviceId, FileId, VaultName};
use photo_sync_core::port::{StorageError, VaultFileFacts};

/// The suffix a staged file carries until its digest has matched. Startup recovery reads it
/// to tell a partial from a file that is ready to commit (`SPEC.md` §7.6).
const PARTIAL: &str = "part";

const STAGING: &str = ".staging";

/// Where one staged file is.
#[derive(Clone, Debug)]
struct Staged {
    device: DeviceId,
    finalized: bool,
}

pub struct Storage {
    vault: PathBuf,

    /// Where each staged file lives, including files an earlier run created. Effects name a
    /// staged file by identifier alone, so the shell is what remembers which device it
    /// belongs to.
    staged: BTreeMap<FileId, Staged>,

    /// Files open for writing now.
    open: BTreeMap<FileId, File>,
}

impl Storage {
    /// Opens a vault, creating it if it is not there, and finds whatever staging still holds.
    ///
    /// The scan is what lets a commit finish work an earlier run began: those files were
    /// never created by this process, and nothing else records where they are.
    pub fn open(vault: &Path) -> Result<Self, StorageError> {
        create_directory(vault)?;
        create_directory(&vault.join(STAGING))?;

        let mut storage = Self {
            vault: vault.to_path_buf(),
            staged: BTreeMap::new(),
            open: BTreeMap::new(),
        };
        storage.index_staging()?;
        Ok(storage)
    }

    fn index_staging(&mut self) -> Result<(), StorageError> {
        for device in read_directory(&self.vault.join(STAGING))? {
            let Some(name) = file_name(&device) else {
                continue;
            };
            if !device.is_dir() {
                continue;
            }
            let device = DeviceId::new(name);
            for file in read_directory(&self.staging_directory(&device))? {
                let Some(name) = file_name(&file) else {
                    continue;
                };
                let (stem, finalized) = match name.strip_suffix(&format!(".{PARTIAL}")) {
                    Some(stem) => (stem, false),
                    None => (name.as_str(), true),
                };
                let Ok(id) = stem.parse::<u64>() else {
                    continue;
                };
                self.staged.insert(
                    FileId(id),
                    Staged {
                        device: device.clone(),
                        finalized,
                    },
                );
            }
        }
        Ok(())
    }

    // ---- paths ---------------------------------------------------------------------------

    fn staging_directory(&self, device: &DeviceId) -> PathBuf {
        self.vault.join(STAGING).join(device.as_str())
    }

    fn staged_path(&self, file: FileId) -> Result<PathBuf, StorageError> {
        let staged = self.staged.get(&file).ok_or(StorageError::NotFound)?;
        let mut path = self
            .staging_directory(&staged.device)
            .join(file.0.to_string());
        if !staged.finalized {
            path.set_extension(PARTIAL);
        }
        Ok(path)
    }

    fn vault_path(&self, name: &VaultName) -> PathBuf {
        self.vault.join(name.as_str())
    }

    // ---- what the core asks for ------------------------------------------------------------

    pub fn create_staging_file(
        &mut self,
        device: &DeviceId,
        file: FileId,
    ) -> Result<(), StorageError> {
        create_directory(&self.staging_directory(device))?;
        self.staged.insert(
            file,
            Staged {
                device: device.clone(),
                finalized: false,
            },
        );

        let path = self.staged_path(file)?;
        let handle = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .map_err(failure)?;
        self.open.insert(file, handle);
        Ok(())
    }

    pub fn write_at(&mut self, file: FileId, offset: u64, data: &[u8]) -> Result<(), StorageError> {
        let handle = self.writable(file)?;
        handle.seek(SeekFrom::Start(offset)).map_err(failure)?;
        handle.write_all(data).map_err(failure)
    }

    /// `fdatasync`, which also flushes the size change needed to read the bytes back.
    pub fn sync_file(&mut self, file: FileId) -> Result<(), StorageError> {
        self.writable(file)?.sync_data().map_err(failure)
    }

    /// Opening a directory read-only and syncing its descriptor is how Linux makes a rename
    /// or a creation durable. `SPEC.md` §7.3 needs this before any done-mark.
    pub fn sync_directory(&self, directory: &Directory) -> Result<(), StorageError> {
        let path = match directory {
            Directory::Vault => self.vault.clone(),
            Directory::Staging { device } => self.staging_directory(device),
        };
        File::open(&path)
            .map_err(failure)?
            .sync_all()
            .map_err(failure)
    }

    pub fn truncate(&mut self, file: FileId, length: u64) -> Result<(), StorageError> {
        let path = self.staged_path(file)?;
        let handle = OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(failure)?;
        handle.set_len(length).map_err(failure)?;
        handle.sync_data().map_err(failure)
    }

    /// Strips the `.part` suffix now that the digest has matched.
    pub fn finalize_staging_file(&mut self, file: FileId) -> Result<(), StorageError> {
        let from = self.staged_path(file)?;
        let Some(staged) = self.staged.get_mut(&file) else {
            return Err(StorageError::NotFound);
        };
        staged.finalized = true;

        let to = self.staged_path(file)?;
        std::fs::rename(&from, &to).map_err(failure)?;
        self.open.remove(&file);
        Ok(())
    }

    pub fn rename_into_vault(
        &mut self,
        file: FileId,
        name: &VaultName,
    ) -> Result<(), StorageError> {
        let from = self.staged_path(file)?;
        std::fs::rename(&from, self.vault_path(name)).map_err(failure)?;
        self.open.remove(&file);
        self.staged.remove(&file);
        Ok(())
    }

    pub fn remove_staging_file(&mut self, file: FileId) -> Result<(), StorageError> {
        let path = self.staged_path(file)?;
        self.open.remove(&file);
        self.staged.remove(&file);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Removing what is already gone is what was wanted.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(failure(error)),
        }
    }

    pub fn clear_staging_directory(&mut self, device: &DeviceId) -> Result<(), StorageError> {
        let directory = self.staging_directory(device);
        for path in read_directory(&directory)? {
            std::fs::remove_file(&path).map_err(failure)?;
        }
        self.staged.retain(|_, staged| staged.device != *device);
        self.open.clear();
        Ok(())
    }

    pub fn stat_vault_file(
        &self,
        name: &VaultName,
    ) -> Result<Option<VaultFileFacts>, StorageError> {
        match std::fs::metadata(self.vault_path(name)) {
            Ok(facts) => Ok(Some(VaultFileFacts { size: facts.len() })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(failure(error)),
        }
    }

    pub fn stat_staging_file(&self, file: FileId) -> Result<bool, StorageError> {
        match self.staged_path(file) {
            Ok(path) => Ok(path.exists()),
            Err(StorageError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn writable(&mut self, file: FileId) -> Result<&mut File, StorageError> {
        if !self.open.contains_key(&file) {
            let path = self.staged_path(file)?;
            let handle = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(failure)?;
            self.open.insert(file, handle);
        }
        self.open.get_mut(&file).ok_or(StorageError::NotFound)
    }
}

fn create_directory(path: &Path) -> Result<(), StorageError> {
    std::fs::create_dir_all(path).map_err(failure)
}

fn read_directory(path: &Path) -> Result<Vec<PathBuf>, StorageError> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(failure(error)),
    };

    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    // Ordered, because a run that reads its own directories in a different order each time
    // cannot be replayed from a trace.
    found.sort();
    Ok(found)
}

fn file_name(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_str()?.to_string())
}

fn failure(error: std::io::Error) -> StorageError {
    match error.kind() {
        std::io::ErrorKind::NotFound => StorageError::NotFound,
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => {
            StorageError::NoSpace
        }
        _ => StorageError::Failed(error.to_string()),
    }
}
