//! A phone, playing its half of the protocol.
//!
//! It drives a whole session the way `SPEC.md` §6 describes: catalog, diff, one stream per
//! file, finish, then the deletion prompt. It also runs §8's own checks before deleting
//! anything, because a client that deleted whatever it was offered would not be evidence
//! that the offer was safe.

use std::collections::BTreeMap;

use photo_sync_core::effect::{
    CandidateOrigin, DeletionCandidate, Effect, RejectReason, SessionSummary, ToSend,
};
use photo_sync_core::event::{DeletionOutcome, DeletionResult, Event};
use photo_sync_core::id::{DeviceId, DevicePath, Sha256, Timestamp};
use photo_sync_core::{CatalogEntry, RunningDigest};

/// Something a phone can talk to: it takes an event and says what the desktop answered.
///
/// The simulator is one. The real desktop is another, which is what lets one session be run
/// against both and compared, as `docs/VERIFICATION.md` §L4 asks.
pub trait Driver {
    fn deliver(&mut self, event: Event);
    fn take_log(&mut self) -> Vec<Effect>;

    /// Takes the power away and brings the desktop back, saying whether it could.
    ///
    /// A simulated desktop can be stopped between any two effects, which is the only way to
    /// arrange a machine that lost power holding part of a file. A real one under test is
    /// shut down politely or not at all, so it answers no and whoever asked says where the
    /// case was checked instead of quietly checking something weaker.
    fn power_cycle(&mut self) -> bool {
        false
    }
}

/// How much of a file travels in one message. `STACK.md` §5.3 fixes it at 512 KB.
const CHUNK_BYTES: usize = 512 * 1024;

/// One file as the phone holds it.
#[derive(Clone, Debug)]
pub struct PhoneFile {
    pub mtime: Timestamp,
    pub content: Vec<u8>,
}

impl PhoneFile {
    #[must_use]
    pub fn new(mtime: i64, content: impl Into<Vec<u8>>) -> Self {
        Self {
            mtime: Timestamp(mtime),
            content: content.into(),
        }
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        self.content.len() as u64
    }

    #[must_use]
    pub fn digest(&self) -> Sha256 {
        let mut running = RunningDigest::new();
        running.update(&self.content);
        running.peek()
    }
}

/// What one session did, as the phone saw it.
#[derive(Clone, Debug, Default)]
pub struct SessionOutcome {
    pub summary: Option<SessionSummary>,

    /// Files this session streamed, whole or resumed.
    pub uploaded: Vec<DevicePath>,

    pub offered: Vec<DeletionCandidate>,
    pub deleted: Vec<DevicePath>,
    pub kept: Vec<DevicePath>,
    pub rejected: Option<RejectReason>,
}

#[derive(Clone, Debug)]
pub struct Phone {
    /// A photograph the camera writes over once the diff has been answered, which is the
    /// mid-session change `SPEC.md` §6.4 skips. Taken when it happens, so one arrangement
    /// covers one session.
    pub edit_after_diff: Option<(DevicePath, PhoneFile)>,

    /// The same, between the desktop offering a photograph for deletion and the phone
    /// deciding. This is what the two gates of `SPEC.md` §8 are for: what the desktop
    /// verified is not necessarily what is on the phone by the time it is asked to let go.
    pub edit_before_deleting: Option<(DevicePath, PhoneFile)>,

    pub device: DeviceId,
    pub name: String,
    pub files: BTreeMap<DevicePath, PhoneFile>,
}

impl Phone {
    #[must_use]
    pub fn new(device: &str, name: &str) -> Self {
        Self {
            device: DeviceId::new(device),
            name: name.to_string(),
            files: BTreeMap::new(),
            edit_after_diff: None,
            edit_before_deleting: None,
        }
    }

    #[must_use]
    pub fn holding(mut self, path: &str, file: PhoneFile) -> Self {
        self.files.insert(DevicePath::new(path), file);
        self
    }

    /// The catalog this session freezes. `SPEC.md` §3.2 enumerates once and never again.
    fn catalog(&self) -> Vec<CatalogEntry> {
        self.files
            .iter()
            .map(|(path, file)| CatalogEntry {
                path: path.clone(),
                size: file.size(),
                mtime: file.mtime,
            })
            .collect()
    }

