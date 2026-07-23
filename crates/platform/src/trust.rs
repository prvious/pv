use std::collections::BTreeSet;

use camino::Utf8Path;

use crate::LocalCaMetadata;
use crate::ca::is_pv_ca_metadata;
use crate::error::PlatformError;

#[cfg(not(target_os = "macos"))]
use crate::PlatformCapability;
#[cfg(not(target_os = "macos"))]
use crate::capability::unsupported;

#[cfg(target_os = "macos")]
use crate::ca::pem_from_der;
#[cfg(target_os = "macos")]
use crate::command::run_system_command;

#[cfg(target_os = "macos")]
use data_encoding::HEXUPPER;
#[cfg(target_os = "macos")]
use security_framework::trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate};
#[cfg(target_os = "macos")]
use sha1::{Digest, Sha1};

#[cfg(target_os = "macos")]
const SYSTEM_KEYCHAIN_PATH: &str = "/Library/Keychains/System.keychain";

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PrivilegeMode {
    Interactive,
    NonInteractive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeychainCertificate {
    pub metadata: LocalCaMetadata,
    pub trust: KeychainTrustResult,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum KeychainTrustResult {
    TrustRoot,
    TrustAsRoot,
    Deny,
    Unspecified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrustDomainState {
    Current {
        fingerprint: String,
    },
    NotTrusted {
        fingerprint: String,
    },
    Stale {
        expected_fingerprint: String,
        actual_fingerprint: String,
    },
    Denied {
        fingerprint: String,
    },
    Unknown {
        reason: String,
    },
    Unreadable {
        message: String,
    },
}

pub trait SystemTrustInspector {
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, PlatformError>;
}

#[derive(Debug, Default)]
pub struct NativeSystemTrustInspector;

pub fn inspect_system_ca_trust(
    local: Option<&LocalCaMetadata>,
    inspector: &impl SystemTrustInspector,
) -> TrustDomainState {
    let Some(local) = local else {
        return TrustDomainState::Unknown {
            reason: "local CA is unavailable".to_string(),
        };
    };
    let certificates = match inspector.trusted_certificates() {
        Ok(certificates) => certificates,
        Err(error) => {
            return TrustDomainState::Unreadable {
                message: error.to_string(),
            };
        }
    };
    let mut stale_fingerprint = None;
    let mut exact_trust = None;

    for certificate in certificates {
        let same_fingerprint = certificate.metadata.fingerprint == local.fingerprint;
        let pv_looking_ca = is_pv_ca_metadata(&certificate.metadata);

        if same_fingerprint {
            exact_trust = Some(certificate.trust);
            continue;
        }

        if pv_looking_ca
            && matches!(
                certificate.trust,
                KeychainTrustResult::TrustRoot | KeychainTrustResult::TrustAsRoot
            )
        {
            stale_fingerprint = Some(certificate.metadata.fingerprint);
        }
    }

    match exact_trust {
        Some(KeychainTrustResult::TrustRoot | KeychainTrustResult::TrustAsRoot) => {
            TrustDomainState::Current {
                fingerprint: local.fingerprint.clone(),
            }
        }
        Some(KeychainTrustResult::Deny) => TrustDomainState::Denied {
            fingerprint: local.fingerprint.clone(),
        },
        Some(KeychainTrustResult::Unspecified) | None => match stale_fingerprint {
            Some(actual_fingerprint) => TrustDomainState::Stale {
                expected_fingerprint: local.fingerprint.clone(),
                actual_fingerprint,
            },
            None => TrustDomainState::NotTrusted {
                fingerprint: local.fingerprint.clone(),
            },
        },
    }
}

pub fn trusted_pv_ca_fingerprints(
    inspector: &impl SystemTrustInspector,
) -> Result<Vec<String>, PlatformError> {
    let certificates = inspector.trusted_certificates()?;
    let fingerprints = certificates
        .into_iter()
        .filter(|certificate| {
            is_pv_ca_metadata(&certificate.metadata)
                && matches!(
                    certificate.trust,
                    KeychainTrustResult::TrustRoot
                        | KeychainTrustResult::TrustAsRoot
                        | KeychainTrustResult::Deny
                )
        })
        .map(|certificate| certificate.metadata.fingerprint)
        .collect::<BTreeSet<_>>();

    Ok(fingerprints.into_iter().collect())
}

pub fn trust_system_ca(
    certificate_path: &Utf8Path,
    privilege_mode: PrivilegeMode,
) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        trust_system_ca_with_runner(certificate_path, privilege_mode, &mut run_system_command)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = certificate_path;
        let _ = privilege_mode;
        Err(unsupported(PlatformCapability::TrustStore)?)
    }
}

#[cfg(target_os = "macos")]
fn trust_system_ca_with_runner(
    certificate_path: &Utf8Path,
    privilege_mode: PrivilegeMode,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let mut args = sudo_args(privilege_mode);
    args.extend([
        "/usr/bin/security",
        "add-trusted-cert",
        "-d",
        "-r",
        "trustRoot",
        "-p",
        "ssl",
        "-k",
        SYSTEM_KEYCHAIN_PATH,
        certificate_path.as_str(),
    ]);

    run_system("/usr/bin/sudo", &args)
}

pub fn untrust_system_ca(
    fingerprint: &str,
    privilege_mode: PrivilegeMode,
) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        let trust_settings = TrustSettings::new(Domain::Admin);
        let certificates = trust_settings
            .iter()
            .map_err(|error| PlatformError::Keychain(error.to_string()))?;

        for certificate in certificates {
            let certificate_pem = pem_from_der("CERTIFICATE", &certificate.to_der());
            let Ok(metadata) = LocalCaMetadata::from_certificate_pem(&certificate_pem) else {
                continue;
            };
            if metadata.fingerprint != fingerprint || !is_pv_ca_metadata(&metadata) {
                continue;
            }

            let sha1_fingerprint = certificate_sha1_fingerprint(&certificate.to_der());
            delete_system_ca_by_sha1_with_runner(
                &sha1_fingerprint,
                privilege_mode,
                &mut run_system_command,
            )?;
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = fingerprint;
        let _ = privilege_mode;
        Err(unsupported(PlatformCapability::TrustStore)?)
    }
}

