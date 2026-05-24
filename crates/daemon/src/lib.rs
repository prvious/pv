mod error;
mod jobs;
mod protocol;
mod server;

use std::future::Future;
use std::io;
use std::io::ErrorKind;

use state::{Database, PvPaths};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use error::DaemonError;
pub use protocol::PROTOCOL_VERSION;

#[derive(Debug)]
pub struct RunningDaemon {
    paths: PvPaths,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), DaemonError>>,
}

impl RunningDaemon {
    pub async fn start(paths: PvPaths) -> Result<Self, DaemonError> {
        state::fs::ensure_layout(&paths)?;
        Database::open(&paths)?;
        prepare_socket_path(&paths).await?;
        let listener = UnixListener::bind(paths.daemon_socket())?;
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let server_paths = paths.clone();
        let task = tokio::spawn(server::serve(server_paths, listener, shutdown_receiver));

        Ok(Self {
            paths,
            shutdown,
            task,
        })
    }

    pub async fn shutdown(self) -> Result<(), DaemonError> {
        let _ = self.shutdown.send(());
        let task_result = self.task.await?;
        let socket_result = state::fs::remove_daemon_socket(&self.paths);

        task_result?;
        socket_result?;

        Ok(())
    }
}

async fn prepare_socket_path(paths: &PvPaths) -> Result<(), DaemonError> {
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
            state::fs::remove_daemon_socket(paths)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub fn run_blocking(paths: PvPaths) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;

    runtime.block_on(async {
        let daemon = RunningDaemon::start(paths).await?;
        wait_for_shutdown(daemon, termination_signal()).await
    })
}

async fn wait_for_shutdown(
    daemon: RunningDaemon,
    shutdown_signal: impl Future<Output = io::Result<()>>,
) -> Result<(), DaemonError> {
    let RunningDaemon {
        paths,
        shutdown,
        mut task,
    } = daemon;
    tokio::pin!(shutdown_signal);

    tokio::select! {
        signal_result = &mut shutdown_signal => {
            signal_result?;
            let _ = shutdown.send(());
            let task_result = task.await?;
            let socket_result = state::fs::remove_daemon_socket(&paths);

            task_result?;
            socket_result?;

            Ok(())
        }
        task_result = &mut task => {
            let socket_result = state::fs::remove_daemon_socket(&paths);
            let task_result = task_result?;

            socket_result?;
            task_result
        }
    }
}

async fn termination_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(test)]
mod tests {
    use std::{future, io};

    use state::PvPaths;
    use tokio::sync::oneshot;

    use super::{DaemonError, RunningDaemon, wait_for_shutdown};

    #[tokio::test]
    async fn shutdown_wait_returns_when_server_task_fails_before_signal() {
        let paths = PvPaths::for_home("/tmp/pv-daemon-test-home");
        let (shutdown, _shutdown_receiver) = oneshot::channel();
        let task =
            tokio::spawn(async { Err(DaemonError::Io(io::Error::other("server stopped early"))) });
        let daemon = RunningDaemon {
            paths,
            shutdown,
            task,
        };

        let result = wait_for_shutdown(daemon, future::pending::<io::Result<()>>()).await;

        assert!(matches!(
            result,
            Err(DaemonError::Io(error)) if error.to_string() == "server stopped early"
        ));
    }
}
