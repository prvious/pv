#![cfg(target_os = "windows")]

use camino::Utf8Path;
use camino_tempfile::tempdir;
use platform::{
    DaemonRegistrationConfig, PlatformCapability, PlatformError, remove_daemon_registration,
    write_daemon_registration_file,
};

#[test]
fn public_launch_agent_write_rejects_unsupported_platform_without_creating_home()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let path = home.join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let config = DaemonRegistrationConfig::for_paths(&state::PvPaths::for_home(&home));

    let result = write_daemon_registration_file(&path, &config);

    assert!(matches!(
        result,
        Err(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonRegistration,
            ..
        })
    ));
    assert!(!home.exists());

    Ok(())
}

#[test]
fn public_launch_agent_removal_rejects_unsupported_platform_without_removing_file()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let path = tempdir.path().join("com.prvious.pv.daemon.plist");
    let original = "existing launch agent\n";
    write_test_file(&path, original)?;

    let result = remove_daemon_registration(&path);

    assert!(matches!(
        result,
        Err(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonRegistration,
            ..
        })
    ));
    assert_eq!(state::fs::read_to_string(&path)?, original);

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "platform integration tests seed portable LaunchAgent fixtures directly"
)]
fn write_test_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    std::fs::write(path, content)?;

    Ok(())
}
