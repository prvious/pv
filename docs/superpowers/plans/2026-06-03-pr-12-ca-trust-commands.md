# PR 12 CA Trust Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build PR 12 by generating PV's local CA files, inspecting local CA and System keychain trust status, and adding non-privileged `pv ca:*` commands.

**Architecture:** The `state` crate adds path helpers for PV's CA certificate and private key. The `macos` crate owns certificate generation, PEM/X.509 validation, fingerprinting, local CA file inspection, and read-only keychain trust classification. The CLI wires those helpers into `ca:status`, `ca:trust`, and `ca:untrust` without running external commands or mutating the System keychain.

**Tech Stack:** Rust, clap, `rcgen 0.14.8`, `rustls-pemfile 2.2.0`, `x509-parser 0.18.1`, `sha2 0.10`, `data-encoding 2.11.0`, `security-framework 3.7.0`, `state::fs`, `insta`, and `cargo nextest`.

---

## File Structure

- Modify `Cargo.toml`: add workspace dependencies for CA generation, PEM parsing, X.509 parsing, fingerprinting, PEM rendering, and macOS keychain inspection.
- Modify `Cargo.lock`: add precise dependency resolutions.
- Modify `crates/macos/Cargo.toml`: depend on CA/keychain libraries.
- Modify `crates/state/src/paths.rs`: add local CA path helpers.
- Modify `crates/state/tests/state_foundation.rs`: add focused CA path coverage.
- Modify `crates/macos/src/lib.rs`: add local CA generation, validation, inspection, fingerprinting, and keychain classification.
- Modify `crates/macos/tests/resolver_config.rs`: extend macOS integration tests with CA snapshots.
- Modify `crates/cli/src/environment.rs`: add injectable keychain inspector access.
- Modify `crates/cli/src/error.rs`: add macOS error conversion.
- Modify `crates/cli/src/lib.rs`: format macOS errors through the normal CLI error path.
- Modify `crates/cli/src/args.rs`: add `ca:status`, `ca:trust`, and `ca:untrust`.
- Modify `crates/cli/src/commands/mod.rs`: route the new commands.
- Create `crates/cli/src/commands/ca.rs`: implement the CA command handlers.
- Create `crates/cli/tests/ca.rs`: add CLI integration snapshots.
- Modify `IMPLEMENTATION.md`: after opening the PR, mark PR 12 as done with the PR number returned by `gh pr view`.

## Task 1: State CA Paths

**Files:**
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/state/tests/state_foundation.rs`

- [ ] **Step 1: Write the failing state path test**

Add this test after `paths_are_derived_from_an_injected_home` in `crates/state/tests/state_foundation.rs`:

```rust
#[test]
fn ca_paths_are_derived_from_an_injected_home() {
    let paths = PvPaths::for_home(Utf8Path::new("/tmp/pv-test-home"));

    assert_eq!(
        paths.ca_certificate().as_str(),
        "/tmp/pv-test-home/.pv/certificates/ca.pem"
    );
    assert_eq!(
        paths.ca_private_key().as_str(),
        "/tmp/pv-test-home/.pv/certificates/ca-key.pem"
    );
}
```

- [ ] **Step 2: Run the focused failing state test**

Run:

```bash
cargo nextest run -p state -E 'test(ca_paths_are_derived_from_an_injected_home)'
```

Expected: compile failure because `PvPaths::ca_certificate` and `PvPaths::ca_private_key` do not exist.

- [ ] **Step 3: Add CA path helpers**

In `crates/state/src/paths.rs`, add these methods inside `impl PvPaths` after `certificates()`:

```rust
pub fn ca_certificate(&self) -> Utf8PathBuf {
    self.certificates().join("ca.pem")
}

pub fn ca_private_key(&self) -> Utf8PathBuf {
    self.certificates().join("ca-key.pem")
}
```

- [ ] **Step 4: Run the focused state test**

Run:

```bash
cargo nextest run -p state -E 'test(ca_paths_are_derived_from_an_injected_home) or test(layout_creates_expected_directories_with_user_only_modes)'
```

Expected: both selected tests pass.

- [ ] **Step 5: Commit state paths**

Run:

```bash
git add crates/state/src/paths.rs crates/state/tests/state_foundation.rs
git commit -m "feat(state): add CA file paths"
```

## Task 2: Local CA Generation and File Inspection

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/macos/Cargo.toml`
- Modify: `crates/macos/src/lib.rs`
- Modify: `crates/macos/tests/resolver_config.rs`

- [ ] **Step 1: Add precise CA dependencies**

