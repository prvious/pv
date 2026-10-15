use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use async_io::Timer;
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy, proxy};

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
