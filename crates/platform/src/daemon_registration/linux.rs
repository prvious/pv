use std::ffi::OsStr;
use std::future::Future;
use std::io;
use std::path::Path;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use zbus::address::transport::{Unix, UnixSocket};
use zbus::zvariant::OwnedObjectPath;
use zbus::{Proxy, connection};

use super::{
    DaemonRegistrationFileState, delete_registration_file, inspect_daemon_registration_file,
};
use crate::PlatformError;
use crate::clock::{self, Limit};
use crate::systemd::{self, is_method_error, native_proxy};

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

const CONNECTION_TIMEOUT: Limit =
    Limit::new("daemon registration connection", Duration::from_secs(10));
const OPERATION_TIMEOUT: Limit =
    Limit::new("daemon registration operation", Duration::from_secs(10));
const ACTIVATION_TIMEOUT: Limit =
    Limit::new("daemon registration activation", Duration::from_secs(15));

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
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| PlatformError::UserManagerUnavailable(Box::new(source.into())))?;
    runtime
        .block_on(clock::timeout(&CONNECTION_TIMEOUT, async {
            let connection = connection::Builder::address(session_address)?
                .build()
                .await?;
            systemd::system_manager_proxy(&connection).await
        }))
        .map_err(|_elapsed| {
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

/// Reads runtime and enablement evidence, retaining live units whose files are missing.
pub fn inspect_daemon_registration(
    session_address: &str,
) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
    let manager = manager(session_address)?;
    run_operation("inspect", &OPERATION_TIMEOUT, async {
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

async fn require_owned_unit(
    manager: &Proxy<'_>,
) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
    let status = inspect(manager, "LoadUnit").await?;
    if let Some(status) = &status {
        if status.load_state == "not-found" && status.fragment_path.as_str().is_empty() {
            if status.active_state == "active" {
                let result =
                    systemd::unit_job(manager, "StopUnit", DAEMON_REGISTRATION_LABEL).await?;
                if result != "done" {
                    return Err(PlatformError::DaemonRegistrationState {
                        operation: "StopUnit",
                        message: format!("systemd job finished with result {result}"),
                    });
                }
            }
            manager
                .call::<_, _, ()>("ResetFailedUnit", &(DAEMON_REGISTRATION_LABEL,))
                .await?;
            return Ok(None);
        }
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

async fn reload_owned_unit(
    manager: &Proxy<'_>,
) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
    manager.call::<_, _, ()>("Reload", &()).await?;
    require_owned_unit(manager).await
}

pub(super) fn enable(_path: &Utf8Path, session_address: &str) -> Result<(), PlatformError> {
    let manager = manager(session_address)?;
    run_operation("enable", &ACTIVATION_TIMEOUT, async {
        reload_owned_unit(&manager).await?;
        manager
            .call::<_, _, (bool, Vec<(String, String, String)>)>(
                "EnableUnitFiles",
                &([DAEMON_REGISTRATION_LABEL].as_slice(), false, false),
            )
            .await?;
        Ok(())
    })
}

pub(super) fn remove(path: &Utf8Path, session_address: &str) -> Result<(), PlatformError> {
    let manager = manager(session_address)?;
    run_operation("remove", &OPERATION_TIMEOUT, async {
        if reload_owned_unit(&manager).await?.is_some() {
            manager
                .call::<_, _, Vec<(String, String, String)>>(
                    "DisableUnitFiles",
                    &([DAEMON_REGISTRATION_LABEL].as_slice(), false),
                )
                .await?;
        }
        delete_registration_file(path)?;
        manager.call::<_, _, ()>("Reload", &()).await?;
        Ok(())
    })
}

pub(super) fn restart(session_address: &str) -> Result<(), PlatformError> {
    run_job(session_address, "RestartUnit", &ACTIVATION_TIMEOUT)
}

pub(super) fn stop(session_address: &str) -> Result<(), PlatformError> {
    run_job(session_address, "StopUnit", &OPERATION_TIMEOUT)
}

fn run_job(
    session_address: &str,
    operation: &'static str,
    limit: &Limit,
) -> Result<(), PlatformError> {
    let manager = manager(session_address)?;
    run_operation(operation, limit, async {
        let previous = reload_owned_unit(&manager).await?;
        if operation == "StopUnit" && previous.is_none() {
            return Ok(());
        }
        let result = systemd::unit_job(&manager, operation, DAEMON_REGISTRATION_LABEL).await?;
        if result == "done" {
            Ok(())
        } else {
            Err(PlatformError::DaemonRegistrationState {
                operation,
                message: format!("systemd job finished with result {result}"),
            })
        }
    })
}

fn run_operation<T>(
    operation: &'static str,
    limit: &Limit,
    future: impl Future<Output = Result<T, PlatformError>>,
) -> Result<T, PlatformError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    runtime
        .block_on(clock::timeout(limit, future))
        .map_err(|_elapsed| PlatformError::DaemonRegistrationState {
            operation,
            message: format!(
                "operation did not complete within {} seconds",
                limit.duration().as_secs()
            ),
        })?
}
