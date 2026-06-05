use std::fmt::Debug;
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use platform::{
    CaFileState, CaRepairReason, GeneratedLocalCa, KeychainCertificate, KeychainTrustResult,
    LocalCaMetadata, PfConfReference, PfRedirectConfig, ResolverConfig, SystemTrustInspector,
    TrustDomainState, generate_local_ca, inspect_local_ca_files, inspect_pf_anchor_file,
    inspect_pf_conf_reference, inspect_resolver_file, inspect_system_ca_trust,
    loopback_tcp_listener_ports,
};
use state::fs;

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
fn pf_config_renders_pv_owned_anchor_and_pf_conf_reference() {
    let config = PfRedirectConfig::new(48080, 48443);
    let anchor = config.render_anchor();
    let reference = PfConfReference.render();
    let extra_active_line_reference = format!("{reference}set block-policy drop\n");
    let duplicate_anchor_reference = format!("{reference}anchor \"com.prvious.pv\"\n");
    let duplicate_load_reference = format!(
        "{reference}load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n"
    );

    assert_eq!(PfRedirectConfig::parse_anchor(&anchor), Some(config));
    assert_eq!(
        PfConfReference::parse_block(&reference),
        Some(PfConfReference)
    );
    assert_eq!(
        PfConfReference::parse_block(&extra_active_line_reference),
        None
    );
    assert_eq!(
        PfConfReference::parse_block(&duplicate_anchor_reference),
        None
    );
    assert_eq!(
        PfConfReference::parse_block(&duplicate_load_reference),
        None
    );
    assert_debug_snapshot!((anchor, reference));
}

#[test]
fn pf_anchor_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current-anchor");
    let stale_path = tempdir.path().join("stale-anchor");
    let extra_active_rule_path = tempdir.path().join("extra-active-rule-anchor");
    let malformed_path = tempdir.path().join("malformed-anchor");
    let conflict_path = tempdir.path().join("conflict-anchor");
    let unreadable_path = tempdir.path().join("anchor-directory");
    let expected = PfRedirectConfig::new(48080, 48443);

    fs::write_sensitive_file(&current_path, &expected.render_anchor())?;
    fs::write_sensitive_file(
        &stale_path,
        &PfRedirectConfig::new(45000, 45001).render_anchor(),
    )?;
    fs::write_sensitive_file(
        &extra_active_rule_path,
        &format!("{}pass in all\n", expected.render_anchor()),
    )?;
    fs::write_sensitive_file(&malformed_path, "# Managed by PV\npass in all\n")?;
    fs::write_sensitive_file(
        &conflict_path,
        "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\n",
    )?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_pf_anchor_file(&tempdir.path().join("missing-anchor"), Some(&expected)),
        inspect_pf_anchor_file(&current_path, Some(&expected)),
        inspect_pf_anchor_file(&stale_path, Some(&expected)),
        inspect_pf_anchor_file(&extra_active_rule_path, Some(&expected)),
        inspect_pf_anchor_file(&malformed_path, Some(&expected)),
        inspect_pf_anchor_file(&conflict_path, Some(&expected)),
        inspect_pf_anchor_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn pf_conf_reference_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()>
{
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current-pf-conf");
    let stale_path = tempdir.path().join("stale-pf-conf");
    let conflict_path = tempdir.path().join("conflict-pf-conf");
    let active_anchor_conflict_path = tempdir.path().join("active-anchor-conflict-pf-conf");
    let active_load_conflict_path = tempdir.path().join("active-load-conflict-pf-conf");
    let commented_reference_path = tempdir.path().join("commented-reference-pf-conf");
    let prose_reference_path = tempdir.path().join("prose-reference-pf-conf");
    let unrelated_path = tempdir.path().join("unrelated-pf-conf");
    let unreadable_path = tempdir.path().join("pf-conf-directory");
    let expected = PfConfReference;

    fs::write_sensitive_file(
        &current_path,
        &format!(
            "set block-policy drop\n{}\npass out all\n",
            expected.render()
        ),
    )?;
    fs::write_sensitive_file(
        &stale_path,
        "# Managed by PV\nanchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/tmp/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &conflict_path,
        "anchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(&active_anchor_conflict_path, "anchor \"com.prvious.pv\"\n")?;
    fs::write_sensitive_file(
        &active_load_conflict_path,
        "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &commented_reference_path,
        "# anchor \"com.prvious.pv\"\n# load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &prose_reference_path,
        "This note mentions com.prvious.pv and /etc/pf.anchors/com.prvious.pv.\n",
    )?;
    fs::write_sensitive_file(&unrelated_path, "set block-policy drop\npass out all\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_pf_conf_reference(&tempdir.path().join("missing-pf-conf"), Some(&expected)),
        inspect_pf_conf_reference(&current_path, Some(&expected)),
        inspect_pf_conf_reference(&stale_path, Some(&expected)),
        inspect_pf_conf_reference(&conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&active_anchor_conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&active_load_conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&commented_reference_path, Some(&expected)),
        inspect_pf_conf_reference(&prose_reference_path, Some(&expected)),
        inspect_pf_conf_reference(&unrelated_path, Some(&expected)),
        inspect_pf_conf_reference(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn pf_loopback_tcp_listener_ports_include_ipv4_wildcard_listener() -> Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let port = listener.local_addr()?.port();
    let ports = loopback_tcp_listener_ports()?;
    let detection = vec![("ipv4 wildcard listener detected", ports.contains(&port))];

    assert_debug_snapshot!(detection);

    Ok(())
}

#[test]
fn pf_loopback_tcp_listener_ports_include_ipv6_loopback_and_wildcard_listeners() -> Result<()> {
    let loopback_listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0))?;
    let wildcard_listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 0))?;
    let loopback_port = loopback_listener.local_addr()?.port();
    let wildcard_port = wildcard_listener.local_addr()?.port();
    let ports = loopback_tcp_listener_ports()?;
    let detections = vec![
        (
            "ipv6 loopback listener detected",
            ports.contains(&loopback_port),
        ),
        (
            "ipv6 wildcard listener detected",
            ports.contains(&wildcard_port),
        ),
    ];

    assert_debug_snapshot!(detections);

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
    let malformed_key_certificate_path = tempdir.path().join("malformed-key-ca.pem");
    let malformed_key_path = tempdir.path().join("malformed-ca-key.pem");
    let generated = generate_local_ca()?;

    fs::write_sensitive_file(&certificate_path, &generated.certificate_pem)?;
    fs::write_sensitive_file(&malformed_key_certificate_path, &generated.certificate_pem)?;
    fs::write_sensitive_file(&malformed_key_path, "not a private key\n")?;

    let missing_key_state = inspect_local_ca_files(&certificate_path, &key_path);
    let malformed_key_state =
        inspect_local_ca_files(&malformed_key_certificate_path, &malformed_key_path);

    assert!(matches!(
        missing_key_state,
        CaFileState::RepairRequired {
            reason: CaRepairReason::OneFileMissing,
            ..
        }
    ));
    assert!(matches!(
        malformed_key_state,
        CaFileState::RepairRequired {
            reason: CaRepairReason::MalformedPrivateKey,
            ..
        }
    ));

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
