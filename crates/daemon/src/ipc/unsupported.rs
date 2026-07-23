use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use platform::PlatformTarget;
use state::PvPaths;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

use crate::DaemonError;

pub(crate) struct LocalListener;

pub(crate) struct LocalStream(DuplexStream);

impl LocalListener {
    pub(crate) async fn accept(&self) -> io::Result<(LocalStream, ())> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "daemon IPC is unsupported on this platform",
        ))
    }
}

impl AsyncRead for LocalStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for LocalStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

pub(crate) fn bind(_paths: &PvPaths) -> Result<LocalListener, DaemonError> {
    unsupported()
}

pub(crate) async fn connect(_paths: &PvPaths) -> Result<LocalStream, DaemonError> {
    unsupported()
}

pub(crate) async fn prepare_endpoint(_paths: &PvPaths) -> Result<(), DaemonError> {
    unsupported()
}

pub(crate) fn remove_endpoint(_paths: &PvPaths) -> Result<(), DaemonError> {
    unsupported()
}

fn unsupported<ResultType>() -> Result<ResultType, DaemonError> {
    super::require_ipc_for(PlatformTarget::current()?)?;

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "daemon IPC is unsupported on this platform",
    )
    .into())
}
