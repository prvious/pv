use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use rustix::process::{geteuid, getuid};
use zbus::{Proxy, connection};

use super::{
    HTTP, HTTPS, SYSTEM_DIRECTORY, UnitStatus, bus_error, observe, read_loaded_unit, unit_names,
};
use crate::helper::linux::{self as root_files, RootFile};
use crate::helper::validate_high_port;
use crate::systemd::{
    read_owned_unit, require_no_candidate, system_manager_proxy, unit_job, within, write_owned_unit,
};
use crate::{PfRedirectConfig, PlatformError};

const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);
const APPLY_TIMEOUT: Duration = Duration::from_secs(45);
const ROLLBACK_TIMEOUT: Duration = Duration::from_secs(45);
pub(crate) const MUTATION_TIMEOUT: Duration = PREFLIGHT_TIMEOUT
    .saturating_add(APPLY_TIMEOUT)
    .saturating_add(ROLLBACK_TIMEOUT);

struct PreviousUnit {
    name: String,
    checksum: Option<String>,
    active: bool,
    // Some(false) is persistent enablement, Some(true) is runtime-only.
    enabled: Option<bool>,
}

fn path(name: &str) -> Utf8PathBuf {
    Utf8Path::new(SYSTEM_DIRECTORY).join(name)
}

fn require_root() -> Result<(), PlatformError> {
    if getuid().is_root() && geteuid().is_root() {
        Ok(())
    } else {
        Err(PlatformError::PrivilegedHelperAuthentication(
            "socket proxy operations require root".to_owned(),
        ))
    }
}

fn validate_ports(config: &PfRedirectConfig) -> Result<(), PlatformError> {
    validate_high_port("Gateway HTTP", config.http_port)?;
    validate_high_port("Gateway HTTPS", config.https_port)?;
    if config.http_port == config.https_port {
        return Err(PlatformError::PrivilegedHelperRejected {
            message: "Gateway HTTP and HTTPS ports must be different".to_owned(),
        });
    }
    Ok(())
}

pub(in crate::pf) fn apply(config: &PfRedirectConfig) -> Result<(), PlatformError> {
    require_root()?;
    validate_ports(config)?;
    let _lock = mutation_lock()?;
    futures_lite::future::block_on(async {
        let (manager, previous) = preflight().await?;
        let mut enablement_attempted = false;
        let result = within(APPLY_TIMEOUT, async {
            stop_units(&manager).await?;
            let inspection = crate::low_port::inspect_loopback_ports(&[80, 443])?;
            let conflicts = inspection.ports.iter().filter(|port| !port.available)
                .map(crate::LowPortState::conflict_message).collect::<Vec<_>>();
            if !conflicts.is_empty() {
                return Err(PlatformError::SystemIntegration(conflicts.join(" ")));
            }
            backup_files(Utf8Path::new(SYSTEM_DIRECTORY), &previous)?;
            write_units(Utf8Path::new(SYSTEM_DIRECTORY), config)?;
            reload(&manager).await?;
            require_owned_units(&manager, &previous, true).await?;
            enablement_attempted = true;
            for unit in &previous {
                enable(&manager, &unit.name, false).await?;
            }
            for unit in &previous {
                job(&manager, "RestartUnit", &unit.name).await?;
            }
            let http = observe(&manager, &HTTP).await?;
            let https = observe(&manager, &HTTPS).await?;
            let actual = super::complete_inspection(http, https, Utf8Path::new(SYSTEM_DIRECTORY))?;
            // The completed restart jobs establish fresh binds here. Read-only
            // diagnostics cannot make that inference from cached unit properties.
            if actual.pf_enabled && actual.pv_config.as_ref() == Some(config) {
                Ok(())
            } else {
                Err(PlatformError::SystemIntegration(
                    "socket proxy units did not match after restart".to_owned(),
                ))
            }
        }).await.unwrap_or_else(|| Err(timeout("apply")));
        if let Err(original) = result {
            let restored = within(ROLLBACK_TIMEOUT, rollback(&manager, &previous, enablement_attempted))
                .await.unwrap_or_else(|| Err(timeout("rollback")));
            return Err(PlatformError::SystemIntegration(match restored {
                Ok(()) => format!("{original}; restored the previous PV unit files, enablement and activity"),
                Err(error) => format!("{original}; rollback also failed: {error}; administrator recovery is required; inspect any retained transaction files"),
            }));
        }
        clear_backups(Utf8Path::new(SYSTEM_DIRECTORY), &previous)
    }).map_err(|source| PlatformError::PortProxyMutation { operation: "apply", source: Box::new(source) })
}

