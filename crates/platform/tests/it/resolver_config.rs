use std::fmt::Debug;
use std::io::Cursor;

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use platform::{
    GeneratedLocalCa, KeychainCertificate, KeychainTrustResult, LocalCaMetadata, ResolverConfig,
    SystemTrustInspector, TrustDomainState, generate_local_ca, generate_project_certificate,
    inspect_local_ca_files, inspect_resolver_file, inspect_system_ca_trust,
    project_certificate_matches, trusted_pv_ca_fingerprints,
};
use rcgen::{
    CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256,
};
use state::fs;
use time::{Duration, OffsetDateTime};

#[test]
fn resolver_config_renders_pv_owned_test_resolver_file() {
    let config = ResolverConfig::new(35353);
    let rendered = config.render();
    let duplicate_port = format!("{rendered}port 35353\n");
    let duplicate_nameserver =
        rendered.replacen("port 35353\n", "nameserver 127.0.0.1\nport 35353\n", 1);
    let unexpected_active_line = format!("{rendered}search test\n");

    assert_debug_snapshot!(&rendered);
    assert_eq!(ResolverConfig::parse(&rendered), Some(config));
    assert_eq!(ResolverConfig::parse(&duplicate_port), None);
    assert_eq!(ResolverConfig::parse(&duplicate_nameserver), None);
    assert_eq!(ResolverConfig::parse(&unexpected_active_line), None);
}

#[test]
fn resolver_file_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current");
    let stale_path = tempdir.path().join("stale");
    let wrong_nameserver_path = tempdir.path().join("wrong-nameserver");
    let mixed_nameservers_path = tempdir.path().join("mixed-nameservers");
    let conflict_path = tempdir.path().join("conflict");
    let unreadable_path = tempdir.path().join("directory");
    let expected = ResolverConfig::new(35353);

    fs::write_sensitive_file(&current_path, &expected.render())?;
    fs::write_sensitive_file(&stale_path, &ResolverConfig::new(45000).render())?;
    fs::write_sensitive_file(
        &wrong_nameserver_path,
        "# Managed by PV\n# Source: PV prepared resolver config for /etc/resolver/test\nnameserver 10.0.0.1\nport 35353\n",
    )?;
    fs::write_sensitive_file(
        &mixed_nameservers_path,
        "# Managed by PV\n# Source: PV prepared resolver config for /etc/resolver/test\nnameserver 127.0.0.1\nnameserver 10.0.0.1\nport 35353\n",
    )?;
    fs::write_sensitive_file(&conflict_path, "nameserver 127.0.0.1\nport 35353\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_resolver_file(&tempdir.path().join("missing"), Some(&expected)),
        inspect_resolver_file(&current_path, Some(&expected)),
        inspect_resolver_file(&stale_path, Some(&expected)),
        inspect_resolver_file(&wrong_nameserver_path, Some(&expected)),
        inspect_resolver_file(&mixed_nameservers_path, Some(&expected)),
        inspect_resolver_file(&conflict_path, Some(&expected)),
        inspect_resolver_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn local_ca_generation_produces_matching_pv_root_certificate_and_key() -> Result<()> {
    let generated: GeneratedLocalCa = generate_local_ca()?;
    let metadata =
        LocalCaMetadata::from_pem_pair(&generated.certificate_pem, &generated.private_key_pem)?;

    assert_eq!(metadata.common_name, "PV Local Development CA");
    assert_eq!(metadata.organization, Some("PV".to_string()));
    assert!(metadata.is_ca);
    assert!(metadata.can_sign_certificates);
    assert!(!metadata.fingerprint.is_empty());

    Ok(())
}

#[test]
fn project_certificate_generation_signs_primary_hostname_with_local_ca() -> Result<()> {
    let local = generate_local_ca()?;
    let stale = generate_local_ca()?;
    let generated =
        generate_project_certificate("acme.test", &local.certificate_pem, &local.private_key_pem)?;
    let certificate_chain = format!("{}{}", generated.certificate_pem, local.certificate_pem);

    assert!(project_certificate_matches(
        &certificate_chain,
        &generated.private_key_pem,
        "acme.test",
        &local.certificate_pem
    ));
    assert!(!project_certificate_matches(
        &certificate_chain,
        &generated.private_key_pem,
        "api.acme.test",
        &local.certificate_pem
    ));
    assert!(!project_certificate_matches(
        &certificate_chain,
        &generated.private_key_pem,
        "acme.test",
        &stale.certificate_pem
    ));

    Ok(())
}

#[test]
fn project_certificate_generation_uses_current_366_day_validity_profile() -> Result<()> {
    let local = generate_local_ca()?;
    let before_generation = OffsetDateTime::now_utc();
    let generated =
        generate_project_certificate("acme.test", &local.certificate_pem, &local.private_key_pem)?;
    let after_generation = OffsetDateTime::now_utc();
    let certificate_der = first_certificate_der(&generated.certificate_pem)?;
    let (_remaining, certificate) = x509_parser::parse_x509_certificate(&certificate_der)?;
    let validity = certificate.validity();

    assert_time_is_near(
        validity.not_before.to_datetime(),
        before_generation - Duration::days(1),
    );
    assert_time_is_near(
        validity.not_after.to_datetime(),
        after_generation + Duration::days(365),
    );

    Ok(())
}

#[test]
fn project_certificate_match_requires_current_validity_renewal_margin_and_maximum_span()
-> Result<()> {
    let local = generate_local_ca()?;
    let now = OffsetDateTime::now_utc();
    let cases = [
        (
            "not yet valid",
            now + Duration::hours(1),
            now + Duration::days(365),
            false,
        ),
        (
            "expired",
            now - Duration::days(365),
            now - Duration::seconds(1),
            false,
        ),
        (
            "only 30 days remaining",
            now - Duration::days(1),
            now + Duration::days(30),
            false,
        ),
        (
            "too long",
            now - Duration::days(1),
            now + Duration::days(366),
            false,
        ),
        (
            "current and within maximum span",
            now - Duration::days(1),
            now + Duration::days(365),
            true,
        ),
    ];

    for (_name, not_before, not_after, expected) in cases {
        let certificate = project_certificate_with_validity(
            "acme.test",
            &local.certificate_pem,
            &local.private_key_pem,
            not_before,
            not_after,
        )?;
        let certificate_chain = format!("{}{}", certificate.certificate_pem, local.certificate_pem);

        assert_eq!(
            project_certificate_matches(
                &certificate_chain,
                &certificate.private_key_pem,
                "acme.test",
                &local.certificate_pem,
            ),
            expected,
            "{_name}",
        );
    }

    Ok(())
}

fn first_certificate_der(certificate_pem: &str) -> Result<Vec<u8>> {
    let mut reader = Cursor::new(certificate_pem.as_bytes());
    let certificate = rustls_pemfile::certs(&mut reader)
        .next()
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("missing certificate"))?;

    Ok(certificate.as_ref().to_vec())
}