In root `Cargo.toml`, add these entries under `[workspace.dependencies]`:

```toml
rcgen = { version = "0.14.8", features = ["x509-parser"] }
rustls-pemfile = "2.2.0"
security-framework = "3.7.0"
sha2 = "0.10"
x509-parser = "0.18.1"
```

In `crates/macos/Cargo.toml`, add:

```toml
rcgen = { workspace = true }
rustls-pemfile = { workspace = true }
security-framework = { workspace = true }
sha2 = { workspace = true }
x509-parser = { workspace = true }
```

Run:

```bash
cargo update -p rcgen --precise 0.14.8
cargo update -p rustls-pemfile --precise 2.2.0
cargo update -p security-framework --precise 3.7.0
cargo update -p sha2 --precise 0.10.9
cargo update -p x509-parser --precise 0.18.1
```

Expected: `Cargo.lock` gains these dependencies and their transitive dependencies only.

- [ ] **Step 2: Add failing local CA generation and inspection tests**

Update the imports in `crates/macos/tests/resolver_config.rs`:

```rust
use macos::{
    CaFileState, CaRepairReason, GeneratedLocalCa, LocalCaMetadata, ResolverConfig,
    generate_local_ca, inspect_local_ca_files, inspect_resolver_file,
};
```

Append these tests after the resolver tests:

```rust
#[test]
fn local_ca_generation_produces_matching_pv_root_certificate_and_key() -> Result<()> {
    let generated = generate_local_ca()?;
    let metadata = LocalCaMetadata::from_pem_pair(
        &generated.certificate_pem,
        &generated.private_key_pem,
    )?;

    assert_eq!(metadata.common_name, "PV Local Development CA");
    assert_eq!(metadata.organization, Some("PV".to_string()));
    assert!(metadata.is_ca);
    assert!(metadata.can_sign_certificates);
    assert!(!metadata.fingerprint.is_empty());
    assert!(generated.certificate_pem.contains("BEGIN CERTIFICATE"));
    assert!(generated.private_key_pem.contains("BEGIN PRIVATE KEY"));

    Ok(())
}

#[test]
fn local_ca_file_inspection_reports_missing_current_repair_required_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let missing_certificate = tempdir.path().join("missing-ca.pem");
    let missing_key = tempdir.path().join("missing-ca-key.pem");
    let current_certificate = tempdir.path().join("current-ca.pem");
    let current_key = tempdir.path().join("current-ca-key.pem");
    let mismatched_certificate = tempdir.path().join("mismatched-ca.pem");
    let mismatched_key = tempdir.path().join("mismatched-ca-key.pem");
    let malformed_certificate = tempdir.path().join("malformed-ca.pem");
    let malformed_key = tempdir.path().join("malformed-ca-key.pem");
    let unreadable_certificate = tempdir.path().join("unreadable-ca.pem");
    let unreadable_key = tempdir.path().join("unreadable-ca-key.pem");
    let first = generate_local_ca()?;
    let second = generate_local_ca()?;

    fs::write_sensitive_file(&current_certificate, &first.certificate_pem)?;
    fs::write_sensitive_file(&current_key, &first.private_key_pem)?;
    fs::write_sensitive_file(&mismatched_certificate, &first.certificate_pem)?;
    fs::write_sensitive_file(&mismatched_key, &second.private_key_pem)?;
    fs::write_sensitive_file(&malformed_certificate, "not a certificate\n")?;
    fs::write_sensitive_file(&malformed_key, &first.private_key_pem)?;
    fs::write_sensitive_file(&unreadable_certificate.join("child"), "child\n")?;
    fs::write_sensitive_file(&unreadable_key, &first.private_key_pem)?;

    let states = vec![
        inspect_local_ca_files(&missing_certificate, &missing_key),
        inspect_local_ca_files(&current_certificate, &current_key),
        inspect_local_ca_files(&mismatched_certificate, &mismatched_key),
        inspect_local_ca_files(&malformed_certificate, &malformed_key),
        inspect_local_ca_files(&unreadable_certificate, &unreadable_key),
    ];
    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn local_ca_file_state_exposes_typed_repair_reasons() -> Result<()> {
    let tempdir = tempdir()?;
    let certificate_path = tempdir.path().join("ca.pem");
    let key_path = tempdir.path().join("ca-key.pem");
    let generated = generate_local_ca()?;

    fs::write_sensitive_file(&certificate_path, &generated.certificate_pem)?;

    let state = inspect_local_ca_files(&certificate_path, &key_path);

    assert!(matches!(
        state,
        CaFileState::RepairRequired {
            reason: CaRepairReason::OneFileMissing,
            ..
        }
    ));

    Ok(())
}
```