pub(in crate::pf) fn remove() -> Result<(), PlatformError> {
    require_root()?;
    let _lock = mutation_lock()?;
    futures_lite::future::block_on(async {
        let (manager, previous) = preflight().await?;
        within(APPLY_TIMEOUT, async {
            stop_units(&manager).await?;
            for unit in &previous {
                disable(&manager, &unit.name, unit.checksum.is_none()).await?;
            }
            require_owned_units(&manager, &previous, false).await?;
            for unit in &previous {
                if unit.checksum.is_some() {
                    match manager
                        .call::<_, _, ()>("ResetFailedUnit", &(unit.name.as_str(),))
                        .await
                    {
                        Ok(()) => {}
                        Err(zbus::Error::MethodError(name, _, _))
                            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" => {}
                        Err(error) => return Err(bus_error(error)),
                    }
                }
            }
            for unit in &previous {
                read_owned_unit(&path(&unit.name))?;
                remove_file(&path(&unit.name))?;
            }
            reload(&manager).await?;
            for name in unit_names() {
                if read_owned_unit(&path(&name))?.is_some()
                    || read_loaded_unit(&manager, &name).await?.0.status.is_some()
                {
                    return Err(PlatformError::SystemIntegration(format!(
                        "socket proxy unit remains after removal: {name}"
                    )));
                }
            }
            Ok(())
        })
        .await
        .unwrap_or_else(|| Err(timeout("remove")))
    })
    .map_err(|source| PlatformError::PortProxyMutation {
        operation: "remove",
        source: Box::new(source),
    })
}

fn mutation_lock() -> Result<RootFile, PlatformError> {
    root_files::validate_directory_chain(Utf8Path::new("/usr/local/libexec"), false)?;
    root_files::validate_directory_chain(Utf8Path::new(SYSTEM_DIRECTORY), false)?;
    root_files::acquire_lock(Utf8Path::new(root_files::LOCK_PATH))
}

async fn preflight() -> Result<(Proxy<'static>, Vec<PreviousUnit>), PlatformError> {
    within(PREFLIGHT_TIMEOUT, async {
        let connection = connection::Builder::address("unix:path=/run/dbus/system_bus_socket")
            .map_err(bus_error)?
            .build()
            .await
            .map_err(bus_error)?;
        let manager = system_manager_proxy(&connection).await.map_err(bus_error)?;
        let mut previous = capture_files(Utf8Path::new(SYSTEM_DIRECTORY))?;
        for unit in &mut previous {
            let status = read_loaded_unit(&manager, &unit.name).await?.0.status;
            require_owned_status(&unit.name, unit.checksum.is_some(), status.as_ref())?;
            unit.active = status.as_ref().is_some_and(|status| {
                matches!(
                    status.active_state.as_str(),
                    "active" | "activating" | "reloading"
                )
            });
            if unit.checksum.is_some() {
                unit.enabled = enablement(&manager, &unit.name).await?;
            }
        }
        Ok((manager, previous))
    })
    .await
    .ok_or_else(|| timeout("preflight"))?
}

fn capture_files(directory: &Utf8Path) -> Result<Vec<PreviousUnit>, PlatformError> {
    root_files::validate_directory_chain(directory, false)?;
    unit_names().into_iter().map(|name| {
        let path = directory.join(&name);
        require_no_candidate(&path)?;
        let backup = Utf8PathBuf::from(format!("{path}.rollback"));
        if root_files::read_root_file(&backup, 0o644)?.is_some() {
            return Err(PlatformError::SystemIntegration(format!(
                "retained unit rollback requires administrator inspection before retrying: {backup}"
            )));
        }
        Ok(PreviousUnit {
            name,
            checksum: read_owned_unit(&path)?.as_deref().map(root_files::checksum_bytes),
            active: false,
            enabled: None,
        })
    }).collect()
}

