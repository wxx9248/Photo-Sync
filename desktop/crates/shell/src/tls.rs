//! The transport, and who is allowed onto it.
//!
//! `SPEC.md` §5.2 authenticates both ends by pinned key. There is no certificate authority
//! and no name to check, so the usual X.509 questions are not asked: these certificates are
//! self-signed and vouch for nothing beyond carrying a public key. What is verified is that
//! the peer holds the private key for a public key this desktop has agreed to talk to, which
//! `rustls` proves by checking the handshake signature.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};

use crate::identity::Identity;
use crate::pinning::{Admission, Paired, admit, public_key_of};

/// Whether the desktop is currently willing to meet a phone it has never met.
#[derive(Clone, Debug, Default)]
pub struct PairingWindow {
    open: Arc<Mutex<bool>>,
}

impl PairingWindow {
    #[must_use]
    pub fn closed() -> Self {
        Self::default()
    }

    pub fn open(&self) {
        self.set(true);
    }

    pub fn close(&self) {
        self.set(false);
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.lock().is_ok_and(|open| *open)
    }

    fn set(&self, value: bool) {
        if let Ok(mut open) = self.open.lock() {
            *open = value;
        }
    }
}

/// Admits a phone the desktop has pinned, or any phone while pairing is open.
#[derive(Debug)]
struct PinnedPhones {
    paired: Arc<Mutex<Paired>>,
    pairing: PairingWindow,
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for PinnedPhones {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        // Nothing is chained to, so there is no authority to hint at.
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let Ok(paired) = self.paired.lock() else {
            return Err(refused("the paired phones could not be read"));
        };
        match admit(&paired, self.pairing.is_open(), end_entity) {
            Admission::Known(_) | Admission::Offered { .. } => Ok(ClientCertVerified::assertion()),
            Admission::Refused => Err(refused("this phone is not paired with this desktop")),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The desktop's side of the connection.
pub fn server_config(
    identity: &Identity,
    paired: Arc<Mutex<Paired>>,
    pairing: PairingWindow,
) -> Result<ServerConfig, rustls::Error> {
    let provider = provider();
    let verifier = Arc::new(PinnedPhones {
        paired,
        pairing,
        provider: Arc::clone(&provider),
    });

    ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![certificate(identity)], private_key(identity))
}

/// Verifies a desktop by its pinned key. The phone half of `SPEC.md` §5.2, used by the test
/// client that stands in for it.
#[derive(Debug)]
struct PinnedDesktop {
    expected_key: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedDesktop {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match public_key_of(end_entity) {
            Some(key) if key == self.expected_key => Ok(ServerCertVerified::assertion()),
            // A changed desktop key is a re-pair, never a silent acceptance.
            _ => Err(refused("this is not the desktop this phone paired with")),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A phone's side of the connection: it presents its own key and accepts one desktop.
pub fn client_config(
    identity: &Identity,
    desktop_key: &[u8],
) -> Result<ClientConfig, rustls::Error> {
    let provider = provider();
    let verifier = Arc::new(PinnedDesktop {
        expected_key: desktop_key.to_vec(),
        provider: Arc::clone(&provider),
    });

    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![certificate(identity)], private_key(identity))
}

/// The cryptography rustls will use. `aws-lc-rs` is rustls's own default, and the process
/// may have installed it already, so an installed one is preferred over a second copy.
fn provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
}

fn certificate(identity: &Identity) -> CertificateDer<'static> {
    CertificateDer::from(identity.certificate().to_vec())
}

fn private_key(identity: &Identity) -> PrivateKeyDer<'static> {
    PrivateKeyDer::try_from(identity.private_key().to_vec())
        .unwrap_or_else(|_| unreachable!("the identity generated its own key in PKCS#8"))
}

fn refused(reason: &str) -> rustls::Error {
    rustls::Error::General(reason.to_string())
}