- [ ] **Step 3: Run the focused failing macOS tests**

Run:

```bash
cargo nextest run -p macos -E 'test(local_ca_generation_produces_matching_pv_root_certificate_and_key) or test(local_ca_file_inspection_reports_missing_current_repair_required_and_unreadable) or test(local_ca_file_state_exposes_typed_repair_reasons)'
```

Expected: compile failure because the CA types and helpers are not implemented.

- [ ] **Step 4: Add local CA constants, errors, and public types**

In `crates/macos/src/lib.rs`, extend the imports:

```rust
use std::io;
use std::io::Cursor;

use camino::{Utf8Path, Utf8PathBuf};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PublicKeyData, date_time_ymd,
};
use sha2::{Digest, Sha256};
use x509_parser::prelude::FromDer;
use x509_parser::prelude::X509Certificate;
```

Replace the current unit-like `MacosError` with:

```rust
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
```

Add these constants and types below the resolver constants:

```rust
const PV_CA_COMMON_NAME: &str = "PV Local Development CA";
const PV_CA_ORGANIZATION: &str = "PV";

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
```

- [ ] **Step 5: Implement CA generation and metadata parsing**

Add these functions in `crates/macos/src/lib.rs`:

```rust
pub fn generate_local_ca() -> Result<GeneratedLocalCa, MacosError> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, PV_CA_COMMON_NAME);
    distinguished_name.push(DnType::OrganizationName, PV_CA_ORGANIZATION);
    let params = CertificateParams {
        not_before: date_time_ymd(2026, 1, 1),
        not_after: date_time_ymd(2036, 1, 1),
        distinguished_name,
        is_ca: IsCa::Ca(BasicConstraints::Unconstrained),
        key_usages: vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::CrlSign,
        ],
        ..CertificateParams::default()
    };
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
    pub fn from_pem_pair(
        certificate_pem: &str,
        private_key_pem: &str,
    ) -> Result<Self, MacosError> {
        let certificate_der = certificate_der_from_pem(certificate_pem)
            .map_err(|_| MacosError::X509)?;
        let key_pair = KeyPair::from_pem(private_key_pem)
            .map_err(|_| MacosError::InvalidCaShape)?;
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
        let certificate_der = certificate_der_from_pem(certificate_pem)
            .map_err(|_| MacosError::X509)?;
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
```

Add these private helpers:

```rust
fn certificate_der_from_pem(certificate_pem: &str) -> Result<Vec<u8>, io::Error> {
    let mut reader = Cursor::new(certificate_pem.as_bytes());
    let certificates = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()?;

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
```

- [ ] **Step 6: Implement local CA file inspection**

Add:

```rust
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
```

- [ ] **Step 7: Run and accept focused macOS snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- local_ca_file_inspection_reports_missing_current_repair_required_and_unreadable
cargo nextest run -p macos -E 'test(local_ca_generation_produces_matching_pv_root_certificate_and_key) or test(local_ca_file_inspection_reports_missing_current_repair_required_and_unreadable) or test(local_ca_file_state_exposes_typed_repair_reasons)'
```

Expected: all selected tests pass.

- [ ] **Step 8: Commit local CA domain changes**

Run:

```bash
git add Cargo.toml Cargo.lock crates/macos/Cargo.toml crates/macos/src/lib.rs crates/macos/tests/resolver_config.rs crates/macos/tests/snapshots
git commit -m "feat(macos): generate and inspect local CA"
```

## Task 3: Read-Only Keychain Trust Classification

**Files:**
- Modify: `crates/macos/src/lib.rs`
- Modify: `crates/macos/tests/resolver_config.rs`

- [ ] **Step 1: Add failing trust classification tests**

Add these imports to `crates/macos/tests/resolver_config.rs`:

```rust
use macos::{
    KeychainCertificate, KeychainTrustResult, SystemTrustInspector, TrustDomainState,
    inspect_system_ca_trust,
};
```

Append:

```rust
#[test]
fn system_ca_trust_classification_reports_current_missing_stale_denied_and_unknown() -> Result<()> {
    let local = generate_local_ca()?;
    let stale = generate_local_ca()?;
    let local_metadata = LocalCaMetadata::from_pem_pair(
        &local.certificate_pem,
        &local.private_key_pem,
    )?;
    let stale_metadata = LocalCaMetadata::from_pem_pair(
        &stale.certificate_pem,
        &stale.private_key_pem,
    )?;

    let current = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![KeychainCertificate {
            metadata: local_metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        }]),
    );
    let missing = inspect_system_ca_trust(Some(&local_metadata), &FakeTrustInspector::new(vec![]));
    let stale = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![KeychainCertificate {
            metadata: stale_metadata,
            trust: KeychainTrustResult::TrustRoot,
        }]),
    );
    let denied = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![KeychainCertificate {
            metadata: local_metadata.clone(),
            trust: KeychainTrustResult::Deny,
        }]),
    );
    let unknown = inspect_system_ca_trust(None, &FakeTrustInspector::new(vec![]));

    assert_debug_snapshot!((current, missing, stale, denied, unknown));

    Ok(())
}

