//! A phone, speaking the real protocol over a real socket.
//!
//! The simulator's phone decides what to send and what it is safe to delete; this only
//! decides how those decisions travel. Both drive the same [`Phone`], so a session that
//! behaves differently over gRPC than in simulation is a difference in the transport rather
//! than in anyone's idea of the rules.
//!
//! [`Phone`]: photo_sync_sim::Phone

use std::net::SocketAddr;

use photo_sync::identity::Identity;
use photo_sync::tls::client_config;
use photo_sync_core::effect::{
    CandidateOrigin, CommitSummary, DeletionCandidate, DiffSummary, Effect, RejectReason,
    SessionSummary, ToSend, UploadOutcome,
};
use photo_sync_core::event::Event;
use photo_sync_core::id::{DeviceId, DevicePath, FileId, Sha256, Timestamp};
use photo_sync_protocol::v1 as wire;
use photo_sync_protocol::v1::photo_sync_client::PhotoSyncClient;
use photo_sync_sim::Driver;
use tokio::runtime::Handle;
use tonic::Status;
use tonic::transport::{Channel, Endpoint, Uri};

/// One file being sent, held until its digest arrives.
struct Sending {
    file: FileId,
    path: DevicePath,
    offset: u64,
    bytes: Vec<u8>,
}

/// A phone's end of a real connection.
pub struct Connected {
    runtime: Handle,
    client: PhotoSyncClient<Channel>,
    device: DeviceId,
    log: Vec<Effect>,
    sending: Option<Sending>,
}

impl Connected {
    /// Dials a desktop, pinning its key the way a paired phone would.
    ///
    /// # Errors
    /// When the address cannot be reached or the desktop presents a different key.
    pub fn dial(
        runtime: &Handle,
        address: SocketAddr,
        identity: &Identity,
        desktop_key: &[u8],
        device: &DeviceId,
    ) -> Result<Self, String> {
        let config = client_config(identity, desktop_key).map_err(|error| error.to_string())?;
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));

        let channel = runtime.block_on(async move {
            // The name is not checked; the pinned key is the whole of the identity, so any
            // syntactically valid authority does.
            Endpoint::from_static("http://phone-sync.invalid")
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let connector = connector.clone();
                    async move {
                        let socket = tokio::net::TcpStream::connect(address).await?;
                        let name = rustls_name();
                        let stream = connector.connect(name, socket).await?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                    }
                }))
                .await
        });

        Ok(Self {
            runtime: runtime.clone(),
            client: PhotoSyncClient::new(channel.map_err(|error| error.to_string())?),
            device: device.clone(),
            log: Vec::new(),
            sending: None,
        })
    }
}

fn rustls_name() -> tokio_rustls::rustls::pki_types::ServerName<'static> {
    tokio_rustls::rustls::pki_types::ServerName::try_from("photo-sync")
        .unwrap_or_else(|_| unreachable!("a constant name parses"))
}

impl Driver for Connected {
    fn deliver(&mut self, event: Event) {
        let answered = self.runtime.clone().block_on(self.exchange(event));
        self.log.extend(answered);
    }

    fn take_log(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.log)
    }
}

