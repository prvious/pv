use std::ffi::OsStr;
use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::pin;
use std::time::{Duration, Instant};

use async_io::Timer;
use camino::{Utf8Path, Utf8PathBuf};
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use serde::Serialize;
use zbus::address::transport::{Unix, UnixSocket};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy, connection, proxy};

use super::{
    DaemonRegistrationFileState, delete_registration_file, inspect_daemon_registration_file,
};
use crate::PlatformError;

pub const DAEMON_REGISTRATION_LABEL: &str = "pv-daemon.service";
pub const DAEMON_REGISTRATION_FILE_NAME: &str = DAEMON_REGISTRATION_LABEL;
pub(super) const PV_MARKER: &str = "# Managed by PV";

const UNIT: &str = r#"# Managed by PV
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
"#;

/// The fixed per-user unit; systemd resolves its home-relative paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonRegistrationConfig;

impl DaemonRegistrationConfig {
    pub fn for_paths(_paths: &state::PvPaths) -> Self {
        Self
    }

    pub fn render(&self) -> Result<String, PlatformError> {
        Ok(UNIT.to_owned())
    }

    pub fn parse(content: &str) -> Option<Self> {
        (content == UNIT).then_some(Self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DaemonRegistrationStatus {
    pub load_state: String,
    /// Native enablement state, absent for a missing unit file or unqueried lifecycle inspection.
    pub unit_file_state: Option<String>,
    pub active_state: String,
    pub sub_state: String,
    pub main_pid: u32,
    pub fragment_path: Utf8PathBuf,
}

const SERVICE: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const SERVICE_INTERFACE: &str = "org.freedesktop.systemd1.Service";

pub fn daemon_registration_session_address(
    address: Option<&OsStr>,
    runtime_directory: Option<&Path>,
) -> Result<String, PlatformError> {
    if let Some(address) = address.filter(|address| !address.is_empty()) {
        return address
            .to_str()
            .map(str::to_owned)
            .ok_or(PlatformError::UserManagerSessionEnvironment);
    }
    let directory = runtime_directory
        .filter(|path| path.is_absolute())
        .ok_or(PlatformError::UserManagerSessionEnvironment)?;
    Ok(Unix::new(UnixSocket::File(directory.join("bus"))).to_string())
}

fn manager(session_address: &str) -> Result<Proxy<'static>, PlatformError> {
    async_io::block_on(within(Duration::from_secs(10), async {
        let connection = connection::Builder::address(session_address)?
            .build()
            .await?;
        let path = OwnedObjectPath::try_from(MANAGER_PATH)?;
        let manager = native_proxy(&connection, path, MANAGER_INTERFACE).await?;
        // A connected session bus does not imply that it has a systemd user manager.
        manager.get_property::<String>("Version").await?;
        Ok(manager)
    }))
    .ok_or_else(|| {
        PlatformError::UserManagerUnavailable(Box::new(
            io::Error::new(
                io::ErrorKind::TimedOut,
                "session bus connection timed out after 10 seconds",
            )
            .into(),
        ))
    })?
    .map_err(|source| PlatformError::UserManagerUnavailable(Box::new(source)))
}

async fn native_proxy(
    connection: &Connection,
    path: OwnedObjectPath,
    interface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    proxy::Builder::new(connection)
        .destination(SERVICE)?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// Reads runtime and enablement evidence, retaining live units whose files are missing.
pub fn inspect_daemon_registration(
    session_address: &str,
) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
    let manager = manager(session_address)?;
    run_operation("inspect", Duration::from_secs(10), async {
        let mut status = inspect(&manager, "GetUnit").await?;
        if let Some(status) = &mut status {
            status.unit_file_state = match manager
                .call("GetUnitFileState", &(DAEMON_REGISTRATION_LABEL,))
                .await
            {
                Ok(state) => Some(state),
                Err(error)
                    if is_method_error(&error, "org.freedesktop.DBus.Error.FileNotFound") =>
                {
                    None
                }
                Err(error) => return Err(error.into()),
            };
        }
        Ok(status)
    })
}

async fn inspect(
    manager: &Proxy<'_>,
    method: &str,
) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
    let path: OwnedObjectPath = match manager.call(method, &(DAEMON_REGISTRATION_LABEL,)).await {
        Ok(path) => path,
        Err(error) if is_method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    let unit = native_proxy(manager.connection(), path.clone(), UNIT_INTERFACE).await?;
    let drop_ins: Vec<String> = unit.get_property("DropInPaths").await?;
    if let Some(path) = drop_ins.first() {
        return Err(PlatformError::DaemonRegistrationConflict {
            path: path.as_str().into(),
        });
    }
    let service = native_proxy(manager.connection(), path, SERVICE_INTERFACE).await?;
    let load_state: String = unit.get_property("LoadState").await?;
    let fragment_path = unit.get_property::<String>("FragmentPath").await?;
    let status = DaemonRegistrationStatus {
        load_state,
        unit_file_state: None,
        active_state: unit.get_property("ActiveState").await?,
        sub_state: unit.get_property("SubState").await?,
        main_pid: service.get_property("MainPID").await?,
        fragment_path: fragment_path.into(),
    };
    if status.load_state == "not-found"
        && status.fragment_path.as_str().is_empty()
        && status.active_state == "inactive"
        && status.sub_state == "dead"
        && status.main_pid == 0
    {
        Ok(None)
    } else {
        Ok(Some(status))
    }
}

pub(super) fn require_owned_file(path: &Utf8Path) -> Result<(), PlatformError> {
    match inspect_daemon_registration_file(path, None) {
        DaemonRegistrationFileState::Missing { .. }
        | DaemonRegistrationFileState::Current { .. }
        | DaemonRegistrationFileState::Stale { .. } => Ok(()),
        DaemonRegistrationFileState::Conflict { .. } => {
            Err(PlatformError::DaemonRegistrationConflict { path: path.into() })
        }
        DaemonRegistrationFileState::Unreadable { path, message } => {
            Err(PlatformError::DaemonRegistrationUnreadable { path, message })
        }
    }
}

async fn require_owned_unit(
    manager: &Proxy<'_>,
) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
    let status = inspect(manager, "LoadUnit").await?;
    if let Some(status) = &status {
        match inspect_daemon_registration_file(&status.fragment_path, None) {
            DaemonRegistrationFileState::Current { .. }
            | DaemonRegistrationFileState::Stale { .. } => {}
            DaemonRegistrationFileState::Unreadable { path, message } => {
                return Err(PlatformError::DaemonRegistrationUnreadable { path, message });
            }
            _ => {
                return Err(PlatformError::DaemonRegistrationConflict {
                    path: status.fragment_path.clone(),
                });
            }
        }
    }
    Ok(status)
}

pub(super) fn enable(path: &Utf8Path, session_address: &str) -> Result<(), PlatformError> {
    require_owned_file(path)?;
    let manager = manager(session_address)?;
    run_operation("enable", Duration::from_secs(15), async {
        manager.call::<_, _, ()>("Reload", &()).await?;
        require_owned_unit(&manager).await?;
        manager
            .call::<_, _, (bool, Vec<(String, String, String)>)>(
                "EnableUnitFiles",
                &([DAEMON_REGISTRATION_LABEL].as_slice(), false, false),
            )
            .await?;
        Ok(())
    })
}

pub(super) fn reload(session_address: &str) -> Result<(), PlatformError> {
    let manager = manager(session_address)?;
    run_operation("reload", Duration::from_secs(10), async {
        require_owned_unit(&manager).await?;
        manager.call::<_, _, ()>("Reload", &()).await?;
        require_owned_unit(&manager).await?;
        Ok(())
    })
}

pub(super) fn remove(path: &Utf8Path, session_address: &str) -> Result<(), PlatformError> {
    require_owned_file(path)?;
    let manager = manager(session_address)?;
    let started = Instant::now();
    let timeout = Duration::from_secs(10);
    run_operation("remove", timeout, async {
        require_owned_unit(&manager).await?;
        manager
            .call::<_, _, Vec<(String, String, String)>>(
                "DisableUnitFiles",
                &([DAEMON_REGISTRATION_LABEL].as_slice(), false),
            )
            .await?;
        Ok(())
    })?;
    remove_file_and_reload(path, timeout.saturating_sub(started.elapsed()), async {
        manager.call::<_, _, ()>("Reload", &()).await?;
        Ok(())
    })
}

fn remove_file_and_reload(
    path: &Utf8Path,
    timeout: Duration,
    reload: impl Future<Output = Result<(), PlatformError>>,
) -> Result<(), PlatformError> {
    require_owned_file(path)?;
    let previous = match state::fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            None
        }
        Err(error) => return Err(PlatformError::DaemonRegistrationFile(error)),
    };
    delete_registration_file(path)?;
    if let Err(original) = run_operation("remove", timeout, reload) {
        if let Some(content) = previous {
            let restored = require_owned_file(path).and_then(|()| {
                state::fs::write_sensitive_file(path, &content)
                    .map_err(PlatformError::DaemonRegistrationFile)
            });
            if let Err(error) = restored {
                return Err(PlatformError::DaemonRegistrationState {
                    operation: "remove",
                    message: format!("{original}; restoring the unit file also failed: {error}"),
                });
            }
        }
        return Err(original);
    }
    Ok(())
}

