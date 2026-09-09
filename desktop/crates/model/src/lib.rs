//! An independent account of what the desktop should be holding.
//!
//! This is the oracle. It watches what a phone did and works out what must be true afterwards
//! without knowing how the desktop does any of it: it has never heard of a staging file, a
//! watermark, a write-log or a rename. That is the whole value of it. A second implementation
//! that shared the first one's ideas would agree with it about the same mistakes.
//!
//! It is kept small on purpose. Its own correctness is reviewed by reading it, so there is
//! nothing here to optimise and nothing clever to follow.
//!
//! **What it does not predict.** Vault names. `SPEC.md` §7.2 calls naming cosmetic and says
//! correctness never depends on it, so predicting a name would mean carrying a copy of the
//! naming rules and would catch nothing that matters. What it predicts is which photographs
//! the vault holds, which content the index knows, and which row each file on each phone has.

use std::collections::{BTreeMap, BTreeSet};

use photo_sync_core::event::Event;
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp};
use photo_sync_core::{CatalogEntry, RunningDigest};

/// What the desktop should be holding, in the terms the specification cares about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expected {
    /// One entry per photograph in the vault. Deduplication means content appears once
    /// however many phones or paths hold it.
    pub vault: BTreeSet<Sha256>,

    /// Content the index knows about, which is the same set once a commit has finished.
    pub content: BTreeSet<Sha256>,

    /// What each file on each phone was recorded as.
    pub device_files: BTreeMap<(DeviceId, DevicePath), Recorded>,
}

/// The row a committed file leaves behind, minus the parts naming decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub digest: Sha256,
    pub size: u64,
    pub mtime: Timestamp,
}

/// A file that arrived whole and matched the digest the phone stated.
#[derive(Clone, Debug)]
struct Staged {
    path: DevicePath,
    digest: Sha256,
    size: u64,
    mtime: Timestamp,
}

/// A transfer in progress, which counts for nothing until it is whole.
#[derive(Clone, Debug)]
struct Arriving {
    device: DeviceId,
    path: DevicePath,
}

#[derive(Debug, Default)]
pub struct Model {
    /// What each phone said it holds, frozen at the start of its session.
    catalogs: BTreeMap<DeviceId, BTreeMap<DevicePath, CatalogEntry>>,

    /// Files arriving now, by the identifier the desktop gave them.
    arriving: BTreeMap<FileId, Arriving>,

    /// Every byte watched into each file so far, kept across a power loss.
    ///
    /// A resumed transfer sends only the part the desktop says it is missing, while the
    /// phone still states a digest over the whole photograph. Checking that digest therefore
    /// needs the earlier part too. The desktop reads it back off its own disk; this
    /// remembers watching it arrive, which is the same answer reached independently. The
    /// resume point is taken from the desktop, so a desktop that carried on from further
    /// along than it really held would hash bytes this never saw and the two would disagree,
    /// which is the disagreement worth catching.
    partials: BTreeMap<(DeviceId, DevicePath), Vec<u8>>,

    /// Files the desktop said it could not store.
    refused: BTreeSet<FileId>,

    /// Files that arrived whole and are waiting for a finish signal. These outlive a power
    /// loss, because a desktop that verified a file wrote that down before saying so.
    staged: BTreeMap<DeviceId, Vec<Staged>>,

    expected: Expected,
}

impl Model {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Watches one thing the phone did.
    ///
    /// Only the phone's side of `SPEC.md` §6 is listened to. Everything the desktop says to
    /// its own storage is deliberately ignored: an oracle that followed the implementation's
    /// working would agree with its mistakes.
    pub fn observe(&mut self, event: &Event) {
        match event {
            Event::CatalogSubmitted {
                device, entries, ..
            } => {
                self.catalogs.insert(
                    device.clone(),
                    entries
                        .iter()
                        .map(|entry| (entry.path.clone(), entry.clone()))
                        .collect(),
                );
            }
            Event::UploadOpened {
                device,
                file,
                path,
                offset,
            } => {
                self.arriving.insert(
                    *file,
                    Arriving {
                        device: device.clone(),
                        path: path.clone(),
                    },
                );
                // Whatever lies past the point the desktop asked to carry on from is about to
                // be sent again, so it is forgotten rather than counted twice.
                let seen = self
                    .partials
                    .entry((device.clone(), path.clone()))
                    .or_default();
                seen.truncate(usize::try_from(*offset).unwrap_or(usize::MAX));
            }
            Event::ChunkArrived { file, data, .. } => {
                if let Some(arriving) = self.arriving.get(file)
                    && let Some(seen) = self
                        .partials
                        .get_mut(&(arriving.device.clone(), arriving.path.clone()))
                {
                    seen.extend_from_slice(data);
                }
            }
            Event::UploadClosed { file, digest, .. } => self.close(*file, *digest),
            Event::UploadAborted { file, .. } => {
                self.arriving.remove(file);
            }
            Event::FinishRequested { device } | Event::ManualCommitRequested { device } => {
                self.commit(device);
            }
            // A restart forgets what was in flight and nothing else. A file the desktop had
            // verified was written down before it said so, and the part of a file that had
            // reached the disk is still there to be carried on from.
            Event::Started { .. } => {
                self.arriving.clear();
                self.catalogs.clear();
                self.refused.clear();
            }
            Event::PeerConnected { .. }
            | Event::PeerDisconnected { .. }
            | Event::DiffRequested { .. }
            | Event::DeletionsReported { .. }
            | Event::TimerFired { .. }
            | Event::StorageOpCompleted { .. }
            | Event::StoreOpCompleted { .. }
            | Event::FreeSpaceMeasured { .. }
            | Event::ClockRead { .. }
            | Event::ShutdownRequested => {}
        }
    }

