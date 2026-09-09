//! The desktop, answering a phone over gRPC.
//!
//! Every handler does the same thing: work out which device is calling from the key that
//! authenticated the connection, turn the call into the events the core understands, and read
//! the core's answer back out of the effects it emitted. No decision is made here.
//!
//! The device is never taken from a message body. `STACK.md` §3.6 attaches the verified key
//! to the connection for exactly this reason: a phone can write any identifier it likes into
//! a request, and can write none at all with a key it does not hold.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use photo_sync_core::CatalogEntry;
use photo_sync_core::effect::{CandidateOrigin, Effect, UploadOutcome};
use photo_sync_core::event::{DeletionOutcome, DeletionResult, Event};
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp};
use photo_sync_protocol::v1 as wire;
use tokio::sync::Mutex as AsyncMutex;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};

use crate::desk::Desk;
use crate::pinning::Paired;

/// How many entries travel in one diff or candidate message. `STACK.md` §5.3 keeps these
/// bounded so neither end has to hold a whole library in memory.
const ENTRIES_PER_MESSAGE: usize = 1000;

/// The key that authenticated a connection, carried alongside it.
#[derive(Clone, Debug, Default)]
pub struct PeerKey(pub Option<Vec<u8>>);

/// One desktop, answering phones.
pub struct SyncService {
    desk: Arc<AsyncMutex<Desk>>,
    paired: Arc<Mutex<Paired>>,
    name: String,
}

impl SyncService {
    #[must_use]
    pub fn new(desk: Arc<AsyncMutex<Desk>>, paired: Arc<Mutex<Paired>>, name: &str) -> Self {
        Self {
            desk,
            paired,
            name: name.to_string(),
        }
    }

    /// Which phone is calling, according to the key it authenticated with.
    fn caller<T>(&self, request: &Request<T>) -> Result<DeviceId, Status> {
        let Some(PeerKey(Some(key))) = request.extensions().get::<PeerKey>() else {
            return Err(Status::unauthenticated("this connection presented no key"));
        };
        let Ok(paired) = self.paired.lock() else {
            return Err(Status::internal("the paired phones could not be read"));
        };
        paired
            .phone_for(key)
            .map(|phone| DeviceId::new(&phone.device))
            .ok_or_else(|| Status::unauthenticated("this phone is not paired with this desktop"))
    }

    /// Delivers events and hands back everything the core said about them.
    async fn deliver(&self, events: Vec<Event>) -> Vec<Effect> {
        let mut desk = self.desk.lock().await;
        // The core is a state machine and its adapters block, so this runs where blocking is
        // allowed rather than on an executor thread with other work waiting behind it.
        tokio::task::block_in_place(|| {
            desk.take_log();
            for event in events {
                desk.deliver(event);
            }
            desk.take_log()
        })
    }
}

type ChunkStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl wire::photo_sync_server::PhotoSync for SyncService {
    async fn handshake(
        &self,
        request: Request<wire::HandshakeRequest>,
    ) -> Result<Response<wire::HandshakeResponse>, Status> {
        let device = self.caller(&request)?;
        let asked = request.into_inner();
        if asked.protocol_version != photo_sync_protocol::PROTOCOL_VERSION {
            return Err(Status::failed_precondition(format!(
                "this desktop speaks version {}",
                photo_sync_protocol::PROTOCOL_VERSION
            )));
        }

        let effects = self
            .deliver(vec![Event::PeerConnected {
                device,
                name: asked.device_name,
            }])
            .await;

        Ok(Response::new(wire::HandshakeResponse {
            protocol_version: photo_sync_protocol::PROTOCOL_VERSION,
            desktop_name: self.name.clone(),
            commit_in_progress: refused_for_commit(&effects),
        }))
    }

    async fn submit_catalog(
        &self,
        request: Request<Streaming<wire::CatalogChunk>>,
    ) -> Result<Response<wire::CatalogAck>, Status> {
        let device = self.caller(&request)?;
        let mut chunks = request.into_inner();

        let mut entries = Vec::new();
        let mut total_bytes = 0;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            entries.extend(chunk.entries.into_iter().map(|entry| CatalogEntry {
                path: DevicePath::new(entry.path),
                size: entry.size,
                mtime: Timestamp(entry.mtime),
            }));
            if chunk.last {
                total_bytes = chunk.total_bytes;
            }
        }

        let received = entries.len() as u64;
        self.deliver(vec![Event::CatalogSubmitted {
            device,
            entries,
            total_bytes,
        }])
        .await;

        Ok(Response::new(wire::CatalogAck {
            entries_received: received,
        }))
    }

    type GetDiffStream = ChunkStream<wire::DiffChunk>;

    async fn get_diff(
        &self,
        request: Request<wire::DiffRequest>,
    ) -> Result<Response<Self::GetDiffStream>, Status> {
        let device = self.caller(&request)?;
        let effects = self.deliver(vec![Event::DiffRequested { device }]).await;

        if let Some(reason) = refusal(&effects) {
            return Err(reason);
        }
        let Some((to_send, summary)) = diff(&effects) else {
            return Err(Status::internal("the desktop produced no diff"));
        };

        let chunks = into_chunks(to_send, |entries, last| wire::DiffChunk {
            to_send: entries,
            last,
            summary: last.then_some(wire::DiffSummary {
                to_send_count: summary.to_send_count,
                to_send_bytes: summary.to_send_bytes,
                already_imported: summary.already_imported,
                already_staged: summary.already_staged,
            }),
        });
        Ok(Response::new(stream_of(chunks)))
    }

    async fn upload_file(
        &self,
        request: Request<Streaming<wire::FileChunk>>,
    ) -> Result<Response<wire::UploadResult>, Status> {
        let device = self.caller(&request)?;
        let mut chunks = request.into_inner();

        let mut file = None;
        let mut offset = 0;
        let mut answer = None;

        // Each message is handed over as it arrives rather than gathered up first. A file is
        // as large as the phone's storage, so holding one in memory to deliver it in one
        // piece would be a promise the desktop cannot keep; more than that, the durability of
        // `SPEC.md` §7.6 is built out of what has been written and synced *so far*, and
        // nothing has been written so far if the whole stream is still in hand. Taking the
        // desk lock per message rather than per file is also what lets §6.4's concurrent
        // streams make progress against one another.
        while let Some(chunk) = chunks.next().await {
            let event = match chunk?.kind {
                Some(wire::file_chunk::Kind::Header(header)) => {
                    let named = FileId(header.file_id.parse().map_err(|_| {
                        Status::invalid_argument("the file identifier is not a number")
                    })?);
                    file = Some(named);
                    offset = header.offset;
                    Event::UploadOpened {
                        device: device.clone(),
                        file: named,
                        path: DevicePath::new(header.path),
                        size: header.size,
                        mtime: Timestamp(header.mtime),
                        offset: header.offset,
                    }
                }
                Some(wire::file_chunk::Kind::Data(data)) => {
                    let Some(named) = file else {
                        return Err(Status::invalid_argument("bytes arrived before a header"));
                    };
                    let length = data.len() as u64;
                    let arrived = Event::ChunkArrived {
                        device: device.clone(),
                        file: named,
                        offset,
                        data: data.to_vec(),
                    };
                    offset += length;
                    arrived
                }
                Some(wire::file_chunk::Kind::Trailer(trailer)) => {
                    let Some(named) = file else {
                        return Err(Status::invalid_argument("a digest arrived before a header"));
                    };
                    Event::UploadClosed {
                        device: device.clone(),
                        file: named,
                        digest: digest_from(&trailer.sha256)?,
                    }
                }
                None => return Err(Status::invalid_argument("an empty message arrived")),
            };

            let effects = self.deliver(vec![event]).await;
            // The desktop has decided what became of the file. Anything still coming is bytes
            // it has already refused, so the answer goes back now rather than after them.
            if let Some(outcome) = upload_result(&effects) {
                answer = Some(outcome);
                break;
            }
        }

        let Some(outcome) = answer else {
            // The stream ended without a digest, which is a connection that went away
            // mid-file. Saying so is what makes the bytes that did arrive durable and fixes
            // the watermark the next diff resumes from, so it happens before the error goes
            // back. `SPEC.md` §7.6.
            if let Some(named) = file {
                self.deliver(vec![Event::UploadAborted {
                    device,
                    file: named,
                }])
                .await;
            }
            return Err(Status::aborted("the file was not finished"));
        };
        Ok(Response::new(outcome))
    }

    type FinishStream = ChunkStream<wire::FinishChunk>;

    async fn finish(
        &self,
        request: Request<wire::FinishRequest>,
    ) -> Result<Response<Self::FinishStream>, Status> {
        let device = self.caller(&request)?;
        let effects = self.deliver(vec![Event::FinishRequested { device }]).await;

        let Some((candidates, commit)) = candidates(&effects) else {
            return Err(Status::internal("the desktop produced no candidate list"));
        };
        let chunks = into_chunks(candidates, |entries, last| wire::FinishChunk {
            candidates: entries,
            last,
            commit: last.then_some(wire::CommitSummary {
                imported: commit.imported,
                duplicates: commit.duplicates,
                bytes_committed: commit.bytes_committed,
            }),
        });
        Ok(Response::new(stream_of(chunks)))
    }

    async fn report_deletions(
        &self,
        request: Request<Streaming<wire::DeletionReport>>,
    ) -> Result<Response<wire::SessionSummary>, Status> {
        let device = self.caller(&request)?;
        let mut reports = request.into_inner();

        let mut outcomes = Vec::new();
        while let Some(report) = reports.next().await {
            for outcome in report?.outcomes {
                outcomes.push(DeletionOutcome {
                    path: DevicePath::new(outcome.path),
                    result: deletion_result(outcome.result),
                });
            }
        }

        let effects = self
            .deliver(vec![Event::DeletionsReported { device, outcomes }])
            .await;
        let Some(summary) = session_summary(&effects) else {
            return Err(Status::internal("the desktop did not sum the session up"));
        };
        Ok(Response::new(summary))
    }
}

