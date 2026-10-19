use std::io::Cursor;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use rustix::process::{geteuid, getuid};
use state::fs;

use super::{KeychainCertificate, KeychainTrustResult, TrustDomainState};
use crate::ca::{certificate_der_from_pem, is_pv_ca_metadata, pem_from_der};
use crate::command::{
    command_output_with_timeout, run_system_command_output_with_timeout, system_command_output,
};
use crate::helper::linux as root_files;
use crate::helper::{owner_ca_certificate_path, validate_fingerprint};
use crate::{LocalCaMetadata, PlatformError};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const CERTUTIL: &str = "/usr/bin/certutil";
const STAGED_CA: &str = "/var/lib/pv/ca.pem";
const NICKNAME_PREFIX: &str = "PV Local Development CA ";
const ANCHOR_PREFIX: &str = "pv-local-ca-";

struct SystemStore {
    tool: &'static str,
    arguments: &'static [&'static str],
    directory: &'static str,
    extension: &'static str,
    bundle: &'static str,
}

impl SystemStore {
    fn detect() -> Result<Self, PlatformError> {
        for store in [
            Self {
                tool: "/usr/sbin/update-ca-certificates",
                arguments: &[],
                directory: "/usr/local/share/ca-certificates",
                extension: "crt",
                bundle: "/etc/ssl/certs/ca-certificates.crt",
            },
            Self {
                tool: "/usr/bin/update-ca-trust",
                arguments: &["extract"],
                directory: "/etc/pki/ca-trust/source/anchors",
                extension: "pem",
                bundle: "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
            },
        ] {
            if fs::path_is_file(Utf8Path::new(store.tool)).map_err(integration_error)? {
                return Ok(store);
            }
        }
        Err(integration_error(
            "install the distribution's ca-certificates package",
        ))
    }

    fn anchor(&self, fingerprint: &str) -> Utf8PathBuf {
        Utf8Path::new(self.directory).join(format!(
            "{ANCHOR_PREFIX}{}.{}",
            &fingerprint[..16],
            self.extension
        ))
    }

    fn update(&self) -> Result<(), PlatformError> {
        run_system_command_output_with_timeout(self.tool, self.arguments, COMMAND_TIMEOUT, None)?;
        Ok(())
    }

    fn certificates(&self) -> Result<Vec<KeychainCertificate>, PlatformError> {
        if !fs::path_is_directory(Utf8Path::new(self.directory)).map_err(integration_error)? {
            return Ok(Vec::new());
        }
        let mut anchors = Vec::new();
        for path in fs::read_dir_paths(Utf8Path::new(self.directory)).map_err(integration_error)? {
            if path
                .file_name()
                .is_some_and(|name| name.starts_with(ANCHOR_PREFIX))
                && path.extension() == Some(self.extension)
            {
                let pem = read_anchor(&path)?;
                let metadata = checked_metadata(&pem, None)?;
                if path != self.anchor(&metadata.fingerprint) {
                    return Err(integration_error(format!(
                        "PV CA anchor fingerprint mismatch: {path}"
                    )));
                }
                anchors.push((
                    metadata,
                    certificate_der_from_pem(&pem).map_err(integration_error)?,
                ));
            }
        }
        if anchors.is_empty() {
            return Ok(Vec::new());
        }
        let bundle = fs::read_to_string(Utf8Path::new(self.bundle)).map_err(integration_error)?;
        let certificates = rustls_pemfile::certs(&mut Cursor::new(bundle))
            .collect::<Result<Vec<_>, _>>()
            .map_err(integration_error)?;
        Ok(anchors
            .into_iter()
            .map(|(metadata, der)| KeychainCertificate {
                metadata,
                trust: if certificates
                    .iter()
                    .any(|certificate| certificate.as_ref() == der)
                {
                    KeychainTrustResult::TrustRoot
                } else {
                    KeychainTrustResult::Unspecified
                },
            })
            .collect())
    }
}

pub(crate) fn trusted_certificates() -> Result<Vec<KeychainCertificate>, PlatformError> {
    SystemStore::detect()?.certificates()
}