#[test]
fn system_ca_trust_classification_reports_unreadable_inspector_errors() -> Result<()> {
    let local = generate_local_ca()?;
    let local_metadata = LocalCaMetadata::from_pem_pair(
        &local.certificate_pem,
        &local.private_key_pem,
    )?;
    let state = inspect_system_ca_trust(Some(&local_metadata), &FailingTrustInspector);

    assert_debug_snapshot!(state);

    Ok(())
}

#[derive(Debug)]
struct FakeTrustInspector {
    certificates: Vec<KeychainCertificate>,
}

impl FakeTrustInspector {
    fn new(certificates: Vec<KeychainCertificate>) -> Self {
        Self { certificates }
    }
}

impl SystemTrustInspector for FakeTrustInspector {
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, macos::MacosError> {
        Ok(self.certificates.clone())
    }
}

#[derive(Debug)]
struct FailingTrustInspector;

impl SystemTrustInspector for FailingTrustInspector {
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, macos::MacosError> {
        Err(macos::MacosError::Keychain("fixture failure".to_string()))
    }
}
```

- [ ] **Step 2: Run the focused failing trust tests**

Run:

```bash
cargo nextest run -p macos -E 'test(system_ca_trust_classification_reports_current_missing_stale_denied_and_unknown) or test(system_ca_trust_classification_reports_unreadable_inspector_errors)'
```

Expected: compile failure because trust types and functions do not exist.

- [ ] **Step 3: Add trust classification types and injected inspector trait**

In `crates/macos/src/lib.rs`, add:

```rust
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
```

- [ ] **Step 4: Implement trust classification**

Add:

```rust
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
```

- [ ] **Step 5: Implement read-only macOS System trust inspector**

Add this implementation:

```rust
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
                    Ok(Some(TrustSettingsForCertificate::Invalid)) => KeychainTrustResult::Unspecified,
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
```

Add `data-encoding = "2.11.0"` to `[workspace.dependencies]` in root `Cargo.toml`.
Add `data-encoding = { workspace = true }` to `crates/macos/Cargo.toml`.

Run:

```bash
cargo update -p data-encoding --precise 2.11.0
```

Expected: `Cargo.lock` records `data-encoding` at `2.11.0` if it was not already pinned there.

- [ ] **Step 6: Run and accept focused trust snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- system_ca_trust_classification_reports_current_missing_stale_denied_and_unknown
cargo insta test --accept --test-runner nextest -- system_ca_trust_classification_reports_unreadable_inspector_errors
cargo nextest run -p macos -E 'test(system_ca_trust_classification_reports_current_missing_stale_denied_and_unknown) or test(system_ca_trust_classification_reports_unreadable_inspector_errors)'
```

Expected: all selected tests pass.

- [ ] **Step 7: Commit trust classification changes**

Run:

```bash
git add Cargo.toml Cargo.lock crates/macos/Cargo.toml crates/macos/src/lib.rs crates/macos/tests/resolver_config.rs crates/macos/tests/snapshots
git commit -m "feat(macos): inspect CA trust status"
```

## Task 4: CLI CA Commands

**Files:**
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/cli/src/error.rs`
- Modify: `crates/cli/src/lib.rs`
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Create: `crates/cli/src/commands/ca.rs`
- Create: `crates/cli/tests/ca.rs`

- [ ] **Step 1: Add failing CLI CA integration tests**

Create `crates/cli/tests/ca.rs`:

```rust
use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use macos::{
    KeychainCertificate, KeychainTrustResult, MacosError, generate_local_ca,
};
use state::{PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    certificates: Vec<KeychainCertificate>,
    keychain_error: Option<String>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            certificates: Vec::new(),
            keychain_error: None,
        }
    }

    fn with_certificate(mut self, certificate: KeychainCertificate) -> Self {
        self.certificates.push(certificate);
        self
    }

    fn with_keychain_error(mut self, message: &str) -> Self {
        self.keychain_error = Some(message.to_string());
        self
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.borrow().clone())
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, MacosError> {
        if let Some(message) = &self.keychain_error {
            return Err(MacosError::Keychain(message.clone()));
        }

        Ok(self.certificates.clone())
    }
}