    /// A file is only worth anything when it arrived whole and matched what the phone said.
    fn close(&mut self, file: FileId, claimed: Sha256) {
        let Some(arriving) = self.arriving.remove(&file) else {
            return;
        };
        let key = (arriving.device.clone(), arriving.path.clone());

        let mut digest = RunningDigest::new();
        digest.update(self.partials.get(&key).map_or(&[][..], Vec::as_slice));
        if self.refused.contains(&file) || digest.peek() != claimed {
            return;
        }
        let Some(entry) = self
            .catalogs
            .get(&arriving.device)
            .and_then(|catalog| catalog.get(&arriving.path))
        else {
            return;
        };

        let staged = self.staged.entry(arriving.device).or_default();
        staged.retain(|held| held.path != arriving.path);
        staged.push(Staged {
            path: arriving.path,
            digest: claimed,
            size: entry.size,
            mtime: entry.mtime,
        });
    }

    /// Everything a phone got across becomes part of the archive at once.
    ///
    /// What decides whether a photograph reaches the vault is whether the *index* already
    /// knows its content, not whether the vault still holds it. `SPEC.md` §7.3 deduplicates
    /// against the index and §7.5 says a photograph once imported is never imported again, so
    /// content whose vault copy a person deleted does not come back. It is simply never
    /// offered for deletion afterwards, which is what §8 checks the vault for.
    ///
    /// Every file gets a row whether or not its content was new, because the row is what
    /// later says the phone may delete it.
    fn commit(&mut self, device: &DeviceId) {
        for staged in self.staged.remove(device).unwrap_or_default() {
            // The file is whole and accounted for; nothing is left to carry on from.
            self.partials.remove(&(device.clone(), staged.path.clone()));
            if !self.expected.content.contains(&staged.digest) {
                self.expected.vault.insert(staged.digest);
            }
            self.expected.content.insert(staged.digest);
            self.expected.device_files.insert(
                (device.clone(), staged.path),
                Recorded {
                    digest: staged.digest,
                    size: staged.size,
                    mtime: staged.mtime,
                },
            );
        }
    }

    /// Records what an earlier session imported, so a run that starts part-way through has
    /// the same history the desktop was given.
    pub fn remember_import(
        &mut self,
        device: &DeviceId,
        path: &DevicePath,
        digest: Sha256,
        size: u64,
        mtime: Timestamp,
    ) {
        self.expected.content.insert(digest);
        self.expected.device_files.insert(
            (device.clone(), path.clone()),
            Recorded {
                digest,
                size,
                mtime,
            },
        );
    }

    /// Records that the vault still holds a photograph. A curated one is remembered by the
    /// index and not by the vault, which is the state `SPEC.md` §8 is written around.
    pub fn remember_vault_copy(&mut self, digest: Sha256) {
        self.expected.vault.insert(digest);
    }

    /// What the desktop answered about a file the phone sent.
    ///
    /// Only a refusal is listened to, and only ever to expect less. A desktop that could not
    /// store a file is the one thing an onlooker cannot work out for itself, since a full
    /// disk leaves no trace in what the phone said. A desktop claiming success is told
    /// nothing: this still counts a file only when the bytes it saw hash to what the phone
    /// stated, so no answer can talk it into expecting a photograph that never arrived.
    pub fn upload_refused(&mut self, file: FileId) {
        // A refusal can arrive before the phone has finished sending, so it is remembered
        // against the file rather than acted on once: what matters is that this file does not
        // end up counted, whenever the news comes.
        self.refused.insert(file);
        if let Some(arriving) = self.arriving.get(&file)
            && let Some(staged) = self.staged.get_mut(&arriving.device)
        {
            staged.retain(|held| held.path != arriving.path);
        }
    }

    /// Somebody deleted a photograph out of the vault. `SPEC.md` §8 keeps the index row: a
    /// photograph curated away was still imported, and is simply never offered for deletion
    /// again.
    pub fn forget_vault_copy(&mut self, digest: Sha256) {
        self.expected.vault.remove(&digest);
    }

    /// What the desktop should be holding now.
    #[must_use]
    pub fn expected(&self) -> &Expected {
        &self.expected
    }
}