pub(crate) fn apply_system_ca_privileged(
    owner_uid: u32,
    fingerprint: &str,
) -> Result<(), PlatformError> {
    require_root()?;
    validate_fingerprint(fingerprint)?;
    let store = SystemStore::detect()?;
    let _lock = root_files::acquire_lock(Utf8Path::new(root_files::LOCK_PATH))?;
    let pem =
        fs::read_to_string(&owner_ca_certificate_path(owner_uid)?).map_err(integration_error)?;
    checked_metadata(&pem, Some(fingerprint))?;
    let staged = Utf8Path::new(STAGED_CA);
    if root_files::read_root_file(staged, 0o644)?.is_some() {
        fs::remove_file(staged).map_err(integration_error)?;
    }
    root_files::write_root_file(staged, pem.as_bytes(), 0o644)?;
    let result = (|| {
        let staged_pem = read_anchor(staged)?;
        checked_metadata(&staged_pem, Some(fingerprint))?;
        root_files::validate_directory_chain(Utf8Path::new(store.directory), true)?;
        let anchor = store.anchor(fingerprint);
        if root_files::read_root_file(&anchor, 0o644)?.is_some() {
            checked_metadata(&read_anchor(&anchor)?, Some(fingerprint))?;
        } else {
            root_files::write_root_file(&anchor, staged_pem.as_bytes(), 0o644)?;
        }
        store.update()
    })();
    let cleanup = fs::remove_file_if_exists(staged).map_err(integration_error);
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(integration_error(format!(
            "{error}; CA staging cleanup also failed: {cleanup}"
        ))),
    }
}

pub(crate) fn untrust_system_ca_privileged(fingerprint: &str) -> Result<(), PlatformError> {
    require_root()?;
    validate_fingerprint(fingerprint)?;
    let store = SystemStore::detect()?;
    let _lock = root_files::acquire_lock(Utf8Path::new(root_files::LOCK_PATH))?;
    let anchor = store.anchor(fingerprint);
    if root_files::read_root_file(&anchor, 0o644)?.is_some() {
        checked_metadata(&read_anchor(&anchor)?, Some(fingerprint))?;
        fs::remove_file(&anchor).map_err(integration_error)?;
    }
    store.update()
}

fn read_anchor(path: &Utf8Path) -> Result<String, PlatformError> {
    String::from_utf8(
        root_files::read_root_file(path, 0o644)?
            .ok_or_else(|| integration_error(format!("CA file is missing: {path}")))?,
    )
    .map_err(integration_error)
}

fn checked_metadata(
    pem: &str,
    fingerprint: Option<&str>,
) -> Result<LocalCaMetadata, PlatformError> {
    let metadata = LocalCaMetadata::from_certificate_pem(pem)?;
    if !is_pv_ca_metadata(&metadata)
        || fingerprint.is_some_and(|expected| metadata.fingerprint != expected)
    {
        return Err(PlatformError::PrivilegedHelperRejected {
            message: "certificate does not match the requested PV CA fingerprint".to_owned(),
        });
    }
    Ok(metadata)
}

fn require_root() -> Result<(), PlatformError> {
    if getuid().is_root() && geteuid().is_root() {
        Ok(())
    } else {
        Err(PlatformError::PrivilegedHelperAuthentication(
            "system CA operations require root".to_owned(),
        ))
    }
}

fn integration_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::SystemIntegration(error.to_string())
}