// ---- reading the core's answers ----------------------------------------------------------

fn refusal(effects: &[Effect]) -> Option<Status> {
    effects.iter().find_map(|effect| match effect {
        Effect::RejectSession { reason, .. } => Some(match reason {
            photo_sync_core::RejectReason::NotEnoughSpace {
                required_bytes,
                available_bytes,
            } => Status::failed_precondition(format!(
                "this transfer needs {required_bytes} bytes and there are {available_bytes}"
            )),
            photo_sync_core::RejectReason::CommitInProgress => {
                Status::aborted("finishing the previous import")
            }
            photo_sync_core::RejectReason::UnsupportedProtocolVersion { supported } => {
                Status::failed_precondition(format!("this desktop speaks version {supported}"))
            }
        }),
        _ => None,
    })
}

fn refused_for_commit(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::RejectSession {
                reason: photo_sync_core::RejectReason::CommitInProgress,
                ..
            }
        )
    })
}

fn diff(effects: &[Effect]) -> Option<(Vec<wire::ToSend>, photo_sync_core::effect::DiffSummary)> {
    effects.iter().find_map(|effect| match effect {
        Effect::SendDiff {
            to_send, summary, ..
        } => Some((
            to_send
                .iter()
                .map(|one| wire::ToSend {
                    path: one.path.to_string(),
                    file_id: one.file.0.to_string(),
                    resume_offset: one.resume_offset,
                })
                .collect(),
            *summary,
        )),
        _ => None,
    })
}

fn upload_result(effects: &[Effect]) -> Option<wire::UploadResult> {
    effects.iter().find_map(|effect| match effect {
        Effect::SendUploadResult { outcome, .. } => Some(match outcome {
            UploadOutcome::Verified { durable_bytes } => wire::UploadResult {
                status: wire::upload_result::Status::Verified as i32,
                durable_bytes: *durable_bytes,
            },
            UploadOutcome::HashMismatch => wire::UploadResult {
                status: wire::upload_result::Status::HashMismatch as i32,
                durable_bytes: 0,
            },
            UploadOutcome::ChangedOnPhone => wire::UploadResult {
                status: wire::upload_result::Status::ChangedOnPhone as i32,
                durable_bytes: 0,
            },
            UploadOutcome::WriteFailed => wire::UploadResult {
                status: wire::upload_result::Status::WriteFailed as i32,
                durable_bytes: 0,
            },
        }),
        _ => None,
    })
}

fn candidates(
    effects: &[Effect],
) -> Option<(
    Vec<wire::DeletionCandidate>,
    photo_sync_core::effect::CommitSummary,
)> {
    effects.iter().find_map(|effect| match effect {
        Effect::SendCandidates {
            candidates, commit, ..
        } => Some((
            candidates
                .iter()
                .map(|one| wire::DeletionCandidate {
                    path: one.path.to_string(),
                    size: one.size,
                    mtime: one.mtime.0,
                    expected_sha256: one.expected.0.to_vec(),
                    origin: match one.origin {
                        CandidateOrigin::ThisTransfer => {
                            wire::deletion_candidate::Origin::ThisTransfer as i32
                        }
                        CandidateOrigin::Earlier => {
                            wire::deletion_candidate::Origin::Earlier as i32
                        }
                    },
                })
                .collect(),
            *commit,
        )),
        _ => None,
    })
}

