use crate::Error;
use hudsucker::rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PublicKeyData,
};
use hudsucker::rustls::pki_types::{CertificateDer, pem::PemObject};
use p12_keystore::{
    Certificate, KeyStore, KeyStoreEntry, Pkcs12ImportPolicy, PrivateKey, PrivateKeyChain,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use time::{Duration, OffsetDateTime};

const CERT_FILE: &str = "lantaran-ca.pem";
const KEY_FILE: &str = "lantaran-ca.key";
const VALID_DAYS: i64 = 3650;
const NAME: &str = "Lantaran Root CA";
const PKCS12_ALIAS: &str = "lantaran root ca";

/// The root certificate that devices install so the proxy can decrypt HTTPS.
///
/// It is created once and then reused, because every device that trusts it
/// would have to be set up again if it changed. Only [`RootCa::reset`] and
/// [`RootCa::import_pkcs12`] replace it, and only when the user asks.
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

        if cert_path.exists() && key_path.exists() {
            Self::checked(read(&cert_path)?, read(&key_path)?, cert_path)
        } else {
            Self::reset(dir)
        }
    }

    /// Replaces the certificate and key in `dir` with new ones.
    ///
    /// Every device that trusted the old certificate stops accepting the
    /// proxy and has to be set up again. A running [`crate::Engine`] keeps
    /// signing with the old one until it is given the new one with
    /// [`crate::Engine::set_root_ca`] and restarted.
    pub fn reset(dir: &Path) -> Result<Self, Error> {
        let (cert_pem, key_pem) = generate(&default_name())?;
        Self::store(dir, cert_pem, key_pem)
    }

    /// Replaces the certificate and key in `dir` with the ones in a PKCS#12
    /// file, as written by [`RootCa::to_pkcs12`] on another machine.
    ///
    /// Devices that trusted the imported certificate work with this machine
    /// without being set up again. The file must hold a CA certificate and
    /// its private key; anything else is refused and `dir` is left as it was.
    pub fn import_pkcs12(dir: &Path, data: &[u8], password: &str) -> Result<Self, Error> {
        let store = KeyStore::from_pkcs12(data, password, Pkcs12ImportPolicy::Strict).map_err(
            |e| match e {
                p12_keystore::error::Error::MacError(_) => {
                    Error::Pkcs12("the password is wrong or the file is damaged".to_owned())
                }
                e => Error::Pkcs12(e.to_string()),
            },
        )?;
        let (_, chain) = store
            .private_key_chain()
            .ok_or(Error::NotARootCertificate("the file holds no private key"))?;
        let key = KeyPair::try_from(chain.key().as_der())?;
        let cert = chain
            .certs()
            .iter()
            .find(|cert| public_key_matches(cert.as_der(), &key))
            .ok_or(Error::NotARootCertificate(
                "the file holds no certificate for its private key",
            ))?;

        let cert_pem = pem::encode_config(
            &pem::Pem::new("CERTIFICATE", cert.as_der()),
            pem::EncodeConfig::new().set_line_ending(pem::LineEnding::LF),
        );
        // Check before touching the stored pair, so a bad file changes nothing.
        let ca = Self::checked(cert_pem, key.serialize_pem(), dir.join(CERT_FILE))?;
        Self::store(dir, ca.cert_pem, ca.key_pem)
    }

    /// The certificate and its private key as a PKCS#12 file protected by
    /// `password`, for moving this setup to another machine with
    /// [`RootCa::import_pkcs12`]. Anyone with the file and the password can
    /// decrypt the traffic of every device that trusts the certificate.
    pub fn to_pkcs12(&self, password: &str) -> Result<Vec<u8>, Error> {
        let key = KeyPair::from_pem(&self.key_pem)?;
        let pkcs12 = |e: p12_keystore::error::Error| Error::Pkcs12(e.to_string());
        let cert = Certificate::from_der(&self.cert_der()).map_err(pkcs12)?;
        let key = PrivateKey::from_der(&key.serialize_der()).map_err(pkcs12)?;

        let mut store = KeyStore::new();
        store.add_entry(
            PKCS12_ALIAS,
            KeyStoreEntry::PrivateKeyChain(PrivateKeyChain::new(
                PKCS12_ALIAS.as_bytes(),
                key,
                [cert],
            )),
        );
        store.writer(password).write().map_err(pkcs12)
    }

    /// The certificate in PEM form. This is the part that is safe to share.
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// The certificate in DER form, as Android and Windows expect it.
    pub fn cert_der(&self) -> Vec<u8> {
        // The PEM was checked when this value was made, so it parses.
        CertificateDer::from_pem_slice(self.cert_pem.as_bytes())
            .map(|der| der.to_vec())
            .unwrap_or_default()
    }

    /// The certificate's common name, which devices show in their lists of
    /// trusted certificates.
    pub fn name(&self) -> String {
        let der = self.cert_der();
        x509_parser::parse_x509_certificate(&der)
            .ok()
            .and_then(|(_, cert)| {
                cert.subject()
                    .iter_common_name()
                    .next()
                    .and_then(|name| name.as_str().ok().map(str::to_owned))
            })
            .unwrap_or_else(|| NAME.to_owned())
    }

    /// Where the certificate is stored on disk.
    pub fn cert_path(&self) -> &Path {
        &self.cert_path
    }

    pub(crate) fn issuer(&self) -> Result<Issuer<'static, KeyPair>, Error> {
        let key = KeyPair::from_pem(&self.key_pem)?;
        Ok(Issuer::from_ca_cert_pem(&self.cert_pem, key)?)
    }

    /// Builds a value from a pair that has not been checked yet, failing if
    /// either half is corrupt, the certificate is not a CA, or the key does
    /// not belong to it.
    fn checked(cert_pem: String, key_pem: String, cert_path: PathBuf) -> Result<Self, Error> {
        let key = KeyPair::from_pem(&key_pem)?;
        let der = CertificateDer::from_pem_slice(cert_pem.as_bytes())
            .map_err(|_| Error::NotARootCertificate("the certificate is not PEM"))?;
        let (_, cert) = x509_parser::parse_x509_certificate(&der)
            .map_err(|_| Error::NotARootCertificate("the certificate does not parse"))?;
        if !cert.is_ca() {
            return Err(Error::NotARootCertificate(
                "the certificate is not a certificate authority",
            ));
        }
        if !public_key_matches(&der, &key) {
            return Err(Error::NotARootCertificate(
                "the private key does not belong to the certificate",
            ));
        }

        let ca = Self {
            cert_pem,
            key_pem,
            cert_path,
        };
        ca.issuer()?;
        Ok(ca)
    }

    /// Writes a checked pair to `dir`, replacing what is there.
    fn store(dir: &Path, cert_pem: String, key_pem: String) -> Result<Self, Error> {
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);
        fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;

        // Each file is written beside its final name and moved into place, so
        // a crash leaves either file whole. `checked` refuses a mixed pair.
        let key_tmp = dir.join(format!("{KEY_FILE}.new"));
        let cert_tmp = dir.join(format!("{CERT_FILE}.new"));
        write_private(&key_tmp, &key_pem)?;
        fs::write(&cert_tmp, &cert_pem).map_err(|e| io_error(&cert_tmp, e))?;
        fs::rename(&key_tmp, &key_path).map_err(|e| io_error(&key_path, e))?;
        fs::rename(&cert_tmp, &cert_path).map_err(|e| io_error(&cert_path, e))?;

        Self::checked(cert_pem, key_pem, cert_path)
    }
}

