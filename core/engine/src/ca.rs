use crate::Error;
use hudsucker::rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use time::{Duration, OffsetDateTime};

const CERT_FILE: &str = "lantaran-ca.pem";
const KEY_FILE: &str = "lantaran-ca.key";
const VALID_DAYS: i64 = 3650;

/// The root certificate that devices install so the proxy can decrypt HTTPS.
///
/// It is created once and then reused, because every device that trusts it
/// would have to be set up again if it changed.
#[derive(Debug, Clone)]
pub struct RootCa {
    cert_pem: String,
    key_pem: String,
    cert_path: PathBuf,
}

impl RootCa {
    /// Loads the certificate and key stored in `dir`, creating both on first use.
    pub fn load_or_create(dir: &Path) -> Result<Self, Error> {
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);

        let (cert_pem, key_pem) = if cert_path.exists() && key_path.exists() {
            (read(&cert_path)?, read(&key_path)?)
        } else {
            let (cert_pem, key_pem) = generate()?;
            fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
            write_private(&key_path, &key_pem)?;
            fs::write(&cert_path, &cert_pem).map_err(|e| io_error(&cert_path, e))?;
            (cert_pem, key_pem)
        };

        let ca = Self {
            cert_pem,
            key_pem,
            cert_path,
        };
        // Fail now, with the file path still in hand, if either file is corrupt.
        ca.issuer()?;
        Ok(ca)
    }

    /// The certificate in PEM form. This is the part that is safe to share.
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// The certificate in DER form, as Android and Windows expect it.
    pub fn cert_der(&self) -> Vec<u8> {
        use hudsucker::rustls::pki_types::{CertificateDer, pem::PemObject};

        // The PEM was produced by this type and checked by `issuer()` when
        // it was loaded, so it parses.
        CertificateDer::from_pem_slice(self.cert_pem.as_bytes())
            .map(|der| der.to_vec())
            .unwrap_or_default()
    }

    /// Where the certificate is stored on disk.
    pub fn cert_path(&self) -> &Path {
        &self.cert_path
    }

    pub(crate) fn issuer(&self) -> Result<Issuer<'static, KeyPair>, Error> {
        let key = KeyPair::from_pem(&self.key_pem)?;
        Ok(Issuer::from_ca_cert_pem(&self.cert_pem, key)?)
    }
}

fn generate() -> Result<(String, String), Error> {
    let key = KeyPair::generate()?;

    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, "Lantaran Root CA");
    name.push(DnType::OrganizationName, "Lantaran");

    let mut params = CertificateParams::default();
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = OffsetDateTime::now_utc();
    // Backdated a day so a device whose clock runs behind still accepts it.
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(VALID_DAYS);

    let cert = params.self_signed(&key)?;
    Ok((cert.pem(), key.serialize_pem()))
}

fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| io_error(path, e))
}

/// Writes the private key readable by the current user only.
fn write_private(path: &Path, contents: &str) -> Result<(), Error> {
    use io::Write;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    options
        .open(path)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .map_err(|e| io_error(path, e))
}

fn io_error(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_certificate_on_first_use() {
        let dir = tempfile::tempdir().unwrap();
        let ca = RootCa::load_or_create(dir.path()).unwrap();

        assert!(ca.cert_pem().starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(ca.cert_path().exists());
        assert!(dir.path().join(KEY_FILE).exists());
    }

    #[test]
    fn der_form_is_the_same_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let ca = RootCa::load_or_create(dir.path()).unwrap();
        let der = ca.cert_der();

        // A DER certificate is an ASN.1 SEQUENCE.
        assert_eq!(der.first(), Some(&0x30));
        assert!(der.len() > 100);
    }

    #[test]
    fn reuses_the_stored_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let first = RootCa::load_or_create(dir.path()).unwrap();
        let second = RootCa::load_or_create(dir.path()).unwrap();

        assert_eq!(first.cert_pem(), second.cert_pem());
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");

        assert!(RootCa::load_or_create(&nested).is_ok());
    }

    #[test]
    fn rejects_a_corrupt_key() {
        let dir = tempfile::tempdir().unwrap();
        RootCa::load_or_create(dir.path()).unwrap();
        fs::write(dir.path().join(KEY_FILE), "not a key").unwrap();

        assert!(matches!(
            RootCa::load_or_create(dir.path()),
            Err(Error::Certificate(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn key_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        RootCa::load_or_create(dir.path()).unwrap();
        let mode = fs::metadata(dir.path().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();

        assert_eq!(mode & 0o777, 0o600);
    }
}
