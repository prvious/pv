use std::fmt::Debug;

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use macos::{
    CaFileState, CaRepairReason, GeneratedLocalCa, LocalCaMetadata, ResolverConfig,
    generate_local_ca, inspect_local_ca_files, inspect_resolver_file,
};
use state::fs;

#[test]
fn resolver_config_renders_pv_owned_test_resolver_file() {
    let config = ResolverConfig::new(35353);
    let rendered = config.render();

    assert_debug_snapshot!(&rendered);
    assert_eq!(ResolverConfig::parse(&rendered), Some(config));
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