#[cfg(target_os = "macos")]
fn delete_system_ca_by_sha1_with_runner(
    sha1_fingerprint: &str,
    privilege_mode: PrivilegeMode,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    let mut args = sudo_args(privilege_mode);
    args.extend([
        "/usr/bin/security",
        "delete-certificate",
        "-Z",
        sha1_fingerprint,
        SYSTEM_KEYCHAIN_PATH,
    ]);

    run_system("/usr/bin/sudo", &args)
}

#[cfg(target_os = "macos")]
fn sudo_args(privilege_mode: PrivilegeMode) -> Vec<&'static str> {
    match privilege_mode {
        PrivilegeMode::Interactive => Vec::new(),
        PrivilegeMode::NonInteractive => vec!["-n"],
    }
}

#[cfg(target_os = "macos")]
fn certificate_sha1_fingerprint(certificate_der: &[u8]) -> String {
    let digest = Sha1::digest(certificate_der);
    HEXUPPER.encode(&digest)
}

impl SystemTrustInspector for NativeSystemTrustInspector {
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, PlatformError> {
        #[cfg(target_os = "macos")]
        {
            let trust_settings = TrustSettings::new(Domain::Admin);
            let mut certificates = Vec::new();

            for certificate in trust_settings
                .iter()
                .map_err(|error| PlatformError::Keychain(error.to_string()))?
            {
                let trust = match trust_settings.tls_trust_settings_for_certificate(&certificate) {
                    Ok(Some(TrustSettingsForCertificate::TrustRoot)) => {
                        KeychainTrustResult::TrustRoot
                    }
                    Ok(Some(TrustSettingsForCertificate::TrustAsRoot)) => {
                        KeychainTrustResult::TrustAsRoot
                    }
                    Ok(Some(TrustSettingsForCertificate::Deny)) => KeychainTrustResult::Deny,
                    Ok(Some(TrustSettingsForCertificate::Unspecified)) | Ok(None) => {
                        KeychainTrustResult::Unspecified
                    }
                    Ok(Some(TrustSettingsForCertificate::Invalid)) => {
                        KeychainTrustResult::Unspecified
                    }
                    Err(error) => return Err(PlatformError::Keychain(error.to_string())),
                };
                let certificate_pem = pem_from_der("CERTIFICATE", &certificate.to_der());
                if let Ok(metadata) = LocalCaMetadata::from_certificate_pem(&certificate_pem) {
                    certificates.push(KeychainCertificate { metadata, trust });
                }
            }

            Ok(certificates)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(unsupported(PlatformCapability::TrustStore)?)
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use camino_tempfile::tempdir;

    use super::{
        PrivilegeMode, certificate_sha1_fingerprint, delete_system_ca_by_sha1_with_runner,
        trust_system_ca_with_runner,
    };

    #[test]
    fn trust_system_ca_uses_interactive_security_command() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let certificate_path = tempdir.path().join("ca.pem");
        let mut commands = Vec::new();

        trust_system_ca_with_runner(
            &certificate_path,
            PrivilegeMode::Interactive,
            &mut |program, args| {
                commands.push(format!("{program} {}", args.join(" ")));
                Ok(())
            },
        )?;

        assert_eq!(
            commands,
            [format!(
                "/usr/bin/sudo /usr/bin/security add-trusted-cert -d -r trustRoot -p ssl -k /Library/Keychains/System.keychain {certificate_path}"
            )]
        );

        Ok(())
    }

    #[test]
    fn trust_system_ca_uses_noninteractive_security_command() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let certificate_path = tempdir.path().join("ca.pem");
        let mut commands = Vec::new();

        trust_system_ca_with_runner(
            &certificate_path,
            PrivilegeMode::NonInteractive,
            &mut |program, args| {
                commands.push(format!("{program} {}", args.join(" ")));
                Ok(())
            },
        )?;

        assert_eq!(
            commands,
            [format!(
                "/usr/bin/sudo -n /usr/bin/security add-trusted-cert -d -r trustRoot -p ssl -k /Library/Keychains/System.keychain {certificate_path}"
            )]
        );

        Ok(())
    }