pub(super) fn browser_databases(home: &Utf8Path) -> Result<Vec<Utf8PathBuf>, PlatformError> {
    let mut databases = vec![home.join(".pki/nssdb")];
    for path in [
        home.join(".local/share/pki/nssdb"),
        home.join("snap/chromium/current/.pki/nssdb"),
    ] {
        if fs::path_is_directory(&path).map_err(integration_error)? {
            databases.push(path);
        }
    }
    for root in [
        home.join(".mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
    ] {
        if !fs::path_is_directory(&root).map_err(integration_error)? {
            continue;
        }
        for profile in fs::read_dir_paths(&root).map_err(integration_error)? {
            if fs::path_is_directory(&profile).map_err(integration_error)?
                && fs::path_is_file(&profile.join("cert9.db")).map_err(integration_error)?
            {
                databases.push(profile);
            }
        }
    }
    databases.sort();
    databases.dedup();
    Ok(databases)
}

fn nickname(fingerprint: &str) -> Result<String, PlatformError> {
    validate_fingerprint(fingerprint)?;
    Ok(format!("{NICKNAME_PREFIX}{}", &fingerprint[..16]))
}

fn certutil(database: &Utf8Path, arguments: &[&str]) -> Result<String, PlatformError> {
    let location = format!("sql:{database}");
    let mut all_arguments = vec!["-d", location.as_str()];
    all_arguments.extend_from_slice(arguments);
    run_system_command_output_with_timeout(CERTUTIL, &all_arguments, COMMAND_TIMEOUT, None)
}

fn certificate(
    database: &Utf8Path,
    nickname: &str,
) -> Result<Option<LocalCaMetadata>, PlatformError> {
    let location = format!("sql:{database}");
    let arguments = ["-d", location.as_str(), "-L", "-n", nickname, "-a"];
    let command = format!("{CERTUTIL} {}", arguments.join(" "));
    let output =
        command_output_with_timeout(CERTUTIL, &arguments, COMMAND_TIMEOUT).map_err(|source| {
            PlatformError::SystemIntegrationCommand {
                command: command.clone(),
                source,
            }
        })?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(255)
        && (stderr.contains("SEC_ERROR_UNKNOWN_CERT") || stderr.contains("PR_FILE_NOT_FOUND_ERROR"))
    {
        return Ok(None);
    }
    let pem = system_command_output(command, output)?;
    for der in rustls_pemfile::certs(&mut Cursor::new(pem)) {
        let der = der.map_err(integration_error)?;
        let metadata =
            LocalCaMetadata::from_certificate_pem(&pem_from_der("CERTIFICATE", der.as_ref()))?;
        if is_pv_ca_metadata(&metadata)
            && nickname == format!("{NICKNAME_PREFIX}{}", &metadata.fingerprint[..16])
        {
            return Ok(Some(metadata));
        }
    }
    Err(integration_error(
        "PV browser CA nickname does not match its certificate",
    ))
}

pub(super) fn inspect_browser_ca(
    database: &Utf8Path,
    metadata: Option<&LocalCaMetadata>,
) -> TrustDomainState {
    let Some(metadata) = metadata else {
        return TrustDomainState::Unknown {
            reason: "local CA is unavailable".to_owned(),
        };
    };
    let inspect = || -> Result<TrustDomainState, PlatformError> {
        if !fs::path_is_directory(database).map_err(integration_error)? {
            return Ok(TrustDomainState::NotTrusted {
                fingerprint: metadata.fingerprint.clone(),
            });
        }
        let name = nickname(&metadata.fingerprint)?;
        let Some(actual) = certificate(database, &name)? else {
            return Ok(TrustDomainState::NotTrusted {
                fingerprint: metadata.fingerprint.clone(),
            });
        };
        if actual.fingerprint != metadata.fingerprint {
            return Ok(TrustDomainState::Stale {
                expected_fingerprint: metadata.fingerprint.clone(),
                actual_fingerprint: actual.fingerprint,
            });
        }
        let listing = certutil(database, &["-L"])?;
        let trusted = listing
            .lines()
            .filter_map(|line| line.trim().rsplit_once(char::is_whitespace))
            .any(|(nickname, flags)| {
                nickname.trim_end() == name
                    && flags.split(',').next().is_some_and(|ssl| ssl.contains('C'))
            });
        Ok(if trusted {
            TrustDomainState::Current {
                fingerprint: metadata.fingerprint.clone(),
            }
        } else {
            TrustDomainState::NotTrusted {
                fingerprint: metadata.fingerprint.clone(),
            }
        })
    };
    inspect().unwrap_or_else(|error| TrustDomainState::Unreadable {
        message: error.to_string(),
    })
}

pub(super) fn trust_browser_ca(
    database: &Utf8Path,
    certificate_path: &Utf8Path,
) -> Result<(), PlatformError> {
    let pem = fs::read_to_string(certificate_path).map_err(integration_error)?;
    let metadata = checked_metadata(&pem, None)?;
    let name = nickname(&metadata.fingerprint)?;
    if !fs::path_is_directory(database).map_err(integration_error)? {
        fs::ensure_user_dir(database).map_err(integration_error)?;
        certutil(database, &["-N", "--empty-password"])?;
    }
    match certificate(database, &name)? {
        Some(existing) if existing.fingerprint != metadata.fingerprint => {
            return Err(integration_error(
                "PV browser CA nickname contains a different certificate",
            ));
        }
        Some(_) => {}
        None => {
            certutil(
                database,
                &[
                    "-A",
                    "-t",
                    "C,,",
                    "-n",
                    &name,
                    "-i",
                    certificate_path.as_str(),
                ],
            )?;
        }
    }
    certutil(database, &["-M", "-t", "C,,", "-n", &name])?;
    Ok(())
}

pub(super) fn untrust_browser_ca(
    database: &Utf8Path,
    fingerprint: Option<&str>,
) -> Result<(), PlatformError> {
    if !fs::path_is_directory(database).map_err(integration_error)? {
        return Ok(());
    }
    let names = if let Some(fingerprint) = fingerprint {
        vec![nickname(fingerprint)?]
    } else {
        certutil(database, &["-L"])?
            .lines()
            .filter_map(|line| line.trim().rsplit_once(char::is_whitespace))
            .map(|(name, _)| name.trim_end())
            .filter(|name| {
                name.strip_prefix(NICKNAME_PREFIX).is_some_and(|suffix| {
                    suffix.len() == 16 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
            .map(str::to_owned)
            .collect()
    };
    for name in names {
        if certificate(database, &name)?.is_some() {
            certutil(database, &["-D", "-n", &name])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rustix::process::getuid;

    use super::*;
    use crate::generate_local_ca;

    #[test]
    #[ignore = "requires root in a disposable Linux trust-store container"]
    fn linux_system_ca_apply_inspect_remove_matches_the_distribution_store() -> Result<()> {
        assert!(getuid().is_root());
        let certificate_path = owner_ca_certificate_path(1000)?;
        assert!(!fs::path_entry_exists(&certificate_path)?);
        root_files::validate_directory_chain(Utf8Path::new("/usr/local/libexec"), true)?;
        fs::ensure_user_dir(Utf8Path::new("/var/lib/pv"))?;
        let generated = generate_local_ca()?;
        fs::write_sensitive_file(&certificate_path, &generated.certificate_pem)?;
        let mut wrong = generated.metadata.fingerprint.clone();
        wrong.replace_range(..1, if wrong.starts_with('0') { "1" } else { "0" });
        assert!(matches!(
            apply_system_ca_privileged(1000, &wrong),
            Err(PlatformError::PrivilegedHelperRejected { .. })
        ));
        apply_system_ca_privileged(1000, &generated.metadata.fingerprint)?;
        apply_system_ca_privileged(1000, &generated.metadata.fingerprint)?;
        assert!(
            trusted_certificates()?
                .iter()
                .any(|certificate| certificate.metadata.fingerprint
                    == generated.metadata.fingerprint
                    && certificate.trust == KeychainTrustResult::TrustRoot)
        );
        untrust_system_ca_privileged(&generated.metadata.fingerprint)?;
        untrust_system_ca_privileged(&generated.metadata.fingerprint)?;
        assert!(trusted_certificates()?.is_empty());
        assert!(!fs::path_entry_exists(Utf8Path::new(STAGED_CA))?);
        assert!(fs::read_to_string(&certificate_path)? == generated.certificate_pem);
        Ok(())
    }
}