#[test]
fn ca_trust_generates_local_ca_and_defers_system_keychain_trust() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let environment = TestEnvironment::new(&home, &current_dir);
    let paths = pv_paths(&home);

    let output = run_pv(&["ca:trust"], &environment)?;
    let certificate = read_required_file(&paths.ca_certificate())?;
    let private_key = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert!(certificate.contains("BEGIN CERTIFICATE"));
    assert!(private_key.contains("BEGIN PRIVATE KEY"));

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, paths.ca_certificate(), paths.ca_private_key()));
    });

    Ok(())
}

#[test]
fn ca_trust_reuses_existing_current_local_ca() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let environment = TestEnvironment::new(&home, &current_dir);
    let paths = pv_paths(&home);
    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;

    let output = run_pv(&["ca:trust"], &environment)?;
    let certificate_after = read_required_file(&paths.ca_certificate())?;
    let private_key_after = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_eq!(certificate_after, generated.certificate_pem);
    assert_eq!(private_key_after, generated.private_key_pem);

    Ok(())
}

#[test]
fn ca_trust_repairs_malformed_local_ca_files() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let environment = TestEnvironment::new(&home, &current_dir);
    let paths = pv_paths(&home);
    write_file(&paths.ca_certificate(), "not a certificate\n")?;
    write_file(&paths.ca_private_key(), "not a private key\n")?;

    let output = run_pv(&["ca:trust"], &environment)?;
    let certificate_after = read_required_file(&paths.ca_certificate())?;
    let private_key_after = read_required_file(&paths.ca_private_key())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(certificate_after.contains("BEGIN CERTIFICATE"));
    assert!(private_key_after.contains("BEGIN PRIVATE KEY"));

    Ok(())
}

#[test]
fn ca_status_reports_local_and_system_trust_without_creating_files() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let paths = pv_paths(&home);
    let missing_environment = TestEnvironment::new(&home, &current_dir);

    let missing = run_pv(&["ca:status"], &missing_environment)?;
    let certificate_after_missing = read_optional_file(&paths.ca_certificate())?;
    let key_after_missing = read_optional_file(&paths.ca_private_key())?;

    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let current_environment = TestEnvironment::new(&home, &current_dir)
        .with_certificate(KeychainCertificate {
            metadata: generated.metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        });
    let current = run_pv(&["ca:status"], &current_environment)?;

    let unreadable_environment = TestEnvironment::new(&home, &current_dir)
        .with_keychain_error("fixture keychain failure");
    let unreadable = run_pv(&["ca:status"], &unreadable_environment)?;

    assert_eq!(missing.exit_code, ExitCode::SUCCESS);
    assert_eq!(current.exit_code, ExitCode::SUCCESS);
    assert_eq!(unreadable.exit_code, ExitCode::SUCCESS);
    assert!(certificate_after_missing.is_none());
    assert!(key_after_missing.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((missing, current, unreadable));
    });

    Ok(())
}

