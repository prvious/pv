use std::io;
use std::io::Cursor;

use camino::{Utf8Path, Utf8PathBuf};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PublicKeyData, date_time_ymd,
};
use sha2::{Digest, Sha256};
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::FromDer;
use x509_parser::prelude::X509Certificate;

use crate::error::PlatformError;

pub(crate) const PV_CA_COMMON_NAME: &str = "PV Local Development CA";
pub(crate) const PV_CA_ORGANIZATION: &str = "PV";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedLocalCa {
    pub certificate_pem: String,
    pub private_key_pem: String,
    pub metadata: LocalCaMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedProjectCertificate {
    pub certificate_pem: String,
    pub private_key_pem: String,
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

pub fn generate_local_ca() -> Result<GeneratedLocalCa, PlatformError> {
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

pub fn generate_project_certificate(
    primary_hostname: &str,
    ca_certificate_pem: &str,
    ca_private_key_pem: &str,
) -> Result<GeneratedProjectCertificate, PlatformError> {
    let ca_key_pair = KeyPair::from_pem(ca_private_key_pem)
        .map_err(PlatformError::ProjectCertificateGeneration)?;
    let issuer = Issuer::from_ca_cert_pem(ca_certificate_pem, ca_key_pair)
        .map_err(PlatformError::ProjectCertificateGeneration)?;
    let mut params = CertificateParams::new(vec![primary_hostname.to_string()])
        .map_err(PlatformError::ProjectCertificateGeneration)?;
    params.not_before = date_time_ymd(2026, 1, 1);
    params.not_after = date_time_ymd(2036, 1, 1);
    params
        .distinguished_name
        .push(DnType::CommonName, primary_hostname);
    params.use_authority_key_identifier_extension = true;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(PlatformError::ProjectCertificateGeneration)?;
    let certificate = params
        .signed_by(&key_pair, &issuer)
        .map_err(PlatformError::ProjectCertificateGeneration)?;

    Ok(GeneratedProjectCertificate {
        certificate_pem: certificate.pem(),
        private_key_pem: key_pair.serialize_pem(),
    })
}

pub fn project_certificate_matches(
    certificate_chain_pem: &str,
    private_key_pem: &str,
    primary_hostname: &str,
    ca_certificate_pem: &str,
) -> bool {
    let Ok(certificate_der) = first_certificate_der_from_pem(certificate_chain_pem) else {
        return false;
    };
    let Ok(ca_certificate_der) = certificate_der_from_pem(ca_certificate_pem) else {
        return false;
    };
    let Ok(key_pair) = KeyPair::from_pem(private_key_pem) else {
        return false;
    };
    let Ok((_remaining, certificate)) = X509Certificate::from_der(&certificate_der) else {
        return false;
    };
    let Ok((_remaining, ca_certificate)) = X509Certificate::from_der(&ca_certificate_der) else {
        return false;
    };

    if certificate.public_key().raw != key_pair.subject_public_key_info().as_slice() {
        return false;
    }
    if certificate
        .verify_signature(Some(&ca_certificate.tbs_certificate.subject_pki))
        .is_err()
    {
        return false;
    }

    certificate_has_dns_name(&certificate, primary_hostname)
}

impl LocalCaMetadata {
    pub fn from_pem_pair(
        certificate_pem: &str,
        private_key_pem: &str,
    ) -> Result<Self, PlatformError> {
        let certificate_der =
            certificate_der_from_pem(certificate_pem).map_err(|_| PlatformError::X509)?;
        let key_pair =
            KeyPair::from_pem(private_key_pem).map_err(|_| PlatformError::MalformedPrivateKey)?;
        let (_remaining, certificate) =
            X509Certificate::from_der(&certificate_der).map_err(|_| PlatformError::X509)?;
        let common_name = certificate
            .subject()
            .iter_common_name()
            .next()
            .and_then(|name| name.as_str().ok())
            .ok_or(PlatformError::InvalidCaShape)?
            .to_string();
        let organization = certificate
            .subject()
            .iter_organization()
            .next()
            .and_then(|name| name.as_str().ok())
            .map(ToString::to_string);
        let basic_constraints = certificate
            .basic_constraints()
            .map_err(|_| PlatformError::InvalidCaShape)?
            .ok_or(PlatformError::InvalidCaShape)?;
        let key_usage = certificate
            .key_usage()
            .map_err(|_| PlatformError::InvalidCaShape)?
            .ok_or(PlatformError::InvalidCaShape)?;
        let is_ca = basic_constraints.value.ca;
        let can_sign_certificates = key_usage.value.key_cert_sign();

        if common_name != PV_CA_COMMON_NAME
            || organization.as_deref() != Some(PV_CA_ORGANIZATION)
            || !is_ca
            || !can_sign_certificates
        {
            return Err(PlatformError::InvalidCaShape);
        }

        if certificate.public_key().raw != key_pair.subject_public_key_info().as_slice() {
            return Err(PlatformError::KeyMismatch);
        }

        Ok(Self {
            common_name,
            organization,
            fingerprint: certificate_fingerprint(&certificate_der),
            is_ca,
            can_sign_certificates,
        })
    }

    pub fn from_certificate_pem(certificate_pem: &str) -> Result<Self, PlatformError> {
        let certificate_der =
            certificate_der_from_pem(certificate_pem).map_err(|_| PlatformError::X509)?;
        let (_remaining, certificate) =
            X509Certificate::from_der(&certificate_der).map_err(|_| PlatformError::X509)?;
        let common_name = certificate
            .subject()
            .iter_common_name()
            .next()
            .and_then(|name| name.as_str().ok())
            .ok_or(PlatformError::InvalidCaShape)?
            .to_string();
        let organization = certificate
            .subject()
            .iter_organization()
            .next()
            .and_then(|name| name.as_str().ok())
            .map(ToString::to_string);
        let basic_constraints = certificate
            .basic_constraints()
            .map_err(|_| PlatformError::InvalidCaShape)?
            .ok_or(PlatformError::InvalidCaShape)?;
        let key_usage = certificate
            .key_usage()
            .map_err(|_| PlatformError::InvalidCaShape)?
            .ok_or(PlatformError::InvalidCaShape)?;

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

pub(crate) fn is_pv_ca_metadata(metadata: &LocalCaMetadata) -> bool {
    metadata.common_name == PV_CA_COMMON_NAME
        && metadata.organization.as_deref() == Some(PV_CA_ORGANIZATION)
        && metadata.is_ca
        && metadata.can_sign_certificates
}

#[cfg(target_os = "macos")]
pub(crate) fn pem_from_der(label: &str, der: &[u8]) -> String {
    let base64 = data_encoding::BASE64.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    for chunk in base64.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----\n"));
    pem
}

pub(crate) fn certificate_der_from_pem(certificate_pem: &str) -> Result<Vec<u8>, io::Error> {
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

fn first_certificate_der_from_pem(certificate_pem: &str) -> Result<Vec<u8>, io::Error> {
    let mut reader = Cursor::new(certificate_pem.as_bytes());
    let mut certificates = rustls_pemfile::certs(&mut reader);

    match certificates.next().transpose()? {
        Some(certificate) => Ok(certificate.as_ref().to_vec()),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected at least one certificate PEM block",
        )),
    }
}

fn certificate_has_dns_name(certificate: &X509Certificate<'_>, primary_hostname: &str) -> bool {
    let Ok(Some(subject_alternative_name)) = certificate.subject_alternative_name() else {
        return false;
    };

    subject_alternative_name
        .value
        .general_names
        .iter()
        .any(|name| matches!(name, GeneralName::DNSName(hostname) if *hostname == primary_hostname))
}

fn certificate_fingerprint(certificate_der: &[u8]) -> String {
    let digest = Sha256::digest(certificate_der);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn repair_reason_from_ca_error(error: PlatformError) -> CaRepairReason {
    match error {
        PlatformError::X509 => CaRepairReason::MalformedCertificate,
        PlatformError::MalformedPrivateKey => CaRepairReason::MalformedPrivateKey,
        PlatformError::InvalidCaShape => CaRepairReason::InvalidCaShape,
        PlatformError::KeyMismatch => CaRepairReason::KeyMismatch,
        PlatformError::CaGeneration(_)
        | PlatformError::ProjectCertificateGeneration(_)
        | PlatformError::Pem(_)
        | PlatformError::LocalCaPostWriteMissing
        | PlatformError::LocalCaPostWriteRepairRequired { .. }
        | PlatformError::LocalCaPostWriteUnreadable { .. }
        | PlatformError::Unsupported { .. }
        | PlatformError::UnsupportedTarget { .. }
        | PlatformError::BrowserOpen(_)
        | PlatformError::BrowserOpenStatus { .. }
        | PlatformError::Keychain(_)
        | PlatformError::LaunchAgent(_)
        | PlatformError::LaunchAgentCommand { .. }
        | PlatformError::LaunchAgentCommandStatus { .. }
        | PlatformError::SystemIntegration(_)
        | PlatformError::SystemIntegrationCommand { .. }
        | PlatformError::SystemIntegrationCommandStatus { .. } => CaRepairReason::InvalidCaShape,
        #[cfg(target_os = "macos")]
        PlatformError::SocketTable(_)
        | PlatformError::SocketTableCommand(_)
        | PlatformError::SocketTableCommandStatus { .. }
        | PlatformError::SocketTableCommandUtf8(_) => CaRepairReason::InvalidCaShape,
    }
}
