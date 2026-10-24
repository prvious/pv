use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use async_io::Timer;
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