async fn require_owned_units(
    manager: &Proxy<'_>,
    previous: &[PreviousUnit],
    installed: bool,
) -> Result<(), PlatformError> {
    for unit in previous {
        let has_file = if installed {
            read_owned_unit(&path(&unit.name))?.is_some()
        } else {
            owned_during_rollback(Utf8Path::new(SYSTEM_DIRECTORY), unit)?
        };
        if installed && !has_file {
            return Err(PlatformError::SystemIntegration(format!(
                "installed unit is missing: {}",
                unit.name
            )));
        }
        let status = read_loaded_unit(manager, &unit.name).await?.0.status;
        require_owned_status(&unit.name, has_file, status.as_ref())?;
    }
    Ok(())
}

fn owned_during_rollback(directory: &Utf8Path, unit: &PreviousUnit) -> Result<bool, PlatformError> {
    if read_owned_unit(&directory.join(&unit.name))?.is_some() {
        return Ok(true);
    }
    let backup = directory.join(format!("{}.rollback", unit.name));
    Ok(read_owned_unit(&backup)?
        .as_deref()
        .map(root_files::checksum_bytes)
        .is_some_and(|checksum| Some(checksum.as_str()) == unit.checksum.as_deref()))
}

fn require_owned_status(
    name: &str,
    has_file: bool,
    status: Option<&UnitStatus>,
) -> Result<(), PlatformError> {
    if let Some(status) = status
        && (!has_file || status.fragment_path != path(name) || !status.drop_in_paths.is_empty())
    {
        return Err(PlatformError::SystemIntegration(format!(
            "refusing socket proxy unit {name}: loaded ownership could not be verified; inspect the unit and its overrides with an administrator"
        )));
    }
    Ok(())
}

fn backup_files(directory: &Utf8Path, previous: &[PreviousUnit]) -> Result<(), PlatformError> {
    for unit in previous {
        if let Some(checksum) = &unit.checksum {
            let path = directory.join(&unit.name);
            let actual = read_owned_unit(&path)?
                .as_deref()
                .map(root_files::checksum_bytes);
            if actual.as_ref() != Some(checksum) {
                return Err(PlatformError::SystemIntegration(format!(
                    "unit changed before backup: {path}"
                )));
            }
            root_files::rename(&path, &Utf8PathBuf::from(format!("{path}.rollback")))?;
        }
    }
    Ok(())
}

fn write_units(directory: &Utf8Path, config: &PfRedirectConfig) -> Result<(), PlatformError> {
    for (name, content) in super::render_units(config) {
        write_owned_unit(&directory.join(name), &content)?;
    }
    Ok(())
}