/// Whether the certificate in `der` carries the public half of `key`.
fn public_key_matches(der: &[u8], key: &KeyPair) -> bool {
    x509_parser::parse_x509_certificate(der).is_ok_and(|(_, cert)| {
        cert.public_key().subject_public_key.data.as_ref() == key.der_bytes()
    })
}

/// A name that tells this machine's certificate apart from others on the
/// same device: `Lantaran Root CA (machine, 2026-11-27)`.
fn default_name() -> String {
    let date = OffsetDateTime::now_utc().date();
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let host = host.trim_end_matches(".local").trim();
    if host.is_empty() {
        format!("{NAME} ({date})")
    } else {
        format!("{NAME} ({host}, {date})")
    }
}

fn generate(name: &str) -> Result<(String, String), Error> {
    let key = KeyPair::generate()?;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, name);
    dn.push(DnType::OrganizationName, "Lantaran");

    let mut params = CertificateParams::default();
    params.distinguished_name = dn;
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

    #[test]
    fn names_the_certificate_after_the_machine_and_date() {
        let dir = tempfile::tempdir().unwrap();
        let ca = RootCa::load_or_create(dir.path()).unwrap();
        let name = ca.name();

        assert!(name.starts_with("Lantaran Root CA ("), "{name}");
        let today = OffsetDateTime::now_utc().date().to_string();
        assert!(name.ends_with(&format!("{today})")), "{name}");
    }

    #[test]
    fn reset_replaces_the_stored_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let old = RootCa::load_or_create(dir.path()).unwrap();
        let new = RootCa::reset(dir.path()).unwrap();
        let loaded = RootCa::load_or_create(dir.path()).unwrap();

        assert_ne!(old.cert_pem(), new.cert_pem());
        assert_eq!(loaded.cert_pem(), new.cert_pem());
        assert!(!dir.path().join(format!("{KEY_FILE}.new")).exists());
    }

    #[test]
    fn pkcs12_moves_the_certificate_and_key_to_another_machine() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let ca = RootCa::load_or_create(first.path()).unwrap();
        RootCa::load_or_create(second.path()).unwrap();

        let file = ca.to_pkcs12("correct horse").unwrap();
        let imported = RootCa::import_pkcs12(second.path(), &file, "correct horse").unwrap();
        let loaded = RootCa::load_or_create(second.path()).unwrap();

        assert_eq!(imported.cert_pem(), ca.cert_pem());
        assert_eq!(loaded.cert_pem(), ca.cert_pem());
        assert_eq!(read(&second.path().join(KEY_FILE)).unwrap(), ca.key_pem);
    }

    #[test]
    fn a_wrong_password_changes_nothing() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let file = RootCa::load_or_create(first.path())
            .unwrap()
            .to_pkcs12("right")
            .unwrap();
        let before = RootCa::load_or_create(second.path()).unwrap();

        assert!(matches!(
            RootCa::import_pkcs12(second.path(), &file, "wrong"),
            Err(Error::Pkcs12(_))
        ));
        let after = RootCa::load_or_create(second.path()).unwrap();
        assert_eq!(after.cert_pem(), before.cert_pem());
    }

    #[test]
    fn refuses_to_import_a_certificate_that_is_not_a_ca() {
        let dir = tempfile::tempdir().unwrap();
        let key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec!["example.com".to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let mut store = KeyStore::new();
        store.add_entry(
            "leaf",
            KeyStoreEntry::PrivateKeyChain(PrivateKeyChain::new(
                b"leaf".as_slice(),
                PrivateKey::from_der(&key.serialize_der()).unwrap(),
                [Certificate::from_der(leaf.der()).unwrap()],
            )),
        );
        let file = store.writer("pw").write().unwrap();

        assert!(matches!(
            RootCa::import_pkcs12(dir.path(), &file, "pw"),
            Err(Error::NotARootCertificate(_))
        ));
        assert!(!dir.path().join(CERT_FILE).exists());
    }

    #[test]
    fn refuses_a_key_that_belongs_to_another_certificate() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        RootCa::load_or_create(first.path()).unwrap();
        RootCa::load_or_create(second.path()).unwrap();
        fs::copy(first.path().join(KEY_FILE), second.path().join(KEY_FILE)).unwrap();

        assert!(matches!(
            RootCa::load_or_create(second.path()),
            Err(Error::NotARootCertificate(_))
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
