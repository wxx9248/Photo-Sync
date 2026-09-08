//! The import index and the staging manifests, in SQLite.
//!
//! `STACK.md` §3.5 splits durable state in two. The index lives outside the vault, so
//! curating or relocating the vault never touches what the desktop remembers importing. Each
//! device's staging manifest and commit write-log live together in one file on the vault
//! filesystem, beside the staged files they describe.
//!
//! Putting the write-log in SQLite is what makes `SPEC.md` §7.3's "write the map, flush, seal
//! with an end marker" a single transaction. A half-written log cannot physically exist, so
//! the presence of its rows is the seal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp, VaultName};
use photo_sync_core::naming::CivilTime;
use photo_sync_core::store::{
    ContentRow, DeviceFileRow, PlanAction, PlanEntry, StagingEntry, StoreError, StoreRequest,
    StoreResponse,
};
use rusqlite::{Connection, OptionalExtension, params};

mod index_migrations {
    refinery::embed_migrations!("migrations/index");
}

mod manifest_migrations {
    refinery::embed_migrations!("migrations/manifest");
}

const STAGING: &str = ".staging";
const MANIFEST: &str = "manifest.db";

pub struct Store {
    index: Connection,
    staging_root: PathBuf,

    /// One connection per device, opened when that device is first asked about.
    manifests: BTreeMap<DeviceId, Connection>,

    /// Which device's manifest holds each staged file. Several requests name a file by
    /// identifier alone, and the manifests are per device, so this is what joins the two.
    owner: BTreeMap<FileId, DeviceId>,
}