fn assert_time_is_near(actual: OffsetDateTime, expected: OffsetDateTime) {
    assert!((actual - expected).abs() <= Duration::seconds(2));
}

fn project_certificate_with_validity(
    primary_hostname: &str,
    ca_certificate_pem: &str,
    ca_private_key_pem: &str,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
) -> Result<platform::GeneratedProjectCertificate> {
    let ca_key_pair = KeyPair::from_pem(ca_private_key_pem)?;
    let issuer = Issuer::from_ca_cert_pem(ca_certificate_pem, ca_key_pair)?;
    let mut params = CertificateParams::new(vec![primary_hostname.to_string()])?;
    params.not_before = not_before;
    params.not_after = not_after;
    params
        .distinguished_name
        .push(DnType::CommonName, primary_hostname);
    params.use_authority_key_identifier_extension = true;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let certificate = params.signed_by(&key_pair, &issuer)?;

    Ok(platform::GeneratedProjectCertificate {
        certificate_pem: certificate.pem(),
        private_key_pem: key_pair.serialize_pem(),
    })
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
    let malformed_private_key = tempdir.path().join("malformed-private-key.pem");
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
    fs::write_sensitive_file(&malformed_private_key, "not a private key\n")?;

    let states = vec![
        inspect_local_ca_files(&missing_certificate, &missing_key),
        inspect_local_ca_files(&current_certificate, &current_key),
        inspect_local_ca_files(&mismatched_certificate, &mismatched_key),
        inspect_local_ca_files(&malformed_certificate, &malformed_key),
        inspect_local_ca_files(&unreadable_certificate, &unreadable_key),
        inspect_local_ca_files(&current_certificate, &missing_key),
        inspect_local_ca_files(&current_certificate, &malformed_private_key),
    ];
    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn system_ca_trust_classification_reports_current_missing_stale_denied_unspecified_and_unknown()
-> Result<()> {
    let local = generate_local_ca()?;
    let stale = generate_local_ca()?;
    let local_metadata =
        LocalCaMetadata::from_pem_pair(&local.certificate_pem, &local.private_key_pem)?;
    let stale_metadata =
        LocalCaMetadata::from_pem_pair(&stale.certificate_pem, &stale.private_key_pem)?;

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
    let unspecified = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![KeychainCertificate {
            metadata: local_metadata.clone(),
            trust: KeychainTrustResult::Unspecified,
        }]),
    );
    let unknown = inspect_system_ca_trust(None, &FakeTrustInspector::new(vec![]));

    with_normalized_fingerprint_snapshots(|| {
        assert_debug_snapshot!((current, missing, stale, denied, unspecified, unknown));
    });

    Ok(())
}

#[test]
fn system_ca_trust_classification_keeps_scanning_after_exact_unspecified() -> Result<()> {
    let local = generate_local_ca()?;
    let stale = generate_local_ca()?;
    let local_metadata =
        LocalCaMetadata::from_pem_pair(&local.certificate_pem, &local.private_key_pem)?;
    let stale_metadata =
        LocalCaMetadata::from_pem_pair(&stale.certificate_pem, &stale.private_key_pem)?;

    let state = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![
            KeychainCertificate {
                metadata: local_metadata.clone(),
                trust: KeychainTrustResult::Unspecified,
            },
            KeychainCertificate {
                metadata: stale_metadata,
                trust: KeychainTrustResult::TrustRoot,
            },
        ]),
    );

    assert!(matches!(state, TrustDomainState::Stale { .. }));

    Ok(())
}

