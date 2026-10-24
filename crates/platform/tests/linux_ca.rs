#![cfg(target_os = "linux")]

use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use platform::{
    PlatformError, TrustDomainState, browser_ca_databases, generate_local_ca,
    inspect_browser_ca_trust, trust_browser_ca, untrust_browser_ca,
};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use state::{PvPaths, fs};

#[expect(
    clippy::disallowed_types,
    reason = "isolated NSS integration fixture invokes certutil directly to seed independent trust flags"
)]
type Command = std::process::Command;

#[test]
fn linux_browser_ca_discovery_uses_existing_profiles_without_creating_files() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path();
    let expected = [
        ".local/share/pki/nssdb",
        ".mozilla/firefox/default",
        ".pki/nssdb",
        "snap/chromium/current/.pki/nssdb",
        "snap/firefox/common/.mozilla/firefox/default",
    ];
    assert_eq!(browser_ca_databases(home)?, vec![home.join(".pki/nssdb")]);
    assert!(!fs::path_entry_exists(&home.join(".pki"))?);
    for relative in expected {
        if relative != ".pki/nssdb" {
            fs::write_sensitive_file(&home.join(relative).join("cert9.db"), "fixture")?;
        }
    }
    fs::ensure_user_dir(&home.join(".mozilla/firefox/without-database"))?;
    fs::write_sensitive_file(&home.join(".mozilla/firefox/profiles.ini"), "fixture")?;
    let observed = browser_ca_databases(home)?;
    assert_eq!(observed, expected.map(|relative| home.join(relative)));
    assert!(!fs::path_entry_exists(&home.join(".pki"))?);
    let names = observed
        .iter()
        .map(|path| {
            path.strip_prefix(home)
                .map(|relative| relative.as_str().to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    assert_debug_snapshot!(names);
    Ok(())
}

fn certutil(database: &Utf8Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("/usr/bin/certutil")
        .args(["-d", &format!("sql:{database}")])
        .args(arguments)
        .output()?;
    ensure!(
        output.status.success(),
        "fixture certutil failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
#[ignore = "requires certutil in a disposable Linux environment"]
fn linux_certutil_adds_inspects_repairs_and_removes_only_pv_trust() -> Result<()> {
    let directory = tempdir()?;
    let paths = PvPaths::for_home(directory.path());
    let generated = generate_local_ca()?;
    let other = generate_local_ca()?;
    fs::write_sensitive_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    let other_path = directory.path().join("other.pem");
    fs::write_sensitive_file(&other_path, &other.certificate_pem)?;
    let database = directory.path().join(".pki/nssdb");
    assert!(matches!(
        inspect_browser_ca_trust(&database, Some(&generated.metadata)),
        TrustDomainState::NotTrusted { .. }
    ));
    fs::ensure_user_dir(&database)?;
    untrust_browser_ca(&database, Some(&generated.metadata.fingerprint))?;
    assert!(matches!(
        inspect_browser_ca_trust(&database, Some(&generated.metadata)),
        TrustDomainState::NotTrusted { .. }
    ));
    trust_browser_ca(&database, &paths.ca_certificate())?;
    assert!(
        matches!(inspect_browser_ca_trust(&database, Some(&generated.metadata)), TrustDomainState::Current { fingerprint } if fingerprint == generated.metadata.fingerprint)
    );
    certutil(
        &database,
        &[
            "-A",
            "-n",
            "Other CA",
            "-t",
            "C,,",
            "-i",
            other_path.as_str(),
        ],
    )?;
    let name = format!(
        "PV Local Development CA {}",
        &generated.metadata.fingerprint[..16]
    );
    certutil(&database, &["-M", "-n", &name, "-t", ",,"])?;
    assert!(matches!(
        inspect_browser_ca_trust(&database, Some(&generated.metadata)),
        TrustDomainState::NotTrusted { .. }
    ));
    trust_browser_ca(&database, &paths.ca_certificate())?;
    assert!(
        matches!(inspect_browser_ca_trust(&database, Some(&generated.metadata)), TrustDomainState::Current { fingerprint } if fingerprint == generated.metadata.fingerprint)
    );
    untrust_browser_ca(&database, Some(&generated.metadata.fingerprint))?;
    assert!(matches!(
        inspect_browser_ca_trust(&database, Some(&generated.metadata)),
        TrustDomainState::NotTrusted { .. }
    ));
    certutil(&database, &["-L", "-n", "Other CA"])?;
    let mut parameters = CertificateParams::default();
    parameters
        .distinguished_name
        .push(DnType::CommonName, "Other browser CA");
    parameters.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    parameters.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let misleading = parameters.self_signed(&KeyPair::generate()?)?.pem();
    let misleading_path = directory.path().join("misleading.pem");
    fs::write_sensitive_file(&misleading_path, &misleading)?;
    let misleading_name = format!(
        "PV Local Development CA {}",
        &other.metadata.fingerprint[..16]
    );
    certutil(
        &database,
        &[
            "-A",
            "-n",
            &misleading_name,
            "-t",
            "C,,",
            "-i",
            misleading_path.as_str(),
        ],
    )?;
    assert!(matches!(
        inspect_browser_ca_trust(&database, Some(&other.metadata)),
        TrustDomainState::Unreadable { .. }
    ));
    assert!(matches!(
        trust_browser_ca(&database, &other_path),
        Err(PlatformError::SystemIntegration(_))
    ));
    trust_browser_ca(&database, &paths.ca_certificate())?;
    let listing = certutil(&database, &["-L"])?;
    let misleading_position = listing
        .find(&misleading_name)
        .context("misleading nickname is missing from the fixture")?;
    let pv_position = listing
        .find(&name)
        .context("PV nickname is missing from the fixture")?;
    assert!(misleading_position < pv_position);
    untrust_browser_ca(&database, None)?;
    untrust_browser_ca(&database, None)?;
    assert!(matches!(
        inspect_browser_ca_trust(&database, Some(&generated.metadata)),
        TrustDomainState::NotTrusted { .. }
    ));
    certutil(&database, &["-L", "-n", "Other CA"])?;
    certutil(&database, &["-L", "-n", &misleading_name])?;
    assert!(fs::read_to_string(&paths.ca_certificate())? == generated.certificate_pem);
    assert!(!fs::path_entry_exists(
        &paths.certificates().join("ca-ownership.json")
    )?);
    Ok(())
}