#[test]
fn ca_untrust_leaves_local_ca_files_and_defers_system_keychain_removal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let paths = pv_paths(&home);
    let generated = generate_local_ca()?;
    write_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    write_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let environment = TestEnvironment::new(&home, &current_dir)
        .with_certificate(KeychainCertificate {
            metadata: generated.metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        });

    let output = run_pv(&["ca:untrust"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_no_privileged_guidance(&output.stdout);
    assert!(read_optional_file(&paths.ca_certificate())?.is_some());
    assert!(read_optional_file(&paths.ca_private_key())?.is_some());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

fn read_required_file(path: &Utf8Path) -> anyhow::Result<String> {
    read_optional_file(path)?
        .ok_or_else(|| anyhow::anyhow!("expected fixture file to exist: {path}"))
}

fn read_optional_file(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    state::fs::write_sensitive_file(path, content)?;

    Ok(())
}

fn assert_no_privileged_guidance(output: &str) {
    for pattern in ["sudo", "security ", "security\n", "openssl"] {
        assert!(
            !output.contains(pattern),
            "output contains privileged guidance `{pattern}`: {output}"
        );
    }
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"[a-f0-9]{64}", "<fingerprint>");
    settings.bind(assertion);
}
```

- [ ] **Step 2: Run the focused failing CLI tests**

Run:

```bash
cargo nextest run -p cli -E 'test(ca_trust_generates_local_ca_and_defers_system_keychain_trust) or test(ca_trust_reuses_existing_current_local_ca) or test(ca_trust_repairs_malformed_local_ca_files) or test(ca_status_reports_local_and_system_trust_without_creating_files) or test(ca_untrust_leaves_local_ca_files_and_defers_system_keychain_removal)'
```

Expected: compile failure because the CA commands and environment hook do not exist.

- [ ] **Step 3: Add CLI environment hook and error conversion**

In `crates/cli/src/environment.rs`, add this default method to `Environment`:

```rust
fn trusted_ca_certificates(&self) -> Result<Vec<macos::KeychainCertificate>, macos::MacosError> {
    macos::SystemTrustInspector::trusted_certificates(&macos::MacosSystemTrustInspector)
}
```

In `crates/cli/src/error.rs`, add this variant to `ExecuteError`:

```rust
#[error(transparent)]
Macos(#[from] macos::MacosError),
```

In `crates/cli/src/lib.rs`, add a branch to `finish_execution`:

```rust
Err(ExecuteError::Macos(error)) => {
    let mut output = Output::new(stderr, output_mode);
    output.error(&error.to_string())?;

    Ok(ExitCode::FAILURE)
}
```

- [ ] **Step 4: Wire command arguments and routing**

In `crates/cli/src/args.rs`, add enum variants after the DNS commands:

```rust
#[command(name = "ca:status", about = "Show PV local CA trust status")]
CaStatus,

#[command(name = "ca:trust", about = "Prepare PV local CA trust")]
CaTrust,

#[command(
    name = "ca:untrust",
    about = "Prepare removal of PV local CA trust"
)]
CaUntrust,
```

In `crates/cli/src/commands/mod.rs`, add:

```rust
mod ca;
```

Add match arms:

```rust
Command::CaStatus => ca::status(environment, stdout),
Command::CaTrust => ca::trust(environment, stdout),
Command::CaUntrust => ca::untrust(environment, stdout),
```

- [ ] **Step 5: Implement `crates/cli/src/commands/ca.rs`**

Create `crates/cli/src/commands/ca.rs`:

```rust
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use macos::{CaFileState, GeneratedLocalCa, LocalCaMetadata, TrustDomainState};
use state::{PvPaths, StateError};

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

pub(crate) fn status(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state = macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("CA trust status")?;
    write_local_ca_state(&mut output, &local_state)?;
    write_system_trust_state(&mut output, &trust_state)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn trust(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let initial_state = macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let (local_state, generated) = ensure_local_ca(&paths, initial_state)?;
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Prepared PV local CA")?;
    match generated {
        Some(generated) => {
            output.line(&format!("  certificate: {}", paths.ca_certificate()))?;
            output.line(&format!("  private key: {}", paths.ca_private_key()))?;
            output.line(&format!("  fingerprint: {}", generated.metadata.fingerprint))?;
        }
        None => output.line("  existing local CA is current")?,
    }
    write_system_trust_state(&mut output, &trust_state)?;
    output.line("Privileged trust installation deferred to PR 13 setup/system-integration work.")?;

    Ok(ExitCode::FAILURE)
}

pub(crate) fn untrust(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state = macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Prepared PV local CA trust removal")?;
    write_local_ca_state(&mut output, &local_state)?;
    write_system_trust_state(&mut output, &trust_state)?;

    match trust_state {
        TrustDomainState::NotTrusted { .. } => {
            output.line("System keychain trust is already absent.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Current { .. } | TrustDomainState::Stale { .. } => {
            output.line("Privileged trust removal deferred to PR 13 setup/system-integration work.")?;
            Ok(ExitCode::FAILURE)
        }
        TrustDomainState::Denied { .. }
        | TrustDomainState::Unknown { .. }
        | TrustDomainState::Unreadable { .. } => Ok(ExitCode::FAILURE),
    }
}
```

Continue the same file:

```rust
fn ensure_local_ca(
    paths: &PvPaths,
    initial_state: CaFileState,
) -> Result<(CaFileState, Option<GeneratedLocalCa>), ExecuteError> {
    if matches!(initial_state, CaFileState::Current { .. }) {
        return Ok((initial_state, None));
    }

    let generated = macos::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let repaired_state = macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());

    Ok((repaired_state, Some(generated)))
}

fn metadata_from_local_state(state: &CaFileState) -> Option<LocalCaMetadata> {
    match state {
        CaFileState::Current { metadata, .. } => Some(metadata.clone()),
        CaFileState::Missing { .. }
        | CaFileState::RepairRequired { .. }
        | CaFileState::Unreadable { .. } => None,
    }
}

fn trust_state(
    environment: &impl Environment,
    metadata: Option<&LocalCaMetadata>,
) -> TrustDomainState {
    struct EnvironmentTrustInspector<'environment, E> {
        environment: &'environment E,
    }

    impl<E: Environment> macos::SystemTrustInspector for EnvironmentTrustInspector<'_, E> {
        fn trusted_certificates(&self) -> Result<Vec<macos::KeychainCertificate>, macos::MacosError> {
            self.environment.trusted_ca_certificates()
        }
    }

    let inspector = EnvironmentTrustInspector { environment };
    macos::inspect_system_ca_trust(metadata, &inspector)
}

