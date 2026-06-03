use std::io;
use std::io::Cursor;

use camino::{Utf8Path, Utf8PathBuf};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, PublicKeyData, date_time_ymd,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use x509_parser::prelude::FromDer;
use x509_parser::prelude::X509Certificate;

pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/resolver/test";
const PV_MARKER: &str = "# Managed by PV";
const PREPARED_MARKER: &str = "# Source: PV prepared resolver config for /etc/resolver/test";
const LOOPBACK_NAMESERVER: &str = "nameserver 127.0.0.1";
const PV_CA_COMMON_NAME: &str = "PV Local Development CA";
const PV_CA_ORGANIZATION: &str = "PV";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolverConfig {
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolverFileState {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        port: u16,
    },
    Stale {
        path: Utf8PathBuf,
        expected_port: Option<u16>,
        actual_port: Option<u16>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum MacosError {
    #[error("could not generate PV local CA: {0}")]
    CaGeneration(#[from] rcgen::Error),

    #[error("could not parse PEM file: {0}")]
    Pem(#[from] io::Error),

    #[error("could not parse X.509 certificate")]
    X509,

    #[error("local CA certificate is not a usable PV root CA")]
    InvalidCaShape,

    #[error("local CA certificate and private key do not match")]
    KeyMismatch,

    #[error("macOS keychain inspection failed: {0}")]
    Keychain(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedLocalCa {
    pub certificate_pem: String,
    pub private_key_pem: String,
    pub metadata: LocalCaMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalCaMetadata {
    pub common_name: String,
    pub organization: Option<String>,
    pub fingerprint: String,
    pub is_ca: bool,
    pub can_sign_certificates: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaFileState {
    Missing {
        certificate_path: Utf8PathBuf,
        private_key_path: Utf8PathBuf,
    },
    Current {
        certificate_path: Utf8PathBuf,
        private_key_path: Utf8PathBuf,
        metadata: LocalCaMetadata,
    },
    RepairRequired {
        certificate_path: Utf8PathBuf,
        private_key_path: Utf8PathBuf,
        reason: CaRepairReason,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CaRepairReason {
    OneFileMissing,
    MalformedCertificate,
    MalformedPrivateKey,
    InvalidCaShape,
    KeyMismatch,
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
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, MacosError>;
}

#[derive(Debug, Default)]
pub struct MacosSystemTrustInspector;

impl ResolverConfig {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn render(&self) -> String {
        format!(
            "{PV_MARKER}\n{PREPARED_MARKER}\n{LOOPBACK_NAMESERVER}\nport {}\n",
            self.port
        )
    }

    pub fn parse(content: &str) -> Option<Self> {
        let mut port = None;
        let mut nameserver_count = 0;
        let mut has_loopback_nameserver = false;

        for line in content.lines().map(str::trim) {
            if line.starts_with("nameserver ") {
                nameserver_count += 1;
                if line == LOOPBACK_NAMESERVER {
                    has_loopback_nameserver = true;
                }
                continue;
            }

            let Some(value) = line.strip_prefix("port ") else {
                continue;
            };

            port = value.parse::<u16>().ok();
        }

        if nameserver_count == 1 && has_loopback_nameserver {
            port.map(Self::new)
        } else {
            None
        }
    }
}

pub fn inspect_resolver_file(
    path: &Utf8Path,
    expected: Option<&ResolverConfig>,
) -> ResolverFileState {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return ResolverFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return ResolverFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) {
        return ResolverFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = ResolverConfig::parse(&content);

    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => ResolverFileState::Current {
            path: path.to_path_buf(),
            port: actual.port,
        },
        (Some(expected), actual) => ResolverFileState::Stale {
            path: path.to_path_buf(),
            expected_port: Some(expected.port),
            actual_port: actual.map(|config| config.port),
        },
        (None, Some(actual)) => ResolverFileState::Current {
            path: path.to_path_buf(),
            port: actual.port,
        },
        (None, None) => ResolverFileState::Stale {
            path: path.to_path_buf(),
            expected_port: None,
            actual_port: None,
        },
    }
}

pub fn generate_local_ca() -> Result<GeneratedLocalCa, MacosError> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, PV_CA_COMMON_NAME);
    distinguished_name.push(DnType::OrganizationName, PV_CA_ORGANIZATION);
    let mut params = CertificateParams::default();
    params.not_before = date_time_ymd(2026, 1, 1);
    params.not_after = date_time_ymd(2036, 1, 1);
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let certificate = params.self_signed(&key_pair)?;
    let certificate_pem = certificate.pem();
    let private_key_pem = key_pair.serialize_pem();
    let metadata = LocalCaMetadata::from_pem_pair(&certificate_pem, &private_key_pem)?;

    Ok(GeneratedLocalCa {
        certificate_pem,
        private_key_pem,
        metadata,
    })
}

impl LocalCaMetadata {
    pub fn from_pem_pair(certificate_pem: &str, private_key_pem: &str) -> Result<Self, MacosError> {
        let certificate_der =
            certificate_der_from_pem(certificate_pem).map_err(|_| MacosError::X509)?;
        let key_pair =
            KeyPair::from_pem(private_key_pem).map_err(|_| MacosError::InvalidCaShape)?;
        let (_remaining, certificate) =
            X509Certificate::from_der(&certificate_der).map_err(|_| MacosError::X509)?;
        let common_name = certificate
            .subject()
            .iter_common_name()
            .next()
            .and_then(|name| name.as_str().ok())
            .ok_or(MacosError::InvalidCaShape)?
            .to_string();
        let organization = certificate
            .subject()
            .iter_organization()
            .next()
            .and_then(|name| name.as_str().ok())
            .map(ToString::to_string);
        let basic_constraints = certificate
            .basic_constraints()
            .map_err(|_| MacosError::InvalidCaShape)?
            .ok_or(MacosError::InvalidCaShape)?;
        let key_usage = certificate
            .key_usage()
            .map_err(|_| MacosError::InvalidCaShape)?
            .ok_or(MacosError::InvalidCaShape)?;
        let is_ca = basic_constraints.value.ca;
        let can_sign_certificates = key_usage.value.key_cert_sign();

        if common_name != PV_CA_COMMON_NAME
            || organization.as_deref() != Some(PV_CA_ORGANIZATION)
            || !is_ca
            || !can_sign_certificates
        {
            return Err(MacosError::InvalidCaShape);
        }

        if certificate.public_key().raw != key_pair.subject_public_key_info().as_slice() {
            return Err(MacosError::KeyMismatch);
        }

        Ok(Self {
            common_name,
            organization,
            fingerprint: certificate_fingerprint(&certificate_der),
            is_ca,
            can_sign_certificates,
        })
    }

    pub fn from_certificate_pem(certificate_pem: &str) -> Result<Self, MacosError> {
        let certificate_der =
            certificate_der_from_pem(certificate_pem).map_err(|_| MacosError::X509)?;
        let (_remaining, certificate) =
            X509Certificate::from_der(&certificate_der).map_err(|_| MacosError::X509)?;
        let common_name = certificate
            .subject()
            .iter_common_name()
            .next()
            .and_then(|name| name.as_str().ok())
            .ok_or(MacosError::InvalidCaShape)?
            .to_string();
        let organization = certificate
            .subject()
            .iter_organization()
            .next()
            .and_then(|name| name.as_str().ok())
            .map(ToString::to_string);
        let basic_constraints = certificate
            .basic_constraints()
            .map_err(|_| MacosError::InvalidCaShape)?
            .ok_or(MacosError::InvalidCaShape)?;
        let key_usage = certificate
            .key_usage()
            .map_err(|_| MacosError::InvalidCaShape)?
            .ok_or(MacosError::InvalidCaShape)?;

        Ok(Self {
            common_name,
            organization,
            fingerprint: certificate_fingerprint(&certificate_der),
            is_ca: basic_constraints.value.ca,
            can_sign_certificates: key_usage.value.key_cert_sign(),
        })
    }
}

pub fn inspect_local_ca_files(
    certificate_path: &Utf8Path,
    private_key_path: &Utf8Path,
) -> CaFileState {
    let certificate_pem = match state::fs::read_to_string(certificate_path) {
        Ok(content) => Some(content),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            None
        }
        Err(error) => {
            return CaFileState::Unreadable {
                path: certificate_path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };
    let private_key_pem = match state::fs::read_to_string(private_key_path) {
        Ok(content) => Some(content),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            None
        }
        Err(error) => {
            return CaFileState::Unreadable {
                path: private_key_path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    match (certificate_pem, private_key_pem) {
        (None, None) => CaFileState::Missing {
            certificate_path: certificate_path.to_path_buf(),
            private_key_path: private_key_path.to_path_buf(),
        },
        (Some(_), None) | (None, Some(_)) => CaFileState::RepairRequired {
            certificate_path: certificate_path.to_path_buf(),
            private_key_path: private_key_path.to_path_buf(),
            reason: CaRepairReason::OneFileMissing,
        },
        (Some(certificate_pem), Some(private_key_pem)) => {
            match LocalCaMetadata::from_pem_pair(&certificate_pem, &private_key_pem) {
                Ok(metadata) => CaFileState::Current {
                    certificate_path: certificate_path.to_path_buf(),
                    private_key_path: private_key_path.to_path_buf(),
                    metadata,
                },
                Err(error) => CaFileState::RepairRequired {
                    certificate_path: certificate_path.to_path_buf(),
                    private_key_path: private_key_path.to_path_buf(),
                    reason: repair_reason_from_ca_error(error),
                },
            }
        }
    }
}

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

    for certificate in certificates {
        let same_fingerprint = certificate.metadata.fingerprint == local.fingerprint;
        let pv_looking = certificate.metadata.common_name == PV_CA_COMMON_NAME
            && certificate.metadata.organization.as_deref() == Some(PV_CA_ORGANIZATION);

        if same_fingerprint {
            return match certificate.trust {
                KeychainTrustResult::TrustRoot | KeychainTrustResult::TrustAsRoot => {
                    TrustDomainState::Current {
                        fingerprint: local.fingerprint.clone(),
                    }
                }
                KeychainTrustResult::Deny => TrustDomainState::Denied {
                    fingerprint: local.fingerprint.clone(),
                },
                KeychainTrustResult::Unspecified => TrustDomainState::NotTrusted {
                    fingerprint: local.fingerprint.clone(),
                },
            };
        }

        if pv_looking
            && matches!(
                certificate.trust,
                KeychainTrustResult::TrustRoot | KeychainTrustResult::TrustAsRoot
            )
        {
            stale_fingerprint = Some(certificate.metadata.fingerprint);
        }
    }

    match stale_fingerprint {
        Some(actual_fingerprint) => TrustDomainState::Stale {
            expected_fingerprint: local.fingerprint.clone(),
            actual_fingerprint,
        },
        None => TrustDomainState::NotTrusted {
            fingerprint: local.fingerprint.clone(),
        },
    }
}

impl SystemTrustInspector for MacosSystemTrustInspector {
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, MacosError> {
        #[cfg(target_os = "macos")]
        {
            use security_framework::trust_settings::{
                Domain, TrustSettings, TrustSettingsForCertificate,
            };

            let trust_settings = TrustSettings::new(Domain::Admin);
            let mut certificates = Vec::new();

            for certificate in trust_settings
                .iter()
                .map_err(|error| MacosError::Keychain(error.to_string()))?
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
                    Err(error) => return Err(MacosError::Keychain(error.to_string())),
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

fn certificate_der_from_pem(certificate_pem: &str) -> Result<Vec<u8>, io::Error> {
    let mut reader = Cursor::new(certificate_pem.as_bytes());
    let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;

    match certificates.as_slice() {
        [certificate] => Ok(certificate.as_ref().to_vec()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected exactly one certificate PEM block",
        )),
    }
}

fn certificate_fingerprint(certificate_der: &[u8]) -> String {
    let digest = Sha256::digest(certificate_der);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn pem_from_der(label: &str, der: &[u8]) -> String {
    let base64 = data_encoding::BASE64.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    for chunk in base64.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----\n"));
    pem
}

fn repair_reason_from_ca_error(error: MacosError) -> CaRepairReason {
    match error {
        MacosError::X509 => CaRepairReason::MalformedCertificate,
        MacosError::InvalidCaShape => CaRepairReason::InvalidCaShape,
        MacosError::KeyMismatch => CaRepairReason::KeyMismatch,
        MacosError::CaGeneration(_) | MacosError::Pem(_) | MacosError::Keychain(_) => {
            CaRepairReason::MalformedPrivateKey
        }
    }
}