impl Store {
    /// Opens the index and finds the staging manifests that already exist.
    pub fn open(vault: &Path, index_file: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = index_file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(&error))?;
        }
        let mut index = connect(index_file)?;
        index_migrations::migrations::runner()
            .run(&mut index)
            .map_err(|error| failed(&error))?;

        let mut store = Self {
            index,
            staging_root: vault.join(STAGING),
            manifests: BTreeMap::new(),
            owner: BTreeMap::new(),
        };
        store.index_devices()?;
        Ok(store)
    }

    /// Learns which device owns each staged file, including files an earlier run began.
    fn index_devices(&mut self) -> Result<(), StoreError> {
        let entries = match std::fs::read_dir(&self.staging_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(failed(&error)),
        };

        let mut devices: Vec<DeviceId> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(MANIFEST).is_file())
            .filter_map(|entry| entry.file_name().to_str().map(DeviceId::new))
            .collect();
        devices.sort();

        for device in devices {
            for file in self.file_ids(&device)? {
                self.owner.insert(file, device.clone());
            }
        }
        Ok(())
    }

    fn file_ids(&mut self, device: &DeviceId) -> Result<Vec<FileId>, StoreError> {
        let manifest = self.manifest(device)?;
        let mut statement = manifest
            .prepare("SELECT file FROM staging_entry")
            .map_err(|error| failed(&error))?;
        let ids = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| failed(&error))?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(|error| failed(&error))?;
        Ok(ids.into_iter().map(|id| FileId(id as u64)).collect())
    }

    /// The manifest for one device, opened and migrated the first time it is asked for.
    fn manifest(&mut self, device: &DeviceId) -> Result<&mut Connection, StoreError> {
        if !self.manifests.contains_key(device) {
            let directory = self.staging_root.join(device.as_str());
            std::fs::create_dir_all(&directory).map_err(|error| failed(&error))?;
            let mut connection = connect(&directory.join(MANIFEST))?;
            manifest_migrations::migrations::runner()
                .run(&mut connection)
                .map_err(|error| failed(&error))?;
            self.manifests.insert(device.clone(), connection);
        }
        self.manifests
            .get_mut(device)
            .ok_or_else(|| StoreError::Failed(format!("no manifest for {device}")))
    }

    fn owning(&self, file: FileId) -> Result<DeviceId, StoreError> {
        self.owner
            .get(&file)
            .cloned()
            .ok_or_else(|| StoreError::Failed(format!("no manifest holds {file:?}")))
    }

    pub fn run(&mut self, request: &StoreRequest) -> Result<StoreResponse, StoreError> {
        match request {
            StoreRequest::HighestStagingFileId => self.highest_file_id(),
            StoreRequest::ListStagingDevices => Ok(StoreResponse::Devices(self.devices())),
            StoreRequest::ListStagingEntries { device } => self.list_staging(device),
            StoreRequest::BeginStagingEntry { device, entry } => self.begin_entry(device, entry),
            StoreRequest::AdvanceWatermark {
                file,
                durable_bytes,
            } => self.advance_watermark(*file, *durable_bytes),
            StoreRequest::MarkStagingEntryVerified {
                file,
                digest,
                name_sources,
            } => self.mark_verified(*file, *digest, name_sources),
            StoreRequest::DropStagingEntry { file } => self.drop_entry(*file),
            StoreRequest::SealCommitPlan { device, plan } => self.seal_plan(device, plan),
            StoreRequest::LoadCommitPlan { device } => self.load_plan(device),
            StoreRequest::MarkPlanEntriesDone { device, files } => self.mark_done(device, files),
            StoreRequest::ClearCommitPlan { device } => {
                self.execute(device, "DELETE FROM commit_plan", [])
            }
            StoreRequest::ClearStagingManifest { device } => self.clear_manifest(device),
            StoreRequest::LookupContent { digests } => self.lookup_content(digests),
            StoreRequest::TakenVaultNames { stems } => self.taken_names(stems),
            StoreRequest::LookupDeviceFiles { device, paths } => {
                self.lookup_device_files(device, paths)
            }
            StoreRequest::InsertCommittedBatch {
                contents,
                device_files,
            } => self.insert_batch(contents, device_files),
            StoreRequest::ForgetDeviceFile { device, path } => self.forget(device, path),
        }
    }

    // ---- the staging manifest --------------------------------------------------------------

    fn highest_file_id(&mut self) -> Result<StoreResponse, StoreError> {
        Ok(StoreResponse::HighestStagingFileId(
            self.owner.keys().next_back().copied(),
        ))
    }

    /// Which devices have a manifest at all, in a settled order.
    fn devices(&self) -> Vec<DeviceId> {
        let mut devices: Vec<DeviceId> = self.owner.values().cloned().collect();
        devices.sort();
        devices.dedup();
        devices
    }

    fn list_staging(&mut self, device: &DeviceId) -> Result<StoreResponse, StoreError> {
        let manifest = self.manifest(device)?;
        let mut statement = manifest
            .prepare(
                "SELECT file, path, size, mtime, durable_bytes, digest, name_sources
                 FROM staging_entry ORDER BY file",
            )
            .map_err(|error| failed(&error))?;
        let entries = statement
            .query_map([], read_staging_entry)
            .map_err(|error| failed(&error))?
            .collect::<Result<Vec<StagingEntry>, _>>()
            .map_err(|error| failed(&error))?;
        Ok(StoreResponse::StagingEntries(entries))
    }

    fn begin_entry(
        &mut self,
        device: &DeviceId,
        entry: &StagingEntry,
    ) -> Result<StoreResponse, StoreError> {
        self.owner.insert(entry.file, device.clone());
        let manifest = self.manifest(device)?;
        manifest
            .execute(
                "INSERT OR REPLACE INTO staging_entry
                     (file, path, size, mtime, durable_bytes, digest, name_sources)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.file.0 as i64,
                    entry.path.as_str(),
                    entry.size as i64,
                    entry.mtime.0,
                    entry.durable_bytes as i64,
                    entry.digest.map(|digest| digest.0.to_vec()),
                    render_sources(&entry.name_sources),
                ],
            )
            .map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    fn advance_watermark(
        &mut self,
        file: FileId,
        durable_bytes: u64,
    ) -> Result<StoreResponse, StoreError> {
        let device = self.owning(file)?;
        self.manifest(&device)?
            .execute(
                "UPDATE staging_entry SET durable_bytes = ?2 WHERE file = ?1",
                params![file.0 as i64, durable_bytes as i64],
            )
            .map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    fn mark_verified(
        &mut self,
        file: FileId,
        digest: Sha256,
        name_sources: &[CivilTime],
    ) -> Result<StoreResponse, StoreError> {
        let device = self.owning(file)?;
        let rendered = render_sources(name_sources);
        self.manifest(&device)?
            .execute(
                "UPDATE staging_entry SET digest = ?2, name_sources = ?3 WHERE file = ?1",
                params![file.0 as i64, digest.0.to_vec(), rendered],
            )
            .map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    fn drop_entry(&mut self, file: FileId) -> Result<StoreResponse, StoreError> {
        let device = self.owning(file)?;
        self.manifest(&device)?
            .execute(
                "DELETE FROM staging_entry WHERE file = ?1",
                params![file.0 as i64],
            )
            .map_err(|error| failed(&error))?;
        self.owner.remove(&file);
        Ok(StoreResponse::Done)
    }

    fn clear_manifest(&mut self, device: &DeviceId) -> Result<StoreResponse, StoreError> {
        let answer = self.execute(device, "DELETE FROM staging_entry", [])?;
        self.owner.retain(|_, owner| owner != device);
        Ok(answer)
    }

    // ---- the write-log -----------------------------------------------------------------

    /// One transaction writes the whole map, so a partly written plan cannot outlive a crash.
    fn seal_plan(
        &mut self,
        device: &DeviceId,
        plan: &[PlanEntry],
    ) -> Result<StoreResponse, StoreError> {
        let manifest = self.manifest(device)?;
        let transaction = manifest.transaction().map_err(|error| failed(&error))?;
        transaction
            .execute("DELETE FROM commit_plan", [])
            .map_err(|error| failed(&error))?;
        for (ordinal, entry) in plan.iter().enumerate() {
            let (action, name) = match &entry.action {
                PlanAction::Import { name } => ("import", name),
                PlanAction::Duplicate { name } => ("duplicate", name),
            };
            transaction
                .execute(
                    "INSERT INTO commit_plan (file, ordinal, action, vault_name)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![entry.file.0 as i64, ordinal as i64, action, name.as_str()],
                )
                .map_err(|error| failed(&error))?;
        }
        transaction.commit().map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    fn load_plan(&mut self, device: &DeviceId) -> Result<StoreResponse, StoreError> {
        let manifest = self.manifest(device)?;
        let mut statement = manifest
            .prepare("SELECT file, action, vault_name, done FROM commit_plan ORDER BY ordinal")
            .map_err(|error| failed(&error))?;
        let entries = statement
            .query_map([], read_plan_entry)
            .map_err(|error| failed(&error))?
            .collect::<Result<Vec<PlanEntry>, _>>()
            .map_err(|error| failed(&error))?;

        Ok(StoreResponse::CommitPlan(
            (!entries.is_empty()).then_some(entries),
        ))
    }

    /// Done-marks are committed after the directory syncs have returned, which is the whole
    /// reason `SPEC.md` §7.3 puts them in a separate step.
    fn mark_done(
        &mut self,
        device: &DeviceId,
        files: &[FileId],
    ) -> Result<StoreResponse, StoreError> {
        let manifest = self.manifest(device)?;
        let transaction = manifest.transaction().map_err(|error| failed(&error))?;
        for file in files {
            transaction
                .execute(
                    "UPDATE commit_plan SET done = 1 WHERE file = ?1",
                    params![file.0 as i64],
                )
                .map_err(|error| failed(&error))?;
        }
        transaction.commit().map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    fn execute<P: rusqlite::Params>(
        &mut self,
        device: &DeviceId,
        sql: &str,
        parameters: P,
    ) -> Result<StoreResponse, StoreError> {
        self.manifest(device)?
            .execute(sql, parameters)
            .map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    // ---- the index ---------------------------------------------------------------------

    fn lookup_content(&mut self, digests: &[Sha256]) -> Result<StoreResponse, StoreError> {
        let mut statement = self
            .index
            .prepare("SELECT vault_name FROM content WHERE digest = ?1")
            .map_err(|error| failed(&error))?;

        let mut rows = Vec::new();
        for digest in digests {
            let found: Option<String> = statement
                .query_row(params![digest.0.to_vec()], |row| row.get(0))
                .optional()
                .map_err(|error| failed(&error))?;
            if let Some(name) = found {
                rows.push(ContentRow {
                    digest: *digest,
                    vault_name: VaultName::new(name),
                });
            }
        }
        Ok(StoreResponse::Content(rows))
    }

    fn taken_names(&mut self, stems: &[String]) -> Result<StoreResponse, StoreError> {
        let mut statement = self
            .index
            .prepare("SELECT vault_name FROM content WHERE vault_name LIKE ?1 ESCAPE '\\'")
            .map_err(|error| failed(&error))?;

        let mut names = Vec::new();
        for stem in stems {
            let pattern = format!("{}%", escape_like(stem));
            let found = statement
                .query_map(params![pattern], |row| row.get::<_, String>(0))
                .map_err(|error| failed(&error))?
                .collect::<Result<Vec<String>, _>>()
                .map_err(|error| failed(&error))?;
            names.extend(found.into_iter().map(VaultName::new));
        }
        Ok(StoreResponse::VaultNames(names))
    }

    fn lookup_device_files(
        &mut self,
        device: &DeviceId,
        paths: &[DevicePath],
    ) -> Result<StoreResponse, StoreError> {
        let mut statement = self
            .index
            .prepare(
                "SELECT device_id, device_path, size, mtime, digest, vault_name, committed_at
                 FROM device_file WHERE device_id = ?1 AND device_path = ?2",
            )
            .map_err(|error| failed(&error))?;

        let mut rows = Vec::new();
        for path in paths {
            let found = statement
                .query_row(params![device.as_str(), path.as_str()], read_device_file)
                .optional()
                .map_err(|error| failed(&error))?;
            if let Some(row) = found {
                rows.push(row);
            }
        }
        Ok(StoreResponse::DeviceFiles(rows))
    }

    /// The batch's rows go in together, and are durable before the commit moves on.
    fn insert_batch(
        &mut self,
        contents: &[ContentRow],
        device_files: &[DeviceFileRow],
    ) -> Result<StoreResponse, StoreError> {
        let transaction = self.index.transaction().map_err(|error| failed(&error))?;
        for row in contents {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO content (digest, vault_name) VALUES (?1, ?2)",
                    params![row.digest.0.to_vec(), row.vault_name.as_str()],
                )
                .map_err(|error| failed(&error))?;
        }
        for row in device_files {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO device_file
                         (device_id, device_path, size, mtime, digest, vault_name, committed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        row.device.as_str(),
                        row.path.as_str(),
                        row.size as i64,
                        row.mtime.0,
                        row.digest.0.to_vec(),
                        row.vault_name.as_str(),
                        row.committed_at.0,
                    ],
                )
                .map_err(|error| failed(&error))?;
        }
        transaction.commit().map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }

    fn forget(
        &mut self,
        device: &DeviceId,
        path: &DevicePath,
    ) -> Result<StoreResponse, StoreError> {
        self.index
            .execute(
                "DELETE FROM device_file WHERE device_id = ?1 AND device_path = ?2",
                params![device.as_str(), path.as_str()],
            )
            .map_err(|error| failed(&error))?;
        Ok(StoreResponse::Done)
    }
}

fn connect(path: &Path) -> Result<Connection, StoreError> {
    let connection = Connection::open(path).map_err(|error| failed(&error))?;
    // WAL for concurrent readers, FULL because everything here is a durability claim.
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| failed(&error))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| failed(&error))?;
    Ok(connection)
}