impl Connected {
    /// Turns one event into the call it stands for, and the answer back into effects.
    async fn exchange(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::PeerConnected { name, .. } => self.handshake(name).await,
            Event::CatalogSubmitted {
                entries,
                total_bytes,
                ..
            } => self.submit_catalog(entries, total_bytes).await,
            Event::DiffRequested { .. } => self.get_diff().await,
            Event::UploadOpened {
                file, path, offset, ..
            } => {
                self.sending = Some(Sending {
                    file,
                    path,
                    offset,
                    bytes: Vec::new(),
                });
                Vec::new()
            }
            Event::ChunkArrived { data, .. } => {
                if let Some(sending) = self.sending.as_mut() {
                    sending.bytes.extend_from_slice(&data);
                }
                Vec::new()
            }
            Event::UploadClosed { file, digest, .. } => self.upload(file, digest).await,
            Event::FinishRequested { .. } => self.finish().await,
            Event::DeletionsReported { outcomes, .. } => self.report(outcomes).await,
            // A phone never sends the rest of the vocabulary; those are the desktop talking
            // to itself.
            _ => Vec::new(),
        }
    }

    async fn handshake(&mut self, name: String) -> Vec<Effect> {
        let asked = wire::HandshakeRequest {
            protocol_version: photo_sync_protocol::PROTOCOL_VERSION,
            device_id: self.device.to_string(),
            device_name: name,
        };
        match self.client.handshake(asked).await {
            Ok(answer) => {
                if answer.into_inner().commit_in_progress {
                    return vec![Effect::RejectSession {
                        device: self.device.clone(),
                        reason: RejectReason::CommitInProgress,
                    }];
                }
                Vec::new()
            }
            Err(status) => vec![self.refused(&status)],
        }
    }

    async fn submit_catalog(
        &mut self,
        entries: Vec<photo_sync_core::CatalogEntry>,
        total_bytes: u64,
    ) -> Vec<Effect> {
        let chunk = wire::CatalogChunk {
            entries: entries
                .into_iter()
                .map(|entry| wire::CatalogEntry {
                    path: entry.path.to_string(),
                    size: entry.size,
                    mtime: entry.mtime.0,
                })
                .collect(),
            last: true,
            total_bytes,
        };
        match self
            .client
            .submit_catalog(tokio_stream::iter(vec![chunk]))
            .await
        {
            Ok(_) => Vec::new(),
            Err(status) => vec![self.refused(&status)],
        }
    }

    async fn get_diff(&mut self) -> Vec<Effect> {
        let mut chunks = match self.client.get_diff(wire::DiffRequest {}).await {
            Ok(answer) => answer.into_inner(),
            Err(status) => return vec![self.refused(&status)],
        };

        let mut to_send = Vec::new();
        let mut summary = DiffSummary::default();
        while let Ok(Some(chunk)) = chunks.message().await {
            to_send.extend(chunk.to_send.into_iter().map(|one| ToSend {
                file: FileId(one.file_id.parse().unwrap_or_default()),
                path: DevicePath::new(one.path),
                resume_offset: one.resume_offset,
            }));
            if let Some(reported) = chunk.summary {
                summary = DiffSummary {
                    to_send_count: reported.to_send_count,
                    to_send_bytes: reported.to_send_bytes,
                    already_imported: reported.already_imported,
                    already_staged: reported.already_staged,
                };
            }
        }

        vec![Effect::SendDiff {
            device: self.device.clone(),
            to_send,
            summary,
        }]
    }

    async fn upload(&mut self, file: FileId, digest: Sha256) -> Vec<Effect> {
        let Some(sending) = self.sending.take() else {
            return Vec::new();
        };

        let mut messages = vec![wire::FileChunk {
            kind: Some(wire::file_chunk::Kind::Header(wire::FileHeader {
                file_id: sending.file.0.to_string(),
                path: sending.path.to_string(),
                size: sending.offset + sending.bytes.len() as u64,
                mtime: 0,
                offset: sending.offset,
            })),
        }];
        for piece in sending.bytes.chunks(512 * 1024) {
            messages.push(wire::FileChunk {
                kind: Some(wire::file_chunk::Kind::Data(piece.to_vec())),
            });
        }
        messages.push(wire::FileChunk {
            kind: Some(wire::file_chunk::Kind::Trailer(wire::FileTrailer {
                sha256: digest.0.to_vec(),
            })),
        });

        let outcome = match self.client.upload_file(tokio_stream::iter(messages)).await {
            Ok(answer) => {
                let answer = answer.into_inner();
                match wire::upload_result::Status::try_from(answer.status) {
                    Ok(wire::upload_result::Status::Verified) => UploadOutcome::Verified {
                        durable_bytes: answer.durable_bytes,
                    },
                    Ok(wire::upload_result::Status::HashMismatch) => UploadOutcome::HashMismatch,
                    Ok(wire::upload_result::Status::ChangedOnPhone) => {
                        UploadOutcome::ChangedOnPhone
                    }
                    _ => UploadOutcome::WriteFailed,
                }
            }
            Err(_) => UploadOutcome::WriteFailed,
        };

        vec![Effect::SendUploadResult {
            device: self.device.clone(),
            file,
            outcome,
        }]
    }

    async fn finish(&mut self) -> Vec<Effect> {
        let mut chunks = match self.client.finish(wire::FinishRequest {}).await {
            Ok(answer) => answer.into_inner(),
            Err(status) => return vec![self.refused(&status)],
        };

        let mut candidates = Vec::new();
        let mut commit = CommitSummary::default();
        while let Ok(Some(chunk)) = chunks.message().await {
            candidates.extend(chunk.candidates.into_iter().map(|one| {
                DeletionCandidate {
                    path: DevicePath::new(one.path),
                    size: one.size,
                    mtime: Timestamp(one.mtime),
                    expected: <[u8; 32]>::try_from(one.expected_sha256.as_slice())
                        .map(Sha256)
                        .unwrap_or(Sha256([0; 32])),
                    origin: match wire::deletion_candidate::Origin::try_from(one.origin) {
                        Ok(wire::deletion_candidate::Origin::ThisTransfer) => {
                            CandidateOrigin::ThisTransfer
                        }
                        _ => CandidateOrigin::Earlier,
                    },
                }
            }));
            if let Some(reported) = chunk.commit {
                commit = CommitSummary {
                    imported: reported.imported,
                    duplicates: reported.duplicates,
                    bytes_committed: reported.bytes_committed,
                };
            }
        }

        vec![Effect::SendCandidates {
            device: self.device.clone(),
            candidates,
            commit,
        }]
    }

    async fn report(
        &mut self,
        outcomes: Vec<photo_sync_core::event::DeletionOutcome>,
    ) -> Vec<Effect> {
        let report = wire::DeletionReport {
            outcomes: outcomes
                .into_iter()
                .map(|outcome| wire::DeletionOutcome {
                    path: outcome.path.to_string(),
                    result: match outcome.result {
                        photo_sync_core::event::DeletionResult::Deleted => {
                            wire::deletion_outcome::Result::Deleted as i32
                        }
                        photo_sync_core::event::DeletionResult::KeptChanged => {
                            wire::deletion_outcome::Result::KeptChanged as i32
                        }
                        photo_sync_core::event::DeletionResult::KeptUser => {
                            wire::deletion_outcome::Result::KeptUser as i32
                        }
                        photo_sync_core::event::DeletionResult::Failed => {
                            wire::deletion_outcome::Result::Failed as i32
                        }
                    },
                })
                .collect(),
            last: true,
        };

        match self
            .client
            .report_deletions(tokio_stream::iter(vec![report]))
            .await
        {
            Ok(answer) => {
                let reported = answer.into_inner();
                vec![Effect::SendSessionSummary {
                    device: self.device.clone(),
                    summary: SessionSummary {
                        sent: reported.sent,
                        skipped: reported.skipped,
                        failed: reported.failed,
                        deleted: reported.deleted,
                        kept: reported.kept,
                        bytes_freed: reported.bytes_freed,
                    },
                }]
            }
            Err(status) => vec![self.refused(&status)],
        }
    }

    /// Turns a refusal back into the reason the core gave for it, so a session reads the same
    /// whichever way it was carried. `STACK.md` §5.5 fixes which code carries which.
    fn refused(&self, status: &Status) -> Effect {
        let reason = match status.code() {
            tonic::Code::Aborted => RejectReason::CommitInProgress,
            tonic::Code::FailedPrecondition => RejectReason::NotEnoughSpace {
                required_bytes: 0,
                available_bytes: 0,
            },
            _ => RejectReason::UnsupportedProtocolVersion {
                supported: photo_sync_protocol::PROTOCOL_VERSION,
            },
        };
        Effect::RejectSession {
            device: self.device.clone(),
            reason,
        }
    }
}