fn session_summary(effects: &[Effect]) -> Option<wire::SessionSummary> {
    effects.iter().find_map(|effect| match effect {
        Effect::SendSessionSummary { summary, .. } => Some(wire::SessionSummary {
            sent: summary.sent,
            skipped: summary.skipped,
            failed: summary.failed,
            deleted: summary.deleted,
            kept: summary.kept,
            bytes_freed: summary.bytes_freed,
        }),
        _ => None,
    })
}

fn deletion_result(reported: i32) -> DeletionResult {
    match wire::deletion_outcome::Result::try_from(reported) {
        Ok(wire::deletion_outcome::Result::Deleted) => DeletionResult::Deleted,
        Ok(wire::deletion_outcome::Result::KeptChanged) => DeletionResult::KeptChanged,
        Ok(wire::deletion_outcome::Result::KeptUser) => DeletionResult::KeptUser,
        // An outcome this desktop does not recognise is not a deletion, and treating it as a
        // failure is the reading that can only under-delete.
        _ => DeletionResult::Failed,
    }
}

fn digest_from(bytes: &[u8]) -> Result<Sha256, Status> {
    <[u8; 32]>::try_from(bytes)
        .map(Sha256)
        .map_err(|_| Status::invalid_argument("a digest was not thirty-two bytes"))
}

/// Splits a list into messages, marking the last. An empty list still sends one message, so
/// the summary it carries always arrives.
fn into_chunks<T, C>(entries: Vec<T>, build: impl Fn(Vec<T>, bool) -> C) -> Vec<C> {
    if entries.is_empty() {
        return vec![build(Vec::new(), true)];
    }

    let total = entries.len();
    let mut chunks = Vec::new();
    let mut taken = 0;
    let mut remaining = entries;
    while !remaining.is_empty() {
        let rest = remaining.split_off(ENTRIES_PER_MESSAGE.min(remaining.len()));
        taken += remaining.len();
        chunks.push(build(remaining, taken == total));
        remaining = rest;
    }
    chunks
}

fn stream_of<T: Send + 'static>(chunks: Vec<T>) -> ChunkStream<T> {
    Box::pin(tokio_stream::iter(chunks.into_iter().map(Ok)))
}

/// The one-time exchange of `SPEC.md` §5.2.
///
/// A phone the desktop has never met can only get this far while a person has opened pairing.
/// The desktop puts a code on its screen, the phone shows the same code, and a person who can
/// see both says whether they match. Nothing about the code travels over the connection it is
/// protecting: both ends compute it from the two keys they already hold.
pub struct PairingService {
    identity: Arc<crate::identity::Identity>,
    paired: Arc<Mutex<Paired>>,
    window: crate::tls::PairingWindow,
    directory: std::path::PathBuf,
    name: String,
}

impl PairingService {
    #[must_use]
    pub fn new(
        identity: Arc<crate::identity::Identity>,
        paired: Arc<Mutex<Paired>>,
        window: crate::tls::PairingWindow,
        directory: &std::path::Path,
        name: &str,
    ) -> Self {
        Self {
            identity,
            paired,
            window,
            directory: directory.to_path_buf(),
            name: name.to_string(),
        }
    }
}

#[tonic::async_trait]
impl wire::pairing_server::Pairing for PairingService {
    async fn pair(
        &self,
        request: Request<wire::PairRequest>,
    ) -> Result<Response<wire::PairResponse>, Status> {
        if !self.window.is_open() {
            return Err(Status::failed_precondition(
                "this desktop is not open for pairing",
            ));
        }
        let Some(PeerKey(Some(key))) = request.extensions().get::<PeerKey>().cloned() else {
            return Err(Status::unauthenticated("this connection presented no key"));
        };

        let asked = request.into_inner();
        if asked.protocol_version != photo_sync_protocol::PROTOCOL_VERSION {
            return Err(Status::failed_precondition(format!(
                "this desktop speaks version {}",
                photo_sync_protocol::PROTOCOL_VERSION
            )));
        }

        // The code binds both keys, so a man in the middle holding a different key with each
        // side cannot make the two screens agree.
        let code = crate::identity::pairing_code(self.identity.public_key(), &key);
        self.window.show(&code);

        let accepted = self.window.decision().await.unwrap_or(false);
        if accepted {
            let Ok(mut paired) = self.paired.lock() else {
                return Err(Status::internal("the paired phones could not be written"));
            };
            paired.pair(&key, &DeviceId::new(&asked.device_id), &asked.device_name);
            paired
                .save(&self.directory)
                .map_err(|error| Status::internal(error.to_string()))?;
        }
        self.window.close();

        Ok(Response::new(wire::PairResponse {
            accepted,
            desktop_name: self.name.clone(),
        }))
    }
}