fn read_staging_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<StagingEntry> {
    let digest: Option<Vec<u8>> = row.get(5)?;
    let sources: String = row.get(6)?;
    Ok(StagingEntry {
        file: FileId(row.get::<_, i64>(0)? as u64),
        path: DevicePath::new(row.get::<_, String>(1)?),
        size: row.get::<_, i64>(2)? as u64,
        mtime: Timestamp(row.get(3)?),
        durable_bytes: row.get::<_, i64>(4)? as u64,
        digest: digest.and_then(|bytes| read_digest(&bytes)),
        name_sources: read_sources(&sources),
    })
}

fn read_plan_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanEntry> {
    let action: String = row.get(1)?;
    let name = VaultName::new(row.get::<_, String>(2)?);
    Ok(PlanEntry {
        file: FileId(row.get::<_, i64>(0)? as u64),
        action: if action == "import" {
            PlanAction::Import { name }
        } else {
            PlanAction::Duplicate { name }
        },
        done: row.get::<_, i64>(3)? != 0,
    })
}

fn read_device_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceFileRow> {
    let digest: Vec<u8> = row.get(4)?;
    Ok(DeviceFileRow {
        device: DeviceId::new(row.get::<_, String>(0)?),
        path: DevicePath::new(row.get::<_, String>(1)?),
        size: row.get::<_, i64>(2)? as u64,
        mtime: Timestamp(row.get(3)?),
        digest: read_digest(&digest).unwrap_or(Sha256([0; 32])),
        vault_name: VaultName::new(row.get::<_, String>(5)?),
        committed_at: Timestamp(row.get(6)?),
    })
}

