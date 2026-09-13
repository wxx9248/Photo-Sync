//! Listening, and carrying the verified key onto each connection.
//!
//! A phone proves who it is once, during the TLS handshake. Everything after that has to be
//! able to ask which phone it is talking to without believing anything the phone wrote down,
//! so the key that authenticated the connection travels with it.

use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tonic::transport::server::Connected;

use crate::desk::Desk;
use crate::identity::Identity;
use crate::pinning::{Paired, public_key_of};
use crate::server::{PairingService, PeerKey, SyncService};
use crate::tls::{PairingWindow, server_config};
use photo_sync_protocol::v1::pairing_server::PairingServer;
use photo_sync_protocol::v1::photo_sync_server::PhotoSyncServer;

/// A connection, and the key that got it in.
pub struct PinnedStream {
    inner: TlsStream<TcpStream>,
    key: PeerKey,
}

impl Connected for PinnedStream {
    type ConnectInfo = PeerKey;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.key.clone()
    }
}

impl AsyncRead for PinnedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for PinnedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, bytes)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

/// A desktop listening for phones.
pub struct Listening {
    pub address: SocketAddr,
    accepting: tokio::task::JoinHandle<()>,
    serving: tokio::task::JoinHandle<()>,
}

impl Listening {
    /// Stops answering. Anything in flight is dropped, which is what a phone's reconnect path
    /// of `SPEC.md` §6 is written to survive.
    pub fn stop(self) {
        self.accepting.abort();
        self.serving.abort();
    }
}

/// Serves phones on `address`, which may be port zero to let the machine choose, recording
/// anything newly paired in `directory`.
///
/// The directory is asked for rather than defaulted. A desktop that wrote its pairings
/// somewhere other than where it reads them starts up not knowing any phone, and §5.2 makes
/// pairing a once-ever act, so that is a machine somebody has to pair again by hand.
///
/// # Errors
/// When the socket cannot be bound or the identity will not make a server configuration.
pub async fn listen(
    address: SocketAddr,
    identity: Arc<Identity>,
    paired: Arc<Mutex<Paired>>,
    pairing: PairingWindow,
    desk: Arc<AsyncMutex<Desk>>,
    name: &str,
    directory: &Path,
) -> std::io::Result<Listening> {
    let config = server_config(&identity, Arc::clone(&paired), pairing.clone())
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind(address).await?;
    let bound = listener.local_addr()?;

    let (connections, incoming) = tokio::sync::mpsc::channel::<std::io::Result<PinnedStream>>(8);
    let accepting = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                continue;
            };
            let acceptor = acceptor.clone();
            let connections = connections.clone();
            // Each handshake gets its own task, so one phone that connects and says nothing
            // cannot hold up the phone behind it.
            tokio::spawn(async move {
                let stream = match acceptor.accept(socket).await {
                    Ok(stream) => stream,
                    Err(error) => {
                        // A handshake that fails here is a phone that cannot get in and is
                        // told only that the connection went away. Rule 3.2: an error nobody
                        // is going to act on is still an error somebody has to read.
                        tracing::warn!("a phone could not complete the handshake: {error}");
                        return;
                    }
                };
                let key = PeerKey(peer_key(&stream));
                let _ = connections
                    .send(Ok(PinnedStream { inner: stream, key }))
                    .await;
            });
        }
    });

    let sync = SyncService::new(desk, Arc::clone(&paired), name);
    let meeting = PairingService::new(identity, paired, pairing, directory, name);
    let serving = tokio::spawn(async move {
        let incoming = tokio_stream::wrappers::ReceiverStream::new(incoming);
        let _ = tonic::transport::Server::builder()
            .add_service(PhotoSyncServer::new(sync))
            .add_service(PairingServer::new(meeting))
            .serve_with_incoming(incoming)
            .await;
    });

    Ok(Listening {
        address: bound,
        accepting,
        serving,
    })
}

/// The public key of whoever is on the other end, taken from the certificate the handshake
/// already verified.
fn peer_key(stream: &TlsStream<TcpStream>) -> Option<Vec<u8>> {
    let certificates = stream.get_ref().1.peer_certificates()?;
    public_key_of(certificates.first()?)
}
