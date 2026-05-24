mod error;
mod jobs;
mod protocol;
mod server;

use state::{Database, PvPaths};
use tokio::net::UnixListener;
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

pub fn run_blocking(paths: PvPaths) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;

    runtime.block_on(async {
        let daemon = RunningDaemon::start(paths).await?;
        tokio::signal::ctrl_c().await?;
        daemon.shutdown().await
    })
}