fn read_digest(bytes: &[u8]) -> Option<Sha256> {
    <[u8; 32]>::try_from(bytes).ok().map(Sha256)
}

/// Wall-clock readings as one field, in the order `SPEC.md` §7.2 tries them.
fn render_sources(sources: &[CivilTime]) -> String {
    sources
        .iter()
        .map(|time| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                time.year, time.month, time.day, time.hour, time.minute, time.second
            )
        })
        .collect::<Vec<String>>()
        .join(",")
}

fn read_sources(text: &str) -> Vec<CivilTime> {
    text.split(',').filter_map(parse_source).collect()
}

fn parse_source(text: &str) -> Option<CivilTime> {
    let (date, time) = text.trim().split_once(' ')?;
    let date: Vec<&str> = date.split('-').collect();
    let time: Vec<&str> = time.split(':').collect();
    if date.len() != 3 || time.len() != 3 {
        return None;
    }
    Some(CivilTime {
        year: date[0].parse().ok()?,
        month: date[1].parse().ok()?,
        day: date[2].parse().ok()?,
        hour: time[0].parse().ok()?,
        minute: time[1].parse().ok()?,
        second: time[2].parse().ok()?,
    })
}

/// A stem is a literal, and `_` is a wildcard in SQL patterns. A vault name has one.
fn escape_like(stem: &str) -> String {
    stem.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn failed(error: &dyn std::fmt::Display) -> StoreError {
    let message = error.to_string();
    if message.contains("database or disk is full") {
        return StoreError::NoSpace;
    }
    StoreError::Failed(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u8, day: u8) -> CivilTime {
        CivilTime {
            year,
            month,
            day,
            hour: 12,
            minute: 0,
            second: 0,
        }
    }

    #[test]
    fn wall_clock_readings_survive_the_round_trip() {
        let sources = vec![at(2026, 8, 1), at(2026, 9, 2)];

        assert_eq!(read_sources(&render_sources(&sources)), sources);
    }

    #[test]
    fn a_file_with_no_readings_renders_and_reads_as_none() {
        assert_eq!(render_sources(&[]), "");
        assert!(read_sources("").is_empty());
    }

    #[test]
    fn a_stems_underscore_is_a_literal_and_not_a_wildcard() {
        assert_eq!(escape_like("2026-08-01_123456"), r"2026-08-01\_123456");
    }
}
