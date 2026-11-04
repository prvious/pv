#![cfg(target_os = "linux")]

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use platform::{
    DaemonRegistrationConfig, PlatformError, daemon_registration_session_address,
    enable_daemon_registration, inspect_daemon_registration, inspect_daemon_registration_file,
    remove_daemon_registration, restart_daemon_registration, stop_daemon_registration,
};
use state::{PvPaths, fs};
use std::error::Error;

#[test]
fn daemon_unit_keeps_monitors_alive_and_restarts_while_the_user_is_logged_in() -> Result<()> {
    let config = DaemonRegistrationConfig::for_paths(&PvPaths::for_home("/home/alice"));
    let rendered = config.render()?;

    assert_eq!(DaemonRegistrationConfig::parse(&rendered), Some(config));
    assert_snapshot!(rendered, @r#"
    # Managed by PV
    [Unit]
    Description=PV daemon
    StartLimitIntervalSec=0

    [Service]
    Type=simple
    ExecStart=%h/.pv/bin/pv daemon:run
    Restart=always
    RestartSec=1
    KillMode=process
    StandardOutput=append:%h/.pv/logs/systemd.out.log
    StandardError=append:%h/.pv/logs/systemd.err.log

    [Install]
    WantedBy=default.target
    "#);

    Ok(())
}

#[test]
fn daemon_unit_inspection_reports_owned_drift_foreign_and_unreadable_files() -> Result<()> {
    let tempdir = tempdir()?;
    let config = DaemonRegistrationConfig::for_paths(&PvPaths::for_home(tempdir.path()));
    let rendered = config.render()?;
    let current = tempdir.path().join("current.service");
    let stale = tempdir.path().join("stale.service");
    let foreign = tempdir.path().join("foreign.service");
    let unreadable = tempdir.path().join("directory.service");
    fs::write_sensitive_file(&current, &rendered)?;
    fs::write_sensitive_file(&stale, &rendered.replace("RestartSec=1", "RestartSec=2"))?;
    fs::write_sensitive_file(&foreign, &rendered.replace("# Managed by PV\n", ""))?;
    fs::write_sensitive_file(&unreadable.join("child"), "child\n")?;

    let states = [
        tempdir.path().join("missing.service"),
        current,
        stale,
        foreign,
        unreadable,
    ]
    .map(|path| inspect_daemon_registration_file(&path, Some(&config)))
    .map(|state| {
        let state = format!("{state:?}").replace(tempdir.path().as_str(), "<tempdir>");
        if let Some((prefix, _message)) = state.split_once(", message: ")
            && prefix.starts_with("Unreadable ")
        {
            return format!("{prefix}, message: \"<read error>\" }}");
        }
        state
    });
    assert_debug_snapshot!(states, @r#"
    [
        "Missing { path: \"<tempdir>/missing.service\" }",
        "Current { path: \"<tempdir>/current.service\", value: DaemonRegistrationConfig }",
        "Stale { path: \"<tempdir>/stale.service\", expected: Some(DaemonRegistrationConfig), actual: None }",
        "Conflict { path: \"<tempdir>/foreign.service\" }",
        "Unreadable { path: \"<tempdir>/directory.service\", message: \"<read error>\" }",
    ]
    "#);
    Ok(())
}

#[test]
fn daemon_registration_reports_an_unavailable_user_manager_without_changing_the_unit() -> Result<()>
{
    let tempdir = tempdir()?;
    let path = tempdir.path().join("pv-daemon.service");
    let rendered =
        DaemonRegistrationConfig::for_paths(&PvPaths::for_home(tempdir.path())).render()?;
    fs::write_sensitive_file(&path, &rendered)?;
    let address = daemon_registration_session_address(None, Some(tempdir.path().as_std_path()))?;

    for result in [
        inspect_daemon_registration(&address).map(|_| ()),
        enable_daemon_registration(&path, &address),
        remove_daemon_registration(&path, &address),
        stop_daemon_registration(&address),
        restart_daemon_registration(&address),
    ] {
        let Err(error @ PlatformError::UserManagerUnavailable(_)) = result else {
            anyhow::bail!("expected an unavailable user manager, got {result:?}");
        };
        assert!(error.source().is_some());
    }
    assert_eq!(fs::read_to_string(&path)?, rendered);
    Ok(())
}