    /// Gets everything across but stops short of the finish signal, so a caller can decide
    /// what happens to the commit.
    pub fn run_session_until_commit<D: Driver>(&mut self, sim: &mut D) -> SessionOutcome {
        self.transfer(sim, false)
    }

    /// Sends the first `bytes` of the first file the desktop asked for and then drops the
    /// connection, which is what the desktop sees when a phone goes out of range or a
    /// machine loses power part-way through a photograph. `SPEC.md` §7.6.
    pub fn send_partly<D: Driver>(&mut self, sim: &mut D, bytes: u64) -> SessionOutcome {
        let mut outcome = SessionOutcome::default();

        sim.deliver(Event::PeerConnected {
            device: self.device.clone(),
            name: self.name.clone(),
        });
        if let Some(reason) = refusal(&sim.take_log()) {
            outcome.rejected = Some(reason);
            return outcome;
        }

        let catalog = self.catalog();
        let total_bytes = catalog.iter().map(|entry| entry.size).sum();
        sim.deliver(Event::CatalogSubmitted {
            device: self.device.clone(),
            entries: catalog,
            total_bytes,
        });
        sim.deliver(Event::DiffRequested {
            device: self.device.clone(),
        });

        let answered = sim.take_log();
        if let Some(reason) = refusal(&answered) {
            outcome.rejected = Some(reason);
            return outcome;
        }

        if let Some(wanted) = to_send(&answered).first()
            && let Some(file) = self.files.get(&wanted.path)
        {
            let from = usize::try_from(wanted.resume_offset).unwrap_or(usize::MAX);
            let to = from
                .saturating_add(usize::try_from(bytes).unwrap_or(usize::MAX))
                .min(file.content.len());
            sim.deliver(Event::UploadOpened {
                device: self.device.clone(),
                file: wanted.file,
                path: wanted.path.clone(),
                size: file.size(),
                mtime: file.mtime,
                offset: wanted.resume_offset,
            });
            if from < to {
                sim.deliver(Event::ChunkArrived {
                    device: self.device.clone(),
                    file: wanted.file,
                    offset: wanted.resume_offset,
                    data: file.content[from..to].to_vec(),
                });
            }
            // The stream stops with no digest behind it, which is what a connection dying
            // mid-file looks like from the desktop's side.
            sim.deliver(Event::UploadAborted {
                device: self.device.clone(),
                file: wanted.file,
            });
            outcome.uploaded.push(wanted.path.clone());
        }

        sim.deliver(Event::PeerDisconnected {
            device: self.device.clone(),
        });
        sim.take_log();
        outcome
    }

    /// Runs one session from the handshake to the summary.
    pub fn run_session<D: Driver>(&mut self, sim: &mut D) -> SessionOutcome {
        self.transfer(sim, true)
    }

    fn transfer<D: Driver>(&mut self, sim: &mut D, finish: bool) -> SessionOutcome {
        let mut outcome = SessionOutcome::default();

        sim.deliver(Event::PeerConnected {
            device: self.device.clone(),
            name: self.name.clone(),
        });
        if let Some(reason) = refusal(&sim.take_log()) {
            outcome.rejected = Some(reason);
            return outcome;
        }

        let catalog = self.catalog();
        let total_bytes = catalog.iter().map(|entry| entry.size).sum();
        sim.deliver(Event::CatalogSubmitted {
            device: self.device.clone(),
            entries: catalog,
            total_bytes,
        });
        sim.deliver(Event::DiffRequested {
            device: self.device.clone(),
        });

        let answered = sim.take_log();
        if let Some(reason) = refusal(&answered) {
            outcome.rejected = Some(reason);
            return outcome;
        }
        // The camera writes over a photograph while the session is running. What the phone
        // sends from here is the new one, and its header says so.
        if let Some((path, replacement)) = self.edit_after_diff.take() {
            self.files.insert(path, replacement);
        }

        for wanted in to_send(&answered) {
            self.upload(sim, &wanted);
            outcome.uploaded.push(wanted.path.clone());
            sim.take_log();
        }

        if !finish {
            return outcome;
        }

        sim.deliver(Event::FinishRequested {
            device: self.device.clone(),
        });
        let finished = sim.take_log();
        outcome.offered = candidates(&finished);

        // The camera writes over a photograph between the offer and the answer.
        if let Some((path, replacement)) = self.edit_before_deleting.take() {
            self.files.insert(path, replacement);
        }

        let outcomes = self.decide(&outcome.offered);
        for reported in &outcomes {
            match reported.result {
                DeletionResult::Deleted => outcome.deleted.push(reported.path.clone()),
                _ => outcome.kept.push(reported.path.clone()),
            }
        }
        for path in &outcome.deleted {
            self.files.remove(path);
        }

        sim.deliver(Event::DeletionsReported {
            device: self.device.clone(),
            outcomes,
        });
        outcome.summary = summary(&sim.take_log());
        outcome
    }

