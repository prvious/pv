use crate::LocalCaMetadata;
use crate::ca::{is_pv_ca_metadata, pem_from_der};
use crate::error::PlatformError;

#[cfg(target_os = "macos")]
use security_framework::trust_settings::{Domain, TrustSettings, TrustSettingsForCertificate};

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
            Ok(Vec::new())
        }
    }
}
