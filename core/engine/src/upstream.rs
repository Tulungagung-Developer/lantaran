//! How the proxy checks the servers it connects to on a device's behalf.
//!
//! To decrypt a host the proxy makes its own TLS connection to the real
//! server, so it has to judge that server's certificate itself. It trusts the
//! public web roots and every certificate authority installed on this
//! machine, which covers `mkcert` and company CAs. Hosts the user exempts are
//! accepted even when their certificate does not check out, which is what a
//! self-signed development server needs.

use crate::{Error, HostPattern};
use hudsucker::rustls::{
    self, CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    io,
    sync::{Arc, OnceLock, RwLock},
};

/// Hosts whose certificates are accepted without being checked.
pub(crate) type ExemptHosts = Arc<RwLock<Vec<HostPattern>>>;

pub(crate) fn exempt(hosts: &ExemptHosts, host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    hosts
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|pattern| pattern.matches(host))
}

/// The TLS settings for connections to servers, shared by HTTP and WebSocket.
pub(crate) fn client_config(
    provider: Arc<CryptoProvider>,
    exempt_hosts: ExemptHosts,
) -> Result<ClientConfig, Error> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Certificates that cannot be read are skipped: one broken entry in the
    // system store should not stop the proxy.
    roots.add_parsable_certificates(system_roots().iter().cloned());

    let checked =
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
            .build()
            .map_err(|e| Error::Tls(e.to_string()))?;

    // No ALPN here: hyper-rustls sets it for HTTP, and WebSocket upgrades
    // need HTTP/1.1, which is what no ALPN gives them.
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Verifier {
            checked,
            exempt_hosts,
        }))
        .with_no_client_auth();
    Ok(config)
}

/// The operating system's trusted certificates, read once per process.
/// Reading the macOS keychain takes long enough to matter when several
/// engines start together.
fn system_roots() -> &'static [CertificateDer<'static>] {
    static ROOTS: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| rustls_native_certs::load_native_certs().certs)
}

/// Checks certificates normally, and accepts any certificate for an exempt
/// host. Handshake signatures are always checked, so an exempt connection is
/// still encrypted to whoever holds the certificate's key.
#[derive(Debug)]
struct Verifier {
    checked: Arc<WebPkiServerVerifier>,
    exempt_hosts: ExemptHosts,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.checked.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => Ok(verified),
            Err(_) if exempt(&self.exempt_hosts, &server_name.to_str()) => {
                Ok(ServerCertVerified::assertion())
            }
            Err(error) => Err(error),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.checked.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.checked.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.checked.supported_verify_schemes()
    }
}

/// If a failed upstream request failed because the server's certificate was
/// rejected, says why in words a user can act on.
pub(crate) fn certificate_problem(err: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut pending = vec![err];
    while let Some(error) = pending.pop() {
        if let Some(rustls::Error::InvalidCertificate(problem)) =
            error.downcast_ref::<rustls::Error>()
        {
            return Some(explain(problem));
        }

        // The rustls error arrives inside nested io::Errors, which do not
        // expose what they wrap through `source`.
        if let Some(inner) = error
            .downcast_ref::<io::Error>()
            .and_then(|io| io.get_ref())
        {
            pending.push(inner);
        }
        if let Some(source) = error.source() {
            pending.push(source);
        }
    }
    None
}

fn explain(problem: &CertificateError) -> String {
    let what = match problem {
        CertificateError::UnknownIssuer => "it is not signed by an authority this machine trusts",
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => "it has expired",
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
            "it is not valid yet"
        }
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            "it is not issued for this host name"
        }
        CertificateError::Revoked => "it has been revoked",
        _ => "it is not valid",
    };
    format!(
        "the server's certificate was rejected: {what}. For a development server, \
         install its certificate authority on this machine or exempt the host from \
         certificate checks"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exemption_matches_patterns_and_ignores_brackets() {
        let hosts: ExemptHosts = Arc::new(RwLock::new(vec![
            HostPattern::new("*.dev.test"),
            HostPattern::new("::1"),
        ]));

        assert!(exempt(&hosts, "api.dev.test"));
        assert!(exempt(&hosts, "[::1]"));
        assert!(!exempt(&hosts, "dev.test"));
        assert!(!exempt(&hosts, "example.com"));
    }

    #[test]
    fn a_rejected_certificate_is_found_inside_an_io_error() {
        let rustls_error = rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer);
        let wrapped = io::Error::new(io::ErrorKind::InvalidData, rustls_error);

        let reason = certificate_problem(&wrapped).unwrap();
        assert!(reason.contains("not signed by an authority"), "{reason}");
    }

    #[test]
    fn a_rejected_certificate_is_found_inside_nested_io_errors() {
        // The shape hyper-util reports for a failed TLS connection.
        let rustls_error = rustls::Error::InvalidCertificate(CertificateError::Expired);
        let inner = io::Error::new(io::ErrorKind::InvalidData, rustls_error);
        let outer = io::Error::other(inner);

        let reason = certificate_problem(&outer).unwrap();
        assert!(reason.contains("expired"), "{reason}");
    }

    #[test]
    fn other_errors_are_not_called_certificate_problems() {
        let refused = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        assert!(certificate_problem(&refused).is_none());
    }
}
