use std::collections::BTreeSet;

use camino::Utf8Path;

use crate::LocalCaMetadata;
use crate::ca::{certificate_der_from_pem, is_pv_ca_metadata};
use crate::error::PlatformError;

#[cfg(target_os = "macos")]
use crate::ca::pem_from_der;

#[cfg(target_os = "macos")]
use security_framework::certificate::SecCertificate;
#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::SecKeychain;
#[cfg(target_os = "macos")]
use security_framework::trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate};

#[cfg(target_os = "macos")]
const SYSTEM_KEYCHAIN_PATH: &str = "/Library/Keychains/System.keychain";
#[cfg(target_os = "macos")]
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

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

pub fn trust_system_ca(certificate_path: &Utf8Path) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        let certificate_pem = state::fs::read_to_string(certificate_path)
            .map_err(|error| PlatformError::Keychain(error.to_string()))?;
        let certificate_der = certificate_der_from_pem(&certificate_pem)
            .map_err(|error| PlatformError::Keychain(error.to_string()))?;
        let certificate = SecCertificate::from_der(&certificate_der)
            .map_err(|error| PlatformError::Keychain(error.to_string()))?;
        let keychain = SecKeychain::open(SYSTEM_KEYCHAIN_PATH)
            .map_err(|error| PlatformError::Keychain(error.to_string()))?;

        match certificate.add_to_keychain(Some(keychain)) {
            Ok(()) => {}
            Err(error) if error.code() == ERR_SEC_DUPLICATE_ITEM => {}
            Err(error) => return Err(PlatformError::Keychain(error.to_string())),
        }

        TrustSettings::new(Domain::Admin)
            .set_trust_settings_always(&certificate)
            .map_err(|error| PlatformError::Keychain(error.to_string()))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = certificate_path;
        Err(PlatformError::UnsupportedPlatform {
            feature: "System keychain trust mutation",
        })
    }
}

pub fn untrust_system_ca(fingerprint: &str) -> Result<(), PlatformError> {
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

            match certificate.delete() {
                Ok(()) => {}
                Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => {}
                Err(error) => return Err(PlatformError::Keychain(error.to_string())),
            }
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = fingerprint;
        Err(PlatformError::UnsupportedPlatform {
            feature: "System keychain trust mutation",
        })
    }
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
            Err(PlatformError::UnsupportedPlatform {
                feature: "System keychain trust inspection",
            })
        }
    }
}
