use std::future::Future;

use camino::{Utf8Path, Utf8PathBuf};
use futures_util::StreamExt;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy, connection, proxy};

use crate::clock::{self, Limit};

use crate::PlatformError;
use crate::helper::linux as root_files;

const PV_MARKER: &str = "# Managed by PV";

pub(crate) fn read_owned_unit(path: &Utf8Path) -> Result<Option<Vec<u8>>, PlatformError> {
    let content = root_files::read_root_file(path, 0o644)?;
    if let Some(content) = &content
        && !std::str::from_utf8(content)
            .ok()
            .is_some_and(|content| content.lines().any(|line| line.trim() == PV_MARKER))
    {
        return Err(PlatformError::SystemIntegration(format!(
            "systemd unit is not PV-owned: {path}; inspect it with an administrator before retrying"
        )));
    }
    Ok(content)
}

pub(crate) fn require_no_candidate(path: &Utf8Path) -> Result<(), PlatformError> {
    let candidate = Utf8PathBuf::from(format!("{path}.candidate"));
    state::fs::remove_file_if_exists(&candidate)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    Ok(())
}

pub(crate) fn write_owned_unit(path: &Utf8Path, content: &str) -> Result<(), PlatformError> {
    let parent = path.parent().ok_or_else(|| {
        PlatformError::SystemIntegration(format!("systemd unit has no parent directory: {path}"))
    })?;
    root_files::validate_directory_chain(parent, false)?;
    read_owned_unit(path)?;
    require_no_candidate(path)?;
    let candidate = Utf8PathBuf::from(format!("{path}.candidate"));
    root_files::write_root_file(&candidate, content.as_bytes(), 0o644)?;
    root_files::rename(&candidate, path)?;
    Ok(())
}

pub(crate) async fn native_proxy(
    connection: &Connection,
    path: OwnedObjectPath,
    interface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    proxy::Builder::new(connection)
        .destination("org.freedesktop.systemd1")?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

pub(crate) async fn system_manager_proxy(connection: &Connection) -> zbus::Result<Proxy<'static>> {
    let manager = native_proxy(
        connection,
        "/org/freedesktop/systemd1".try_into()?,
        "org.freedesktop.systemd1.Manager",
    )
    .await?;
    manager.call::<_, _, ()>("Subscribe", &()).await?;
    Ok(manager)
}

pub(crate) async fn unit_job(
    manager: &Proxy<'_>,
    operation: &str,
    unit: &str,
) -> zbus::Result<String> {
    let mut removed = manager.receive_signal("JobRemoved").await?;
    let job: OwnedObjectPath = match manager.call(operation, &(unit, "replace")).await {
        Ok(job) => job,
        Err(zbus::Error::MethodError(name, _, _))
            if operation == "StopUnit"
                && name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
        {
            return Ok("done".to_owned());
        }
        Err(error) => return Err(error),
    };
    while let Some(message) = removed.next().await {
        let (_, completed, _, result): (u32, OwnedObjectPath, String, String) =
            message.body().deserialize()?;
        if completed == job {
            return Ok(result);
        }
    }
    Err(zbus::Error::Failure(
        "systemd disconnected before job completion".to_owned(),
    ))
}
pub(crate) fn is_method_error(error: &zbus::Error, expected: &str) -> bool {
    matches!(error, zbus::Error::MethodError(name, _, _) if name.as_str() == expected)
}

pub(crate) async fn within<T>(limit: &Limit, future: impl Future<Output = T>) -> Option<T> {
    clock::timeout(limit, future).await.ok()
}

pub(crate) async fn system_manager() -> zbus::Result<Proxy<'static>> {
    let connection = connection::Builder::address("unix:path=/run/dbus/system_bus_socket")?
        .build()
        .await?;
    system_manager_proxy(&connection).await
}
