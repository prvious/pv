use std::fmt::Display;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy};

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

struct LoadedUnit {
    load_state: String,
    active_state: String,
    sub_state: String,
    main_pid: u32,
    fragment_path: Utf8PathBuf,
}

const OPERATION_TIMEOUT: Limit =
    Limit::new("daemon registration operation", Duration::from_secs(10));
const ACTIVATION_TIMEOUT: Limit =
    Limit::new("daemon registration activation", Duration::from_secs(15));

const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const SERVICE_INTERFACE: &str = "org.freedesktop.systemd1.Service";

fn registration_error(operation: &str, message: impl Display) -> PlatformError {
    PlatformError::SystemIntegration(format!("daemon registration {operation} failed: {message}"))
}

async fn manager() -> Result<Proxy<'static>, PlatformError> {
    let unavailable = |error| {
        registration_error(
            "connection",
            format!("systemd user manager unavailable: {error}"),
        )
    };
    let connection = Connection::session().await.map_err(unavailable)?;
    systemd::system_manager_proxy(&connection)
        .await
        .map_err(unavailable)
}

async fn load(manager: &Proxy<'_>) -> Result<Option<LoadedUnit>, PlatformError> {
    let path: OwnedObjectPath = match manager
        .call("LoadUnit", &(DAEMON_REGISTRATION_LABEL,))
        .await
    {
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
    let status = LoadedUnit {
        load_state,
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

async fn require_owned_unit(manager: &Proxy<'_>) -> Result<Option<LoadedUnit>, PlatformError> {
    let status = load(manager).await?;
    if let Some(status) = &status {
        if status.load_state == "not-found" && status.fragment_path.as_str().is_empty() {
            if status.active_state == "active" {
                let result =
                    systemd::unit_job(manager, "StopUnit", DAEMON_REGISTRATION_LABEL).await?;
                if result != "done" {
                    return Err(registration_error(
                        "StopUnit",
                        format!("systemd job finished with result {result}"),
                    ));
                }
            }
            match manager
                .call::<_, _, ()>("ResetFailedUnit", &(DAEMON_REGISTRATION_LABEL,))
                .await
            {
                Ok(()) => {}
                Err(error) if is_method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => {}
                Err(error) => return Err(error.into()),
            }
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

async fn reload_owned_unit(manager: &Proxy<'_>) -> Result<Option<LoadedUnit>, PlatformError> {
    manager.call::<_, _, ()>("Reload", &()).await?;
    require_owned_unit(manager).await
}

pub(super) fn enable(_path: &Utf8Path) -> Result<(), PlatformError> {
    run_operation("enable", &ACTIVATION_TIMEOUT, async |manager| {
        reload_owned_unit(manager).await?;
        manager
            .call::<_, _, (bool, Vec<(String, String, String)>)>(
                "EnableUnitFiles",
                &([DAEMON_REGISTRATION_LABEL].as_slice(), false, false),
            )
            .await?;
        Ok(())
    })
}

pub(super) fn remove(path: &Utf8Path) -> Result<(), PlatformError> {
    run_operation("remove", &OPERATION_TIMEOUT, async |manager| {
        if reload_owned_unit(manager).await?.is_some() {
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

pub(super) fn restart() -> Result<(), PlatformError> {
    run_job("RestartUnit", &ACTIVATION_TIMEOUT)
}

pub(super) fn stop() -> Result<(), PlatformError> {
    run_job("StopUnit", &OPERATION_TIMEOUT)
}

fn run_job(operation: &'static str, limit: &Limit) -> Result<(), PlatformError> {
    run_operation(operation, limit, async |manager| {
        let previous = reload_owned_unit(manager).await?;
        if operation == "StopUnit" && previous.is_none() {
            return Ok(());
        }
        let result = systemd::unit_job(manager, operation, DAEMON_REGISTRATION_LABEL).await?;
        if result == "done" {
            Ok(())
        } else {
            Err(registration_error(
                operation,
                format!("systemd job finished with result {result}"),
            ))
        }
    })
}

/// Connects to the user manager and runs `work` on one runtime, all within `limit`.
fn run_operation<T>(
    operation: &'static str,
    limit: &Limit,
    work: impl AsyncFnOnce(&Proxy<'static>) -> Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    runtime
        .block_on(clock::timeout(limit, async {
            work(&manager().await?).await
        }))
        .map_err(|_elapsed| {
            registration_error(
                operation,
                format!(
                    "operation did not complete within {} seconds",
                    limit.duration().as_secs()
                ),
            )
        })?
}
