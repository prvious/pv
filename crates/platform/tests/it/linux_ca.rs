#![cfg(target_os = "linux")]

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use platform::browser_ca_databases;
use state::fs;

#[test]
fn linux_browser_ca_discovery_uses_existing_profiles_without_creating_files() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path();
    let expected = [
        ".config/mozilla/firefox/default",
        ".local/share/pki/nssdb",
        ".mozilla/firefox/default",
        ".pki/nssdb",
        "snap/chromium/current/.pki/nssdb",
        "snap/firefox/common/.mozilla/firefox/default",
    ];
    assert_eq!(
        browser_ca_databases(home, &home.join(".config"))?,
        vec![home.join(".pki/nssdb")]
    );
    assert!(!fs::path_entry_exists(&home.join(".pki"))?);
    for relative in expected {
        if relative != ".pki/nssdb" {
            fs::write_sensitive_file(&home.join(relative).join("cert9.db"), "fixture")?;
        }
    }
    fs::ensure_user_dir(&home.join(".mozilla/firefox/without-database"))?;
    fs::write_sensitive_file(&home.join(".mozilla/firefox/profiles.ini"), "fixture")?;
    let observed = browser_ca_databases(home, &home.join(".config"))?;
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
