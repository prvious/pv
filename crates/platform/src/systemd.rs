use std::future::Future;
use std::io::{self, Read};
use std::pin::pin;
use std::time::Duration;

use async_io::Timer;
use camino::{Utf8Path, Utf8PathBuf};
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use rustix::fs::{Mode, OFlags, open};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy, proxy};

use crate::PlatformError;
use crate::helper::linux as root_files;

const PV_MARKER: &str = "# Managed by PV";

// Distribution-wide service defaults are trusted OS policy, not PV overrides.
pub(crate) fn is_vendor_service_drop_in(path: &str, unit_name: &str, scope: &str) -> bool {
    let path = Utf8Path::new(path);
    unit_name.ends_with(".service")
        && path.extension() == Some("conf")
        && matches!(
            (scope, path.parent().map(Utf8Path::as_str)),
            (
                "system",
                Some("/usr/lib/systemd/system/service.d" | "/lib/systemd/system/service.d")
            ) | (
                "user",
                Some("/usr/lib/systemd/user/service.d" | "/lib/systemd/user/service.d")
            )
        )
}

#[expect(
    clippy::disallowed_types,
    reason = "system unit inspection owns a native NOFOLLOW/NONBLOCK-validated descriptor"
)]
type UnitFile = std::fs::File;

pub(crate) fn read_unit(path: &Utf8Path) -> io::Result<Option<String>> {
    let descriptor = match open(
        path.as_std_path(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut file = UnitFile::from(descriptor);
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "system unit is not a regular file",
        ));
    }
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(Some(content))
}

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
    if root_files::read_root_file(&candidate, 0o644)?.is_some() {
        return Err(PlatformError::SystemIntegration(format!(
            "retained unit candidate requires administrator inspection before retrying: {candidate}"
        )));
    }
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
    if read_owned_unit(path)?.as_deref() != Some(content.as_bytes()) {
        return Err(PlatformError::SystemIntegration(format!(
            "systemd unit bytes did not match after write: {path}"
        )));
    }
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

pub(crate) async fn within<T>(timeout: Duration, future: impl Future<Output = T>) -> Option<T> {
    match select(pin!(future), Timer::after(timeout)).await {
        Either::Left((result, _)) => Some(result),
        Either::Right(_) => None,
    }
}

pub(crate) async fn system_manager_proxy(connection: &Connection) -> zbus::Result<Proxy<'static>> {
    let manager = native_proxy(
        connection,
        "/org/freedesktop/systemd1".try_into()?,
        "org.freedesktop.systemd1.Manager",
    )
    .await?;
    manager.get_property::<String>("Version").await?;
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

#[cfg(test)]
mod tests {
    use super::is_vendor_service_drop_in;

    #[test]
    fn vendor_service_defaults_require_the_exact_directory_scope_and_unit_type() {
        for (scope, directory) in [
            ("system", "/usr/lib/systemd/system/service.d"),
            ("system", "/lib/systemd/system/service.d"),
            ("user", "/usr/lib/systemd/user/service.d"),
            ("user", "/lib/systemd/user/service.d"),
        ] {
            let path = format!("{directory}/10-timeout-abort.conf");
            assert!(is_vendor_service_drop_in(&path, "pv-dns.service", scope));
            assert!(!is_vendor_service_drop_in(&path, "pv-http.socket", scope));
        }
        for path in [
            "/etc/systemd/system/service.d/override.conf",
            "/run/systemd/system/service.d/override.conf",
            "/usr/local/lib/systemd/system/service.d/override.conf",
            "/usr/lib/systemd/system/pv-dns.service.d/override.conf",
            "/usr/lib/systemd/system/pv-.service.d/override.conf",
            "/usr/lib/systemd/system/socket.d/override.conf",
            "/usr/lib/systemd/system/service.d/nested/override.conf",
            "/usr/lib/systemd/system/service.d/not-a-conf",
            "/home/dev/.config/systemd/user/service.d/override.conf",
            "/usr/lib/systemd/user/service.d/10-timeout-abort.conf",
        ] {
            assert!(
                !is_vendor_service_drop_in(path, "pv-dns.service", "system"),
                "{path}"
            );
        }
        assert!(!is_vendor_service_drop_in(
            "/usr/lib/systemd/system/service.d/10-timeout-abort.conf",
            "pv-daemon.service",
            "user"
        ));
    }
}
