//! `lantaran cert`: export, import and reset the root certificate.

use clap::{Subcommand, ValueEnum};
use lantaran_engine::RootCa;
use std::{
    error::Error,
    fs,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

/// Read instead of prompting, so scripts can export and import.
const PASSWORD_VAR: &str = "LANTARAN_P12_PASSWORD";

#[derive(Subcommand)]
pub enum CertCommand {
    /// Print the certificate's name and where it is stored.
    Show,

    /// Write the certificate to FILE. As .p12 the private key goes with it,
    /// for moving this setup to another machine with `cert import`.
    Export {
        /// The file to write. Its extension picks the format unless --format
        /// is given: .pem, .crt/.cer/.der, or .p12/.pfx.
        file: PathBuf,

        #[arg(long, value_enum)]
        format: Option<Format>,
    },

    /// Replace the root certificate and key with the ones in a PKCS#12 file
    /// exported by `cert export` on another machine. Devices that trusted
    /// that machine then work with this one too.
    Import {
        file: PathBuf,

        /// Do not ask before replacing the current certificate.
        #[arg(long)]
        yes: bool,
    },

    /// Create a new root certificate and key. Every device that trusted the
    /// old certificate has to be set up again.
    Reset {
        /// Do not ask before replacing the current certificate.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// The certificate as PEM text.
    Pem,
    /// The certificate as binary DER, as Android and Windows expect it.
    Der,
    /// The certificate and its private key, protected by a password.
    P12,
}

impl Format {
    fn from_extension(file: &Path) -> Option<Self> {
        let extension = file.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "pem" => Some(Self::Pem),
            "crt" | "cer" | "der" => Some(Self::Der),
            "p12" | "pfx" => Some(Self::P12),
            _ => None,
        }
    }
}

pub fn run(command: CertCommand, data_dir: &Path) -> Result<(), Box<dyn Error>> {
    match command {
        CertCommand::Show => {
            let ca = RootCa::load_or_create(data_dir)?;
            println!("{}", ca.name());
            println!("{}", ca.cert_path().display());
        }
        CertCommand::Export { file, format } => {
            let format = format.or_else(|| Format::from_extension(&file)).ok_or(
                "cannot tell the format from the file name; pass --format pem, der or p12",
            )?;
            let ca = RootCa::load_or_create(data_dir)?;
            match format {
                Format::Pem => write(&file, ca.cert_pem().as_bytes(), false)?,
                Format::Der => write(&file, &ca.cert_der(), false)?,
                Format::P12 => {
                    let password = new_password()?;
                    write(&file, &ca.to_pkcs12(&password)?, true)?;
                    println!(
                        "The file holds the private key. Anyone with it and the password can read \
                         the traffic of every device that trusts this certificate."
                    );
                }
            }
            println!("Wrote {} to {}", ca.name(), file.display());
        }
        CertCommand::Import { file, yes } => {
            let data =
                fs::read(&file).map_err(|e| format!("could not read {}: {e}", file.display()))?;
            confirm(
                "This replaces the current root certificate and key. Devices that trusted the \
                 current certificate stop working with this machine unless they also trust the \
                 imported one. Export the current one first if you may want it back.",
                yes,
            )?;
            let password = password("Password for the file: ")?;
            let ca = RootCa::import_pkcs12(data_dir, &data, &password)?;
            println!("Imported {}", ca.name());
            println!("Restart any running Lantaran to use it.");
        }
        CertCommand::Reset { yes } => {
            confirm(
                "This creates a new root certificate and key. Every device that trusted the old \
                 certificate has to install the new one and trust it again.",
                yes,
            )?;
            let ca = RootCa::reset(data_dir)?;
            println!("Created {}", ca.name());
            println!("Restart any running Lantaran to use it.");
        }
    }
    Ok(())
}

/// Writes `contents` to `file`. With `private`, the file is readable only by
/// this user from the moment it exists: it is created new beside `file` and
/// moved over it, because an existing file would keep its permissions.
fn write(file: &Path, contents: &[u8], private: bool) -> Result<(), Box<dyn Error>> {
    let failed = |e: io::Error| format!("could not write {}: {e}", file.display());
    if !private {
        return fs::write(file, contents).map_err(|e| failed(e).into());
    }

    let mut name = file.file_name().ok_or("the file needs a name")?.to_owned();
    name.push(".new");
    let temporary = file.with_file_name(name);
    let _ = fs::remove_file(&temporary);

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&temporary)
        .and_then(|mut f| f.write_all(contents))
        .and_then(|()| fs::rename(&temporary, file));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(|e| failed(e).into())
}

/// Asks the user to type `yes`, unless `--yes` was given. Refuses when there
/// is nobody to ask.
fn confirm(warning: &str, yes: bool) -> Result<(), Box<dyn Error>> {
    println!("{warning}");
    if yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        return Err("pass --yes to confirm".into());
    }
    print!("Type yes to continue: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err("cancelled".into())
    }
}

fn password(prompt: &str) -> Result<String, Box<dyn Error>> {
    if let Ok(password) = std::env::var(PASSWORD_VAR) {
        return Ok(password);
    }
    if !io::stdin().is_terminal() {
        return Err(format!("set {PASSWORD_VAR} to give the password without a terminal").into());
    }
    Ok(rpassword::prompt_password(prompt)?)
}

/// A password for a new file, typed twice when prompted.
fn new_password() -> Result<String, Box<dyn Error>> {
    let first = password("Password to protect the file: ")?;
    if first.is_empty() {
        return Err("the file holds the private key, so it needs a password".into());
    }
    if std::env::var(PASSWORD_VAR).is_err() && password("Type it again: ")? != first {
        return Err("the passwords do not match".into());
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_private_file_replaces_an_open_one_without_widening_it() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("backup.p12");
        fs::write(&file, b"old").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

        write(&file, b"secret", true).unwrap();

        assert_eq!(fs::read(&file).unwrap(), b"secret");
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(!dir.path().join("backup.p12.new").exists());
    }

    #[test]
    fn the_extension_picks_the_format() {
        let format = |name: &str| Format::from_extension(Path::new(name));
        assert_eq!(format("ca.pem"), Some(Format::Pem));
        assert_eq!(format("ca.CRT"), Some(Format::Der));
        assert_eq!(format("ca.cer"), Some(Format::Der));
        assert_eq!(format("backup.p12"), Some(Format::P12));
        assert_eq!(format("backup.pfx"), Some(Format::P12));
        assert_eq!(format("ca.txt"), None);
        assert_eq!(format("ca"), None);
    }
}
