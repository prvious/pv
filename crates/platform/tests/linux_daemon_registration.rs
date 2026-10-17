#![cfg(target_os = "linux")]

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use platform::{
    DaemonRegistrationConfig, PlatformError, daemon_registration_session_address,
    enable_daemon_registration, inspect_daemon_registration, inspect_daemon_registration_file,
    remove_daemon_registration, restart_daemon_registration, stop_daemon_registration,
    write_daemon_registration_file,
};
use state::{PvPaths, fs};
use std::error::Error;
use std::io;
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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

#[test]
fn daemon_registration_mutations_preserve_a_foreign_unit_before_connecting_to_the_bus() -> Result<()>
{
    let tempdir = tempdir()?;
    let path = tempdir.path().join("pv-daemon.service");
    let foreign = "[Service]\nExecStart=/some/other/daemon\n";
    fs::write_sensitive_file(&path, foreign)?;

    let config = DaemonRegistrationConfig::for_paths(&PvPaths::for_home(tempdir.path()));
    for result in [
        write_daemon_registration_file(&path, &config),
        enable_daemon_registration(&path, "invalid-bus-address"),
        remove_daemon_registration(&path, "invalid-bus-address"),
    ] {
        assert!(matches!(
            result,
            Err(PlatformError::DaemonRegistrationConflict { path: conflict_path }) if conflict_path == path
        ));
    }
    assert_eq!(fs::read_to_string(&path)?, foreign);
    Ok(())
}

#[test]
fn daemon_registration_bounds_a_stalled_session_bus_handshake() -> Result<()> {
    let tempdir = tempdir()?;
    let listener = UnixListener::bind(tempdir.path().join("bus"))?;
    listener.set_nonblocking(true)?;
    let (stop, release) = mpsc::channel();
    let server = thread::Builder::new()
        .name("stalled-session-bus".to_owned())
        .spawn(move || -> io::Result<bool> {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                match listener.accept() {
                    Ok((_connection, _address)) => {
                        // Keep authentication unanswered until the client returns or the fixture expires.
                        let _release = release.recv_timeout(Duration::from_secs(20));
                        return Ok(true);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if release.try_recv().is_ok() || Instant::now() >= deadline {
                            return Ok(false);
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error),
                }
            }
        })?;
    let address = daemon_registration_session_address(None, Some(tempdir.path().as_std_path()))?;
    let started = Instant::now();
    let result = inspect_daemon_registration(&address);
    let elapsed = started.elapsed();
    let _release = stop.send(());
    let accepted = server
        .join()
        .map_err(|_| anyhow::anyhow!("session bus fixture panicked"))??;

    assert!(accepted);
    let Err(PlatformError::UserManagerUnavailable(source)) = result else {
        anyhow::bail!("expected a timed-out session handshake, got {result:?}");
    };
    let zbus::Error::InputOutput(source) = source.as_ref() else {
        anyhow::bail!("expected a timed-out session handshake, got {source:?}");
    };
    assert_eq!(source.kind(), io::ErrorKind::TimedOut);
    assert!(elapsed < Duration::from_secs(15));
    Ok(())
}