#[test]
fn system_ca_trust_classification_requires_stale_pv_ca_capability() -> Result<()> {
    let local = generate_local_ca()?;
    let stale = generate_local_ca()?;
    let local_metadata =
        LocalCaMetadata::from_pem_pair(&local.certificate_pem, &local.private_key_pem)?;
    let stale_metadata =
        LocalCaMetadata::from_pem_pair(&stale.certificate_pem, &stale.private_key_pem)?;
    let mut same_subject_non_ca = stale_metadata.clone();
    same_subject_non_ca.is_ca = false;
    same_subject_non_ca.can_sign_certificates = false;
    let mut same_subject_non_signing_ca = stale_metadata;
    same_subject_non_signing_ca.can_sign_certificates = false;

    let non_ca_state = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![KeychainCertificate {
            metadata: same_subject_non_ca,
            trust: KeychainTrustResult::TrustRoot,
        }]),
    );
    let non_signing_ca_state = inspect_system_ca_trust(
        Some(&local_metadata),
        &FakeTrustInspector::new(vec![KeychainCertificate {
            metadata: same_subject_non_signing_ca,
            trust: KeychainTrustResult::TrustRoot,
        }]),
    );

    assert!(matches!(non_ca_state, TrustDomainState::NotTrusted { .. }));
    assert!(matches!(
        non_signing_ca_state,
        TrustDomainState::NotTrusted { .. }
    ));

    Ok(())
}

#[test]
fn trusted_pv_ca_fingerprints_reports_only_pv_owned_trust_entries() -> Result<()> {
    let trusted = generate_local_ca()?;
    let denied = generate_local_ca()?;
    let unspecified = generate_local_ca()?;
    let unrelated = generate_local_ca()?;
    let mut unrelated_metadata =
        LocalCaMetadata::from_pem_pair(&unrelated.certificate_pem, &unrelated.private_key_pem)?;
    unrelated_metadata.common_name = "Unrelated Root".to_string();

    let fingerprints = trusted_pv_ca_fingerprints(&FakeTrustInspector::new(vec![
        KeychainCertificate {
            metadata: trusted.metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        },
        KeychainCertificate {
            metadata: denied.metadata.clone(),
            trust: KeychainTrustResult::Deny,
        },
        KeychainCertificate {
            metadata: unspecified.metadata,
            trust: KeychainTrustResult::Unspecified,
        },
        KeychainCertificate {
            metadata: unrelated_metadata,
            trust: KeychainTrustResult::TrustRoot,
        },
    ]))?;

    let mut expected = vec![denied.metadata.fingerprint, trusted.metadata.fingerprint];
    expected.sort();

    assert_eq!(fingerprints, expected);

    Ok(())
}

#[test]
fn system_ca_trust_classification_reports_unreadable_inspector_errors() -> Result<()> {
    let local = generate_local_ca()?;
    let local_metadata =
        LocalCaMetadata::from_pem_pair(&local.certificate_pem, &local.private_key_pem)?;
    let state = inspect_system_ca_trust(Some(&local_metadata), &FailingTrustInspector);

    with_normalized_fingerprint_snapshots(|| {
        assert_debug_snapshot!(state);
    });

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
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, platform::PlatformError> {
        Ok(self.certificates.clone())
    }
}

#[derive(Debug)]
struct FailingTrustInspector;

impl SystemTrustInspector for FailingTrustInspector {
    fn trusted_certificates(&self) -> Result<Vec<KeychainCertificate>, platform::PlatformError> {
        Err(platform::PlatformError::Keychain(
            "fixture failure".to_string(),
        ))
    }
}

fn normalize_state_debug(state: &impl Debug, tempdir_path: &str) -> String {
    let state_debug =
        normalize_fingerprints(format!("{state:?}").replace(tempdir_path, "[tempdir]"));

    if let Some((prefix, _message)) = state_debug.split_once(", message: ")
        && prefix.starts_with("Unreadable ")
    {
        return format!("{prefix}, message: \"<read error>\" }}");
    }

    state_debug
}

fn normalize_fingerprints(mut state_debug: String) -> String {
    let marker = "fingerprint: \"";
    let mut search_start = 0;

    while let Some(relative_start) = state_debug[search_start..].find(marker) {
        let fingerprint_start = search_start + relative_start + marker.len();
        let Some(relative_end) = state_debug[fingerprint_start..].find('"') else {
            break;
        };
        let fingerprint_end = fingerprint_start + relative_end;
        state_debug.replace_range(fingerprint_start..fingerprint_end, "<fingerprint>");
        search_start = fingerprint_start + "<fingerprint>".len();
    }

    state_debug
}

fn with_normalized_fingerprint_snapshots(assertion: impl FnOnce()) {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"[a-f0-9]{64}", "<fingerprint>");
    settings.bind(assertion);
}