    /// Sends one file, from the offset the desktop asked for. The digest covers the whole
    /// file, including a prefix the desktop already holds, so resume proves nothing about
    /// trust. `SPEC.md` §7.6.
    fn upload<D: Driver>(&self, sim: &mut D, wanted: &ToSend) {
        let Some(file) = self.files.get(&wanted.path) else {
            return;
        };
        // What the file is now, not what the catalog said it was. A photograph edited since
        // the session began says so here, and `SPEC.md` §6.4 skips it.
        sim.deliver(Event::UploadOpened {
            device: self.device.clone(),
            file: wanted.file,
            path: wanted.path.clone(),
            size: file.size(),
            mtime: file.mtime,
            offset: wanted.resume_offset,
        });

        let mut offset = wanted.resume_offset as usize;
        while offset < file.content.len() {
            let end = (offset + CHUNK_BYTES).min(file.content.len());
            sim.deliver(Event::ChunkArrived {
                device: self.device.clone(),
                file: wanted.file,
                offset: offset as u64,
                data: file.content[offset..end].to_vec(),
            });
            offset = end;
        }

        sim.deliver(Event::UploadClosed {
            device: self.device.clone(),
            file: wanted.file,
            digest: file.digest(),
        });
    }

    /// Runs the gates of `SPEC.md` §8 over what the desktop offered. A photo that no longer
    /// matches what the desktop verified is kept and reported, never deleted.
    fn decide(&self, offered: &[DeletionCandidate]) -> Vec<DeletionOutcome> {
        offered
            .iter()
            .map(|candidate| DeletionOutcome {
                path: candidate.path.clone(),
                result: self.check(candidate),
            })
            .collect()
    }

    fn check(&self, candidate: &DeletionCandidate) -> DeletionResult {
        let Some(file) = self.files.get(&candidate.path) else {
            return DeletionResult::Failed;
        };
        let unchanged = file.size() == candidate.size && file.mtime == candidate.mtime;

        let holds = match candidate.origin {
            // Its content was matched against a hash-verified staging entry this session, so
            // size and modification time carry the full guarantee.
            CandidateOrigin::ThisTransfer => unchanged,
            // Nothing recent vouches for it, so the phone reads the file and hashes it.
            CandidateOrigin::Earlier => unchanged && file.digest() == candidate.expected,
        };

        if holds {
            DeletionResult::Deleted
        } else {
            DeletionResult::KeptChanged
        }
    }
}

fn to_send(log: &[Effect]) -> Vec<ToSend> {
    log.iter()
        .find_map(|effect| match effect {
            Effect::SendDiff { to_send, .. } => Some(to_send.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn candidates(log: &[Effect]) -> Vec<DeletionCandidate> {
    log.iter()
        .find_map(|effect| match effect {
            Effect::SendCandidates { candidates, .. } => Some(candidates.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn summary(log: &[Effect]) -> Option<SessionSummary> {
    log.iter().find_map(|effect| match effect {
        Effect::SendSessionSummary { summary, .. } => Some(*summary),
        _ => None,
    })
}

fn refusal(log: &[Effect]) -> Option<RejectReason> {
    log.iter().find_map(|effect| match effect {
        Effect::RejectSession { reason, .. } => Some(reason.clone()),
        _ => None,
    })
}