    #[test]
    fn delete_system_ca_uses_interactive_security_command() -> anyhow::Result<()> {
        let mut commands = Vec::new();

        delete_system_ca_by_sha1_with_runner(
            "ABC123",
            PrivilegeMode::Interactive,
            &mut |program, args| {
                commands.push(format!("{program} {}", args.join(" ")));
                Ok(())
            },
        )?;

        assert_eq!(
            commands,
            [
                "/usr/bin/sudo /usr/bin/security delete-certificate -Z ABC123 /Library/Keychains/System.keychain"
            ]
        );

        Ok(())
    }

    #[test]
    fn delete_system_ca_uses_noninteractive_security_command() -> anyhow::Result<()> {
        let mut commands = Vec::new();

        delete_system_ca_by_sha1_with_runner(
            "ABC123",
            PrivilegeMode::NonInteractive,
            &mut |program, args| {
                commands.push(format!("{program} {}", args.join(" ")));
                Ok(())
            },
        )?;

        assert_eq!(
            commands,
            [
                "/usr/bin/sudo -n /usr/bin/security delete-certificate -Z ABC123 /Library/Keychains/System.keychain"
            ]
        );

        Ok(())
    }

    #[test]
    fn certificate_sha1_fingerprint_renders_upper_hex() {
        assert_eq!(
            certificate_sha1_fingerprint(b"abc"),
            "A9993E364706816ABA3E25717850C26C9CD0D89D"
        );
    }
}
