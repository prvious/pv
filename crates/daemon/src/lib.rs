mod error;
mod ipc;
mod jobs;
mod protocol;
mod reconciliation;
mod server;
mod supervisor;

use std::future::Future;
use std::io;

use state::{Database, PvPaths};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use error::DaemonError;
pub use protocol::PROTOCOL_VERSION;
pub use reconciliation::{
    EnqueueResult, QueuedReconciliation, ReconciliationDebouncer, ReconciliationQueue,
    ReconciliationScope, RunningReconciliation,
};
pub use supervisor::{
    ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_custom_readiness, wait_for_readiness,
};

#[derive(Debug)]
pub struct RunningDaemon {
    paths: PvPaths,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), DaemonError>>,
}

impl RunningDaemon {
    pub async fn start(paths: PvPaths) -> Result<Self, DaemonError> {
        Database::open(&paths)?;
        ipc::prepare_endpoint(&paths).await?;
        let listener = ipc::bind(&paths)?;
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
        let join_result = self.task.await;
        let socket_result = ipc::remove_endpoint(&self.paths);

        socket_result?;
        let task_result = join_result?;
        task_result?;

        Ok(())
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
            let join_result = task.await;
            let socket_result = ipc::remove_endpoint(&paths);

            socket_result?;
            let task_result = join_result?;
            task_result?;

            Ok(())
        }
        task_result = &mut task => {
            let socket_result = ipc::remove_endpoint(&paths);
            socket_result?;
            task_result?
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

    use camino_tempfile::tempdir;
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

    #[tokio::test]
    async fn shutdown_removes_socket_when_server_task_is_cancelled() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths)?;
        let stale_listener = tokio::net::UnixListener::bind(paths.daemon_socket())?;
        drop(stale_listener);
        let (shutdown, _shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(future::pending::<Result<(), DaemonError>>());
        task.abort();
        let daemon = RunningDaemon {
            paths: paths.clone(),
            shutdown,
            task,
        };

        let result = daemon.shutdown().await;

        assert!(matches!(result, Err(DaemonError::Task(error)) if error.is_cancelled()));
        assert!(!paths.daemon_socket().exists());

        Ok(())
    }
}
