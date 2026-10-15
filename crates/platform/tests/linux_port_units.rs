#![cfg(target_os = "linux")]

use anyhow::Result;
use camino_tempfile::tempdir;
use platform::{
    PfFileState, PfRedirectConfig, inspect_socket_proxy_unit, render_socket_proxy_units,
    socket_proxy_files_current,
};
use state::fs;

#[test]
fn prepared_socket_proxy_files_require_exact_owned_regular_files() -> Result<()> {
    let directory = tempdir()?;
    let config = PfRedirectConfig::new(48080, 48443);
    for (name, expected) in render_socket_proxy_units(&config) {
        let path = directory.path().join(name);
        assert_eq!(
            inspect_socket_proxy_unit(&path, Some(&expected)),
            PfFileState::Missing { path: path.clone() }
        );
        fs::write_sensitive_file(&path, &expected)?;
        let current = PfFileState::Current {
            path: path.clone(),
            value: expected.clone(),
        };
        assert_eq!(inspect_socket_proxy_unit(&path, Some(&expected)), current);
        assert_eq!(inspect_socket_proxy_unit(&path, None), current);

        let stale = format!("{expected}\n# changed by an administrator\n");
        fs::write_sensitive_file(&path, &stale)?;
        assert_eq!(
            inspect_socket_proxy_unit(&path, Some(&expected)),
            PfFileState::Stale {
                path: path.clone(),
                expected: Some(expected.clone()),
                actual: Some(stale),
            }
        );
        fs::write_sensitive_file(&path, "[Unit]\nDescription=Administrator service\n")?;
        assert_eq!(
            inspect_socket_proxy_unit(&path, Some(&expected)),
            PfFileState::Conflict { path: path.clone() }
        );
        fs::delete_file(&path)?;
        fs::ensure_user_dir(&path)?;
        assert!(matches!(
            inspect_socket_proxy_unit(&path, Some(&expected)),
            PfFileState::Unreadable { path: actual, .. } if actual == path
        ));
    }
    Ok(())
}

#[test]
fn socket_proxy_currency_requires_every_prepared_and_installed_file() -> Result<()> {
    let directory = tempdir()?;
    let prepared = directory.path().join("prepared");
    let installed = directory.path().join("installed");
    let config = PfRedirectConfig::new(48080, 48443);
    for directory in [&prepared, &installed] {
        for (name, content) in render_socket_proxy_units(&config) {
            fs::write_sensitive_file(&directory.join(name), &content)?;
        }
    }
    assert!(socket_proxy_files_current(&config, &prepared, &installed));
    assert!(!socket_proxy_files_current(
        &PfRedirectConfig::new(45080, 45443),
        &prepared,
        &installed,
    ));

    for directory in [&prepared, &installed] {
        for (name, expected) in render_socket_proxy_units(&config) {
            let path = directory.join(name);
            fs::delete_file(&path)?;
            assert!(!socket_proxy_files_current(&config, &prepared, &installed));
            for content in [
                format!("{expected}\n# changed\n"),
                "[Unit]\nDescription=Administrator service\n".to_owned(),
            ] {
                fs::write_sensitive_file(&path, &content)?;
                assert!(!socket_proxy_files_current(&config, &prepared, &installed));
            }
            fs::delete_file(&path)?;
            fs::ensure_user_dir(&path)?;
            assert!(!socket_proxy_files_current(&config, &prepared, &installed));
            fs::delete_dir_all(&path)?;
            fs::write_sensitive_file(&path, &expected)?;
            assert!(socket_proxy_files_current(&config, &prepared, &installed));
        }
    }
    Ok(())
}
