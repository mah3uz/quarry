use std::path::Path;
use std::sync::{Arc, Once};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use super::{DbError, DbResult, ErrorKind};
use crate::conn::{ConnSpec, SslMode};

/// Installs the process-wide rustls crypto provider (aws-lc-rs, already linked by our deps).
pub fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn provider() -> Arc<CryptoProvider> {
    install_crypto_provider();
    CryptoProvider::get_default().cloned().unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
}

fn tls_err(msg: impl Into<String>) -> DbError {
    DbError::new(ErrorKind::Connection, msg)
}

/// prefer/require: encrypt without verifying; verify-ca: chain only; verify-full: chain + host name.
pub fn client_config(spec: &ConnSpec) -> DbResult<Option<ClientConfig>> {
    let Some(verifier) = verifier(spec)? else { return Ok(None) };
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| tls_err(format!("TLS setup failed: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let config = match (&spec.ssl_cert, &spec.ssl_key) {
        (Some(cert), Some(key)) => {
            let certs = load_certs(cert)?;
            let key = PrivateKeyDer::from_pem_file(key)
                .map_err(|e| tls_err(format!("cannot read client key {}: {e}", key.display())))?;
            builder.with_client_auth_cert(certs, key).map_err(|e| tls_err(format!("invalid client certificate: {e}")))?
        }
        _ => builder.with_no_client_auth(),
    };
    Ok(Some(config))
}

fn verifier(spec: &ConnSpec) -> DbResult<Option<Arc<dyn ServerCertVerifier>>> {
    let provider = provider();
    Ok(Some(match spec.ssl_mode {
        SslMode::Disable => return Ok(None),
        SslMode::Prefer | SslMode::Require => Arc::new(NoVerifier(provider)),
        SslMode::VerifyCa | SslMode::VerifyFull => {
            let roots = Arc::new(root_store(spec.ssl_ca.as_deref())?);
            let inner = WebPkiServerVerifier::builder_with_provider(roots, provider)
                .build()
                .map_err(|e| tls_err(format!("TLS setup failed: {e}")))?;
            if spec.ssl_mode == SslMode::VerifyCa { Arc::new(ChainOnlyVerifier(inner)) } else { inner }
        }
    }))
}

pub fn load_certs(path: &Path) -> DbResult<Vec<CertificateDer<'static>>> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(|it| it.collect::<Result<Vec<_>, _>>())
        .map_err(|e| tls_err(format!("cannot read certificates from {}: {e}", path.display())))?;
    if certs.is_empty() {
        return Err(tls_err(format!("no certificates found in {}", path.display())));
    }
    Ok(certs)
}

fn root_store(ca: Option<&Path>) -> DbResult<RootCertStore> {
    let mut store = RootCertStore::empty();
    match ca {
        Some(path) => {
            for cert in load_certs(path)? {
                store.add(cert).map_err(|e| tls_err(format!("invalid CA certificate in {}: {e}", path.display())))?;
            }
        }
        None => {
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let _ = store.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        }
    }
    Ok(store)
}

#[derive(Debug)]
struct NoVerifier(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[derive(Debug)]
struct ChainOnlyVerifier(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for ChainOnlyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.0.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tls");

    fn check(mode: SslMode, ca: &str, host: &str) -> Result<ServerCertVerified, rustls::Error> {
        let spec = ConnSpec {
            ssl_mode: mode,
            ssl_ca: Some(format!("{FIXTURES}/{ca}").into()),
            ..ConnSpec::new(crate::db::Backend::Postgres)
        };
        let leaf = load_certs(Path::new(&format!("{FIXTURES}/leaf.pem"))).unwrap().remove(0);
        let name = ServerName::try_from(host.to_string()).unwrap();
        verifier(&spec).unwrap().unwrap().verify_server_cert(&leaf, &[], &name, &[], UnixTime::now())
    }

    #[test]
    fn verify_ca_trusts_the_chain_but_ignores_the_host_name() {
        assert!(check(SslMode::VerifyCa, "ca.pem", "127.0.0.1").is_ok());
        assert!(check(SslMode::VerifyCa, "other.pem", "db.example").is_err(), "unknown issuer must fail");
    }

    #[test]
    fn verify_full_also_requires_the_host_name() {
        assert!(check(SslMode::VerifyFull, "ca.pem", "db.example").is_ok());
        assert!(check(SslMode::VerifyFull, "ca.pem", "127.0.0.1").is_err());
    }

    #[test]
    fn require_encrypts_without_verifying() {
        assert!(check(SslMode::Require, "other.pem", "anything.invalid").is_ok());
        let off = ConnSpec { ssl_mode: SslMode::Disable, ..ConnSpec::new(crate::db::Backend::Postgres) };
        assert!(client_config(&off).unwrap().is_none());
    }
}