pub(super) fn restart(session_address: &str) -> Result<(), PlatformError> {
    run_job(session_address, "RestartUnit", Duration::from_secs(15))
}

pub(super) fn stop(session_address: &str) -> Result<(), PlatformError> {
    run_job(session_address, "StopUnit", Duration::from_secs(10))
}

fn run_job(
    session_address: &str,
    operation: &'static str,
    timeout: Duration,
) -> Result<(), PlatformError> {
    let manager = manager(session_address)?;
    run_operation(operation, timeout, async {
        let previous = require_owned_unit(&manager).await?;
        if operation == "StopUnit" && previous.is_none() {
            return Ok(());
        }
        if let Some(status) = &previous
            && matches!(
                inspect_daemon_registration_file(&status.fragment_path, None),
                DaemonRegistrationFileState::Stale { .. }
            )
        {
            super::write_daemon_registration_file(
                &status.fragment_path,
                &DaemonRegistrationConfig,
            )?;
        }
        // Reload before stopping: a repaired file can still have a cached KillMode=control-group.
        manager.call::<_, _, ()>("Reload", &()).await?;
        let previous = require_owned_unit(&manager).await?;
        manager.call::<_, _, ()>("Subscribe", &()).await?;
        let mut removed = manager.receive_signal("JobRemoved").await?;
        let job: OwnedObjectPath = manager
            .call(operation, &(DAEMON_REGISTRATION_LABEL, "replace"))
            .await?;
        while let Some(message) = removed.next().await {
            let (_, removed_job, _, result): (u32, OwnedObjectPath, String, String) =
                message.body().deserialize()?;
            if removed_job != job {
                continue;
            }
            if result != "done" {
                return Err(PlatformError::DaemonRegistrationState {
                    operation,
                    message: format!("systemd job finished with result {result}"),
                });
            }
            let status = inspect(&manager, "GetUnit").await?;
            let finished = if operation == "StopUnit" {
                status.as_ref().is_none_or(|status| {
                    matches!(status.active_state.as_str(), "inactive" | "failed")
                        && status.main_pid == 0
                })
            } else {
                status.as_ref().is_some_and(|status| {
                    status.active_state == "active"
                        && status.main_pid != 0
                        && previous
                            .as_ref()
                            .is_none_or(|previous| previous.main_pid != status.main_pid)
                })
            };
            return if finished {
                Ok(())
            } else {
                Err(PlatformError::DaemonRegistrationState {
                    operation,
                    message: format!("job completed without the expected daemon state: {status:?}"),
                })
            };
        }
        Err(PlatformError::DaemonRegistrationState {
            operation,
            message: "systemd disconnected before the job completed".to_owned(),
        })
    })
}

