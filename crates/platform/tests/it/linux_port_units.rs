#![cfg(target_os = "linux")]

use anyhow::Result;
use camino_tempfile::tempdir;
use platform::{
    PfFileState, PfRedirectConfig, inspect_socket_proxy_unit, render_socket_proxy_units,
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