fn write_local_ca_state(
    output: &mut Output<'_, impl Write>,
    state: &CaFileState,
) -> io::Result<()> {
    match state {
        CaFileState::Missing {
            certificate_path,
            private_key_path,
        } => {
            output.line("Local CA files: missing")?;
            output.line(&format!("  certificate: {certificate_path}"))?;
            output.line(&format!("  private key: {private_key_path}"))
        }
        CaFileState::Current {
            certificate_path,
            private_key_path,
            metadata,
        } => {
            output.line("Local CA files: current")?;
            output.line(&format!("  certificate: {certificate_path}"))?;
            output.line(&format!("  private key: {private_key_path}"))?;
            output.line(&format!("  common name: {}", metadata.common_name))?;
            output.line(&format!("  fingerprint: {}", metadata.fingerprint))
        }
        CaFileState::RepairRequired {
            certificate_path,
            private_key_path,
            reason,
        } => {
            output.line("Local CA files: repair required")?;
            output.line(&format!("  certificate: {certificate_path}"))?;
            output.line(&format!("  private key: {private_key_path}"))?;
            output.line(&format!("  reason: {reason:?}"))
        }
        CaFileState::Unreadable { path, message } => {
            output.line("Local CA files: unreadable")?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}
```

Finish the file:

```rust
fn write_system_trust_state(
    output: &mut Output<'_, impl Write>,
    state: &TrustDomainState,
) -> io::Result<()> {
    match state {
        TrustDomainState::Current { fingerprint } => {
            output.line("System keychain trust: current")?;
            output.line(&format!("  fingerprint: {fingerprint}"))
        }
        TrustDomainState::NotTrusted { fingerprint } => {
            output.line("System keychain trust: not trusted")?;
            output.line(&format!("  fingerprint: {fingerprint}"))
        }
        TrustDomainState::Stale {
            expected_fingerprint,
            actual_fingerprint,
        } => {
            output.line("System keychain trust: stale")?;
            output.line(&format!("  expected fingerprint: {expected_fingerprint}"))?;
            output.line(&format!("  actual fingerprint: {actual_fingerprint}"))
        }
        TrustDomainState::Denied { fingerprint } => {
            output.line("System keychain trust: denied")?;
            output.line(&format!("  fingerprint: {fingerprint}"))
        }
        TrustDomainState::Unknown { reason } => {
            output.line("System keychain trust: unknown")?;
            output.line(&format!("  {reason}"))
        }
        TrustDomainState::Unreadable { message } => {
            output.line("System keychain trust: unreadable")?;
            output.line(&format!("  {message}"))
        }
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
```

- [ ] **Step 6: Run and accept focused CLI snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- ca_trust_generates_local_ca_and_defers_system_keychain_trust
cargo insta test --accept --test-runner nextest -- ca_status_reports_local_and_system_trust_without_creating_files
cargo insta test --accept --test-runner nextest -- ca_untrust_leaves_local_ca_files_and_defers_system_keychain_removal
cargo nextest run -p cli -E 'test(ca_trust_generates_local_ca_and_defers_system_keychain_trust) or test(ca_trust_reuses_existing_current_local_ca) or test(ca_trust_repairs_malformed_local_ca_files) or test(ca_status_reports_local_and_system_trust_without_creating_files) or test(ca_untrust_leaves_local_ca_files_and_defers_system_keychain_removal)'
```

Expected: all selected tests pass.

- [ ] **Step 7: Commit CLI CA commands**

Run:

```bash
git add crates/cli/src/environment.rs crates/cli/src/error.rs crates/cli/src/lib.rs crates/cli/src/args.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/ca.rs crates/cli/tests/ca.rs crates/cli/tests/snapshots
git commit -m "feat(cli): add CA trust commands"
```

## Task 5: Verification, PR, and Roadmap Update

**Files:**
- Inspect: `DESIGN.md`
- Inspect: `docs/superpowers/specs/2026-06-03-pr-12-ca-trust-commands-design.md`
- Modify: `IMPLEMENTATION.md`

- [ ] **Step 1: Verify no privileged mutation slipped in**

Run:

```bash
rg -n "sudo|security|openssl|SecTrustSettingsSetTrustSettings|SecCertificateAddToKeychain|SecItemDelete|std::process::Command|Command::new" crates/cli crates/macos crates/state
```

Expected:
- `security` appears only in dependency/import identifiers such as `security_framework` or in output safety tests.
- `sudo` and `openssl` do not appear in command output.
- `SecTrustSettingsSetTrustSettings`, `SecCertificateAddToKeychain`, and `SecItemDelete` do not appear in PV code.
- No new process-spawning code appears.

- [ ] **Step 2: Run focused package tests**

Run:

```bash
cargo nextest run -p state -E 'test(ca_paths_are_derived_from_an_injected_home) or test(layout_creates_expected_directories_with_user_only_modes)'
cargo nextest run -p macos -E 'test(local_ca_) or test(system_ca_trust_)'
cargo nextest run -p cli -E 'test(ca_)'
```

Expected: all selected tests pass.

- [ ] **Step 3: Run formatting and diff hygiene**

Run:

```bash
cargo fmt --all -- --check
git diff --check
```

Expected: both pass.

- [ ] **Step 4: Run full workspace verification**

Run:

```bash
cargo nextest run --workspace
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo shear
```

Expected: all pass. If `cargo shear` is unavailable, record that and run the other checks.

- [ ] **Step 5: Commit verification fixes**

If formatting, clippy, or focused test fixes changed files, run:

```bash
git status --short
git add -A
git commit -m "fix: polish CA trust command implementation"
```

Expected: branch is clean after the commit.

- [ ] **Step 6: Push the branch and open the PR**

Run:

```bash
git status --short --branch
git push -u origin feat/pr12-ca-trust-commands
gh pr create --title "feat: add CA trust command preparation" --body-file -
```

Use this PR body:

```markdown
## Summary
- add PV local CA certificate/private-key path helpers
- generate and inspect PV-owned local CA files
- inspect System keychain trust read-only and add `pv ca:*` command output

## Scope
- does not import certificates into the System keychain
- does not change keychain trust settings
- does not run `sudo`, `security`, or `openssl`
- leaves privileged trust installation/removal to PR 13 setup/system-integration work

## Tests
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo shear`
- `cargo fmt --all -- --check`
- `git diff --check`
```

Expected: GitHub returns the PR URL and number.

- [ ] **Step 7: Update `IMPLEMENTATION.md` with the PR number**

Run:

```bash
PR_NUMBER="$(gh pr view --json number --jq '.number')"
perl -0pi -e 's/\| PR 12 \| CA file generation and trust commands \| PV-054 \| PR 2, PR 4 \| Yes \| No \|/| PR 12 | CA file generation and trust commands | PV-054 | PR 2, PR 4 | Yes | Yes (#$ENV{PR_NUMBER}) |/' IMPLEMENTATION.md
```

Expected: the PR 12 row in `IMPLEMENTATION.md` uses the PR number from `gh pr view`.

- [ ] **Step 8: Verify and push the roadmap update**

Run:

```bash
cargo fmt --all -- --check
git diff --check
git add IMPLEMENTATION.md
git commit -m "docs: mark PR 12 roadmap item"
git push
```

Expected: docs commit pushes to the PR branch.

- [ ] **Step 9: Check PR status**

Run:

```bash
gh pr checks --watch
gh pr view --json number,url,headRefOid,mergeStateStatus,latestReviews,comments
```

Expected: CI is passing or pending with no local action required. If CodeRabbit or another reviewer leaves actionable comments, verify each comment against the source before changing code.

## Self-Review Results

- Spec coverage: covered local CA path derivation, generation, validation, repair, read-only keychain status, `ca:status`, `ca:trust`, `ca:untrust`, no privileged mutation, command snapshots, verification, PR creation, and roadmap tracking.
- Placeholder scan: no open-ended implementation instructions remain; each task has concrete files, code, commands, and expected outcomes.
- Type consistency: the plan consistently uses `GeneratedLocalCa`, `LocalCaMetadata`, `CaFileState`, `CaRepairReason`, `KeychainCertificate`, `KeychainTrustResult`, `TrustDomainState`, and `SystemTrustInspector` across macOS and CLI tasks.
