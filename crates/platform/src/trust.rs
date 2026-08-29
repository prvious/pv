use std::collections::BTreeSet;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeychainCertificate {
    pub metadata: LocalCaMetadata,
    pub trust: KeychainTrustResult,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum KeychainTrustResult {
    TrustRoot,
    TrustAsRoot,
    Deny,
    Unspecified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

pub fn trust_system_ca(certificate_path: &Utf8Path) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        let certificate_pem = state::fs::read_to_string(certificate_path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        let metadata = LocalCaMetadata::from_certificate_pem(&certificate_pem)?;
        crate::PrivilegedHelperClient.apply_ca(&metadata.fingerprint)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = certificate_path;
        Err(unsupported(PlatformCapability::TrustStore)?)
    }
}

pub fn untrust_system_ca(fingerprint: &str) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        crate::PrivilegedHelperClient.remove_ca(fingerprint)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = fingerprint;
        Err(unsupported(PlatformCapability::TrustStore)?)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn trust_system_ca_privileged(certificate_path: &Utf8Path) -> Result<(), PlatformError> {
    run_system_command(
        "/usr/bin/security",
        &[
            "add-trusted-cert",
            "-d",
            "-r",
            "trustRoot",
            "-p",
            "ssl",
            "-k",
            SYSTEM_KEYCHAIN_PATH,
            certificate_path.as_str(),
        ],
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn untrust_system_ca_privileged(fingerprint: &str) -> Result<(), PlatformError> {
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
        run_system_command(
            "/usr/bin/security",
            &[
                "delete-certificate",
                "-Z",
                &sha1_fingerprint,
                SYSTEM_KEYCHAIN_PATH,
            ],
        )?;
    }

    Ok(())
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
    use super::certificate_sha1_fingerprint;

    #[test]
    fn certificate_sha1_fingerprint_renders_upper_hex() {
        assert_eq!(
            certificate_sha1_fingerprint(b"abc"),
            "A9993E364706816ABA3E25717850C26C9CD0D89D"
        );
    }
}
