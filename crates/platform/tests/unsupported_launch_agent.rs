#![cfg(not(target_os = "macos"))]

use camino::Utf8Path;
use camino_tempfile::tempdir;
use platform::{
    LaunchAgentConfig, PlatformCapability, PlatformError, remove_launch_agent_file,
    write_launch_agent_file,
};

#[test]
fn public_launch_agent_write_rejects_unsupported_platform_without_creating_home()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let path = home.join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let config = LaunchAgentConfig::new(
        home.join(".pv/bin/pv"),
        home.join(".pv/logs/launchd.out.log"),
        home.join(".pv/logs/launchd.err.log"),
    );

    let result = write_launch_agent_file(&path, &config);

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

    let result = remove_launch_agent_file(&path);

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