async fn rollback(
    manager: &Proxy<'_>,
    previous: &[PreviousUnit],
    enablement_attempted: bool,
) -> Result<(), PlatformError> {
    let mut errors = Vec::new();
    let owned = match require_owned_units(manager, previous, false).await {
        Ok(()) => true,
        Err(error) => {
            errors.push(error.to_string());
            false
        }
    };
    if owned {
        for name in stop_order() {
            if let Err(error) = job(manager, "StopUnit", &name).await {
                errors.push(error.to_string());
            }
        }
        if enablement_attempted {
            for unit in previous {
                if let Err(error) = disable(manager, &unit.name, false).await {
                    errors.push(error.to_string());
                }
            }
        }
    }
    if let Err(error) = restore_files(Utf8Path::new(SYSTEM_DIRECTORY), previous) {
        errors.push(error.to_string());
    }
    if !errors.is_empty() {
        return Err(PlatformError::SystemIntegration(errors.join("; ")));
    }
    reload(manager).await?;
    require_owned_units(manager, previous, false).await?;
    for unit in previous {
        if let Some(runtime) = unit.enabled
            && let Err(error) = enable(manager, &unit.name, runtime).await
        {
            errors.push(error.to_string());
        }
        if unit.active
            && let Err(error) = job(manager, "RestartUnit", &unit.name).await
        {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty() {
        for unit in previous {
            let status = read_loaded_unit(manager, &unit.name).await?.0.status;
            require_owned_status(&unit.name, unit.checksum.is_some(), status.as_ref())?;
            if (unit.active
                && !status
                    .as_ref()
                    .is_some_and(|status| status.active_state == "active"))
                || (unit.checksum.is_some()
                    && enablement(manager, &unit.name).await? != unit.enabled)
            {
                return Err(PlatformError::SystemIntegration(format!(
                    "unit did not match after rollback: {}",
                    unit.name
                )));
            }
        }
        clear_backups(Utf8Path::new(SYSTEM_DIRECTORY), previous)
    } else {
        Err(PlatformError::SystemIntegration(errors.join("; ")))
    }
}

fn restore_files(directory: &Utf8Path, previous: &[PreviousUnit]) -> Result<(), PlatformError> {
    let mut errors = Vec::new();
    for unit in previous {
        let path = directory.join(&unit.name);
        let result = (|| {
            read_owned_unit(&path)?;
            root_files::restore_file(&path, 0o644, unit.checksum.as_deref())?;
            if read_owned_unit(&path)?
                .as_deref()
                .map(root_files::checksum_bytes)
                != unit.checksum
            {
                return Err(PlatformError::SystemIntegration(format!(
                    "unit bytes did not match after rollback: {path}"
                )));
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::SystemIntegration(errors.join("; ")))
    }
}

fn clear_backups(directory: &Utf8Path, previous: &[PreviousUnit]) -> Result<(), PlatformError> {
    for unit in previous {
        let path = directory.join(&unit.name);
        require_no_candidate(&path)?;
        let backup = Utf8PathBuf::from(format!("{path}.rollback"));
        if let Some(content) = root_files::read_root_file(&backup, 0o644)? {
            if unit.checksum.as_deref() != Some(root_files::checksum_bytes(&content).as_str()) {
                return Err(PlatformError::SystemIntegration(format!(
                    "unit rollback changed before cleanup: {backup}"
                )));
            }
            remove_file(&backup)?;
        }
    }
    Ok(())
}

fn remove_file(path: &Utf8Path) -> Result<(), PlatformError> {
    state::fs::remove_file_if_exists(path)
        .and_then(|()| state::fs::sync_parent_directory(path))
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))
}

fn stop_order() -> [String; 4] {
    [
        HTTP.service_name(),
        HTTPS.service_name(),
        HTTP.socket_name(),
        HTTPS.socket_name(),
    ]
}

async fn stop_units(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    for name in stop_order() {
        job(manager, "StopUnit", &name).await?;
    }
    Ok(())
}

async fn reload(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    manager
        .call::<_, _, ()>("Reload", &())
        .await
        .map_err(bus_error)
}

async fn enable(manager: &Proxy<'_>, name: &str, runtime: bool) -> Result<(), PlatformError> {
    manager
        .call::<_, _, (bool, Vec<(String, String, String)>)>(
            "EnableUnitFiles",
            &([name].as_slice(), runtime, false),
        )
        .await
        .map_err(bus_error)?;
    Ok(())
}

async fn disable(manager: &Proxy<'_>, name: &str, missing: bool) -> Result<(), PlatformError> {
    for runtime in [false, true] {
        match manager
            .call::<_, _, Vec<(String, String, String)>>(
                "DisableUnitFiles",
                &([name].as_slice(), runtime),
            )
            .await
        {
            Ok(_) => {}
            Err(zbus::Error::MethodError(error, _, _))
                if missing && error.as_str() == "org.freedesktop.systemd1.NoSuchUnit" => {}
            Err(error) => return Err(bus_error(error)),
        }
    }
    Ok(())
}

async fn enablement(manager: &Proxy<'_>, name: &str) -> Result<Option<bool>, PlatformError> {
    let state: String = manager
        .call("GetUnitFileState", &(name,))
        .await
        .map_err(bus_error)?;
    match state.as_str() {
        "enabled" => Ok(Some(false)),
        "enabled-runtime" => Ok(Some(true)),
        "disabled" | "static" => Ok(None),
        _ => Err(PlatformError::SystemIntegration(format!(
            "unexpected socket proxy enablement for {name}: {state}"
        ))),
    }
}

async fn job(manager: &Proxy<'_>, operation: &str, name: &str) -> Result<(), PlatformError> {
    let result = within(Duration::from_secs(15), unit_job(manager, operation, name))
        .await
        .ok_or_else(|| {
            PlatformError::SystemIntegration(format!("systemd {operation} {name} timed out"))
        })?
        .map_err(bus_error)?;
    if result == "done" {
        Ok(())
    } else {
        Err(PlatformError::SystemIntegration(format!(
            "systemd {operation} {name} finished with {result}"
        )))
    }
}

fn timeout(operation: &str) -> PlatformError {
    PlatformError::SystemIntegration(format!("socket proxy {operation} timed out"))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use camino::Utf8Path;
    use camino_tempfile::tempdir_in;
    use rustix::process::{geteuid, getuid};

    use super::owned_during_rollback;
    use super::{
        HTTP, HTTPS, PreviousUnit, UnitStatus, apply, backup_files, capture_files, clear_backups,
        path, remove, remove_file, require_owned_status, require_root, restore_files, root_files,
        unit_names, validate_ports, write_units,
    };
    use crate::systemd::read_owned_unit;
    use crate::{PfRedirectConfig, PlatformError};

    #[test]
    fn mutations_require_root_and_only_distinct_high_backend_ports() {
        for config in [
            PfRedirectConfig::new(80, 48443),
            PfRedirectConfig::new(48080, 443),
            PfRedirectConfig::new(48080, 48080),
        ] {
            assert!(matches!(
                validate_ports(&config),
                Err(PlatformError::PrivilegedHelperRejected { .. })
            ));
        }
        assert!(validate_ports(&PfRedirectConfig::new(1024, 1025)).is_ok());
        if !getuid().is_root() || !geteuid().is_root() {
            assert!(matches!(
                apply(&PfRedirectConfig::new(48080, 48443)),
                Err(PlatformError::PrivilegedHelperAuthentication(_))
            ));
            assert!(matches!(
                remove(),
                Err(PlatformError::PrivilegedHelperAuthentication(_))
            ));
        }
    }

    #[test]
    fn loaded_units_require_the_marked_fixed_file_without_overrides() {
        let name = HTTP.socket_name();
        let status = UnitStatus {
            load_state: "loaded".to_owned(),
            needs_reload: false,
            active_state: "active".to_owned(),
            fragment_path: path(&name),
            drop_in_paths: Vec::new(),
        };
        assert!(require_owned_status(&name, false, None).is_ok());
        assert!(require_owned_status(&name, true, Some(&status)).is_ok());
        let mut foreign_fragment = status.clone();
        foreign_fragment.fragment_path = "/run/systemd/system/pv-http.socket".into();
        let mut overridden = status.clone();
        overridden
            .drop_in_paths
            .push("/etc/systemd/system/pv-http.socket.d/foreign.conf".to_owned());
        for (has_file, status) in [
            (false, &status),
            (true, &foreign_fragment),
            (true, &overridden),
        ] {
            assert!(matches!(
                require_owned_status(&name, has_file, Some(status)),
                Err(PlatformError::SystemIntegration(_))
            ));
        }
    }

    fn contents(directory: &Utf8Path) -> Result<Vec<Option<Vec<u8>>>> {
        unit_names()
            .iter()
            .map(|name| Ok(root_files::read_root_file(&directory.join(name), 0o644)?))
            .collect()
    }

    fn require_restored(directory: &Utf8Path, previous: &[PreviousUnit]) -> Result<()> {
        for unit in previous {
            let path = directory.join(&unit.name);
            assert_eq!(
                read_owned_unit(&path)?
                    .as_deref()
                    .map(root_files::checksum_bytes),
                unit.checksum
            );
            assert!(
                root_files::read_root_file(
                    &directory.join(format!("{}.rollback", unit.name)),
                    0o644
                )?
                .is_none()
            );
        }
        Ok(())
    }

    #[test]
    #[ignore = "run as root in an ordinary disposable Linux container; uses only temporary files"]
    fn root_unit_transactions_restore_four_files_and_preserve_foreign_state() -> Result<()> {
        require_root()?;
        let directory = tempdir_in("/run")?;
        let original = PfRedirectConfig::new(48080, 48443);
        let replacement = PfRedirectConfig::new(45000, 45001);
        let empty = capture_files(directory.path())?;
        write_units(directory.path(), &original)?;
        assert!(contents(directory.path())?.iter().all(Option::is_some));
        assert_eq!(
            read_owned_unit(&directory.path().join(HTTP.service_name()))?,
            Some(HTTP.render_service(original.http_port).into_bytes())
        );
        assert_eq!(
            read_owned_unit(&directory.path().join(HTTPS.service_name()))?,
            Some(HTTPS.render_service(original.https_port).into_bytes())
        );
        restore_files(directory.path(), &empty)?;
        require_restored(directory.path(), &empty)?;

        write_units(directory.path(), &original)?;
        let before = contents(directory.path())?;
        let mut previous = capture_files(directory.path())?;
        backup_files(directory.path(), &previous)?;
        write_units(directory.path(), &replacement)?;
        assert_ne!(contents(directory.path())?, before);
        restore_files(directory.path(), &previous)?;
        require_restored(directory.path(), &previous)?;
        assert_eq!(contents(directory.path())?, before);
        clear_backups(directory.path(), &previous)?;

        let first = directory.path().join(&previous[0].name);
        let first_backup = directory
            .path()
            .join(format!("{}.rollback", previous[0].name));
        root_files::rename(&first, &first_backup)?;
        assert!(owned_during_rollback(directory.path(), &previous[0])?);
        let checksum = previous[0].checksum.replace("wrong checksum".to_owned());
        assert!(!owned_during_rollback(directory.path(), &previous[0])?);
        previous[0].checksum = checksum;
        restore_files(directory.path(), &previous)?;
        require_restored(directory.path(), &previous)?;
        assert_eq!(contents(directory.path())?, before);

        for suffix in ["candidate", "rollback"] {
            let retained = directory
                .path()
                .join(format!("{}.{suffix}", previous[2].name));
            root_files::write_root_file(
                &retained,
                b"retained administrator recovery bytes",
                0o644,
            )?;
            assert!(capture_files(directory.path()).is_err());
            assert_eq!(contents(directory.path())?, before);
            assert_eq!(
                root_files::read_root_file(&retained, 0o644)?,
                Some(b"retained administrator recovery bytes".to_vec())
            );
            remove_file(&retained)?;
        }

        let foreign = directory.path().join(&previous[0].name);
        let original_first = read_owned_unit(&foreign)?.context("original fixture unit missing")?;
        remove_file(&foreign)?;
        root_files::write_root_file(&foreign, b"administrator unit", 0o644)?;
        let foreign_state = contents(directory.path())?;
        assert!(matches!(
            capture_files(directory.path()),
            Err(PlatformError::SystemIntegration(_))
        ));
        assert_eq!(contents(directory.path())?, foreign_state);
        remove_file(&foreign)?;
        root_files::write_root_file(&foreign, &original_first, 0o644)?;

        backup_files(directory.path(), &previous)?;
        write_units(directory.path(), &replacement)?;
        remove_file(&foreign)?;
        root_files::write_root_file(&foreign, b"administrator unit", 0o644)?;
        assert!(matches!(
            owned_during_rollback(directory.path(), &previous[0]),
            Err(PlatformError::SystemIntegration(_))
        ));
        assert!(matches!(
            restore_files(directory.path(), &previous),
            Err(PlatformError::SystemIntegration(_))
        ));
        assert_eq!(
            root_files::read_root_file(&foreign, 0o644)?,
            Some(b"administrator unit".to_vec())
        );
        assert_eq!(read_owned_unit(&first_backup)?, Some(original_first));
        for unit in previous.iter().skip(1) {
            assert_eq!(
                read_owned_unit(&directory.path().join(&unit.name))?
                    .as_deref()
                    .map(root_files::checksum_bytes),
                unit.checksum
            );
        }
        Ok(())
    }
}
