use std::io::ErrorKind;

use state::PvPaths;
use tokio::net::{UnixListener, UnixStream};

use crate::DaemonError;

pub(crate) type LocalListener = UnixListener;
pub(crate) type LocalStream = UnixStream;

pub(crate) fn bind(paths: &PvPaths) -> Result<LocalListener, DaemonError> {
    Ok(UnixListener::bind(paths.daemon_socket())?)
}

pub(crate) async fn connect(paths: &PvPaths) -> Result<LocalStream, DaemonError> {
    Ok(UnixStream::connect(paths.daemon_socket()).await?)
}

pub(crate) async fn prepare_endpoint(paths: &PvPaths) -> Result<(), DaemonError> {
    match UnixStream::connect(paths.daemon_socket()).await {
        Ok(_stream) => Err(DaemonError::SocketInUse {
            path: paths.daemon_socket().to_string(),
        }),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            remove_endpoint(paths)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn remove_endpoint(paths: &PvPaths) -> Result<(), DaemonError> {
    Ok(state::fs::remove_daemon_socket(paths)?)
}