fn run_operation<T>(
    operation: &'static str,
    timeout: Duration,
    future: impl Future<Output = Result<T, PlatformError>>,
) -> Result<T, PlatformError> {
    async_io::block_on(within(timeout, future)).ok_or_else(|| {
        PlatformError::DaemonRegistrationState {
            operation,
            message: format!(
                "operation did not complete within {} seconds",
                timeout.as_secs()
            ),
        }
    })?
}

async fn within<T>(timeout: Duration, future: impl Future<Output = T>) -> Option<T> {
    match select(pin!(future), Timer::after(timeout)).await {
        Either::Left((result, _)) => Some(result),
        Either::Right(_) => None,
    }
}

fn is_method_error(error: &zbus::Error, expected: &str) -> bool {
    matches!(error, zbus::Error::MethodError(name, _, _) if name.as_str() == expected)
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};
    use std::time::Duration;

    use camino_tempfile::tempdir;
    use insta::assert_snapshot;
    use state::fs;

    use super::{DaemonRegistrationConfig, remove_file_and_reload};
    use crate::PlatformError;

    #[test]
    fn failed_or_timed_out_removal_reload_keeps_the_unit_retryable() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("pv-daemon.service");
        let content = format!(
            "{}# retained local comment\n",
            DaemonRegistrationConfig.render()?
        );
        for timeout_reload in [false, true] {
            fs::write_sensitive_file(&path, &content)?;
            let result = remove_file_and_reload(&path, Duration::from_millis(10), async {
                if timeout_reload {
                    pending().await
                } else {
                    Err(PlatformError::DaemonRegistrationState {
                        operation: "reload",
                        message: "fixture rejected reload".to_owned(),
                    })
                }
            });
            assert!(matches!(
                result,
                Err(PlatformError::DaemonRegistrationState { .. })
            ));
            assert_eq!(fs::read_to_string(&path)?, content);

            remove_file_and_reload(&path, Duration::from_secs(1), ready(Ok(())))?;
            assert!(!fs::path_entry_exists(&path)?);
        }
        Ok(())
    }

    #[test]
    fn failed_removal_preserves_a_foreign_replacement_and_reports_both_errors() -> anyhow::Result<()>
    {
        let directory = tempdir()?;
        let path = directory.path().join("pv-daemon.service");
        fs::write_sensitive_file(&path, &DaemonRegistrationConfig.render()?)?;
        let foreign = "[Service]\nExecStart=/another/daemon\n";
        let result = remove_file_and_reload(&path, Duration::from_secs(1), async {
            fs::write_sensitive_file(&path, foreign)
                .map_err(PlatformError::DaemonRegistrationFile)?;
            Err(PlatformError::DaemonRegistrationState {
                operation: "reload",
                message: "fixture rejected reload".to_owned(),
            })
        });
        let Err(PlatformError::DaemonRegistrationState { operation, message }) = result else {
            anyhow::bail!("expected a failed removal, got {result:?}");
        };
        assert_eq!(fs::read_to_string(&path)?, foreign);
        assert_eq!(operation, "remove");
        let message = message.replace(directory.path().as_str(), "<tempdir>");
        assert_snapshot!(message, @"daemon registration reload failed: fixture rejected reload; restoring the unit file also failed: daemon registration file is not PV-owned: <tempdir>/pv-daemon.service");
        let retry = remove_file_and_reload(&path, Duration::from_secs(1), ready(Ok(())));
        assert!(matches!(
            retry,
            Err(PlatformError::DaemonRegistrationConflict { path: conflict_path }) if conflict_path == path
        ));
        assert_eq!(fs::read_to_string(&path)?, foreign);
        Ok(())
    }
}
