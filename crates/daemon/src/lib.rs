mod client;
mod dns;
mod error;
pub mod gateway;
pub mod gateway_config;
mod ipc;
mod jobs;
mod managed_resources;
mod project_env;
mod reconciliation;
mod server;
mod structured_log;
mod supervisor;
mod watcher;

use std::future::Future;
use std::io;
use std::sync::Arc;

use managed_resources::ManagedResourceRuntimeCatalog;
use serde::Serialize;
use state::{Database, PvPaths, StateError};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use client::{
    CompletedJob, JobDownloadProgress, JobEventHandler, SubmittedJob, health_blocking,
    managed_resource_update_check_blocking, run_job_blocking, run_job_with_events_blocking,
    submit_job_blocking, wait_until_healthy_allowing_protocol_mismatch_blocking,
    wait_until_healthy_blocking,
};
pub use dns::{dns_port_available, response_bytes};
pub use error::DaemonError;
pub use protocol::PROTOCOL_VERSION;
pub use reconciliation::{
    EnqueueResult, QueuedReconciliation, ReconciliationDebouncer, ReconciliationJob,
    ReconciliationQueue, ReconciliationScope, ReconciliationScopeParseError, RunningReconciliation,
};
pub use supervisor::{
    AdoptedProcess, OwnedRuntime, ProcessSpec, ProcessSupervisor, ReadinessCheck,
    wait_for_custom_readiness, wait_for_readiness,
};

#[derive(Debug)]
pub struct RunningDaemon {
    paths: PvPaths,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), DaemonError>>,
    dns: dns::RunningDnsResolver,
}

impl RunningDaemon {
    pub async fn start(paths: PvPaths) -> Result<Self, DaemonError> {
        Self::start_with_runtime_catalog(paths, None).await
    }

    #[doc(hidden)]
    pub async fn start_without_managed_resource_adapters(
        paths: PvPaths,
    ) -> Result<Self, DaemonError> {
        Self::start_with_runtime_catalog(
            paths,
            Some(ManagedResourceRuntimeCatalog::without_adapters()?),
        )
        .await
    }

    #[doc(hidden)]
    pub async fn start_without_managed_resource_adapters_with_manifest_client(
        paths: PvPaths,
        manifest_url: impl Into<String>,
        client: impl resources::ResourceHttpClient + Send + Sync + 'static,
    ) -> Result<Self, DaemonError> {
        Self::start_with_runtime_catalog(
            paths,
            Some(
                ManagedResourceRuntimeCatalog::without_adapters_with_manifest_client(
                    manifest_url,
                    client,
                )?,
            ),
        )
        .await
    }

    async fn start_with_runtime_catalog(
        paths: PvPaths,
        runtime_catalog: Option<ManagedResourceRuntimeCatalog>,
    ) -> Result<Self, DaemonError> {
        match Self::start_with_runtime_catalog_inner(paths.clone(), runtime_catalog).await {
            Ok(daemon) => Ok(daemon),
            Err(error) => {
                write_startup_failure_marker(&paths, &error);

                Err(error)
            }
        }
    }

    async fn start_with_runtime_catalog_inner(
        paths: PvPaths,
        runtime_catalog: Option<ManagedResourceRuntimeCatalog>,
    ) -> Result<Self, DaemonError> {
        let mut database = Database::open(&paths)?;
        ipc::prepare_endpoint(&paths).await?;
        let listener = ipc::bind(&paths)?;
        if let Err(error) =
            database.fail_running_jobs("daemon was interrupted before job completed")
        {
            return Err(cleanup_startup_endpoint(&paths, error.into()));
        }
        if let Err(error) = clear_startup_failure_marker(&paths) {
            return Err(cleanup_startup_endpoint(&paths, error));
        }
        let dns = match dns::RunningDnsResolver::start(paths.clone()).await {
            Ok(dns) => dns,
            Err(error) => {
                return Err(cleanup_startup_endpoint(&paths, error));
            }
        };
        structured_log::daemon_started(&paths);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let server_paths = paths.clone();
        let runtime_catalog = runtime_catalog.map(Arc::new);
        let task = tokio::spawn(server::serve(
            server_paths,
            listener,
            shutdown_receiver,
            runtime_catalog,
        ));

        Ok(Self {
            paths,
            shutdown,
            task,
            dns,
        })
    }

    pub async fn shutdown(self) -> Result<(), DaemonError> {
        let _ = self.shutdown.send(());
        let join_result = self.task.await;
        let dns_result = self.dns.shutdown().await;
        let socket_result = ipc::remove_endpoint(&self.paths);

        socket_result?;
        dns_result?;
        let task_result = join_result?;
        task_result?;
        structured_log::daemon_stopped(&self.paths);

        Ok(())
    }
}

#[derive(Serialize)]
struct StartupFailureMarker {
    kind: &'static str,
    message: String,
}

fn clear_startup_failure_marker(paths: &PvPaths) -> Result<(), DaemonError> {
    state::fs::remove_file_if_exists(&paths.daemon_startup_error())?;

    Ok(())
}

fn write_startup_failure_marker(paths: &PvPaths, error: &DaemonError) {
    let marker = StartupFailureMarker {
        kind: startup_failure_kind(error),
        message: error.to_string(),
    };
    let Ok(json) = serde_json::to_string(&marker) else {
        return;
    };

    let _result = state::fs::write_sensitive_file(&paths.daemon_startup_error(), &json);
}

fn startup_failure_kind(error: &DaemonError) -> &'static str {
    match error {
        DaemonError::StartupCleanupFailed { source, .. } => startup_failure_kind(source),
        DaemonError::State(
            StateError::MigrationFailed { .. } | StateError::MigrationNameMismatch { .. },
        ) => "migration_failed",
        _ => "startup_failed",
    }
}

fn cleanup_startup_endpoint(paths: &PvPaths, startup_error: DaemonError) -> DaemonError {
    startup_error_after_endpoint_cleanup(startup_error, ipc::remove_endpoint(paths))
}

fn startup_error_after_endpoint_cleanup(
    startup_error: DaemonError,
    cleanup_result: Result<(), DaemonError>,
) -> DaemonError {
    match cleanup_result {
        Ok(()) => startup_error,
        Err(cleanup) => DaemonError::StartupCleanupFailed {
            source: Box::new(startup_error),
            cleanup: Box::new(cleanup),
        },
    }
}

pub fn run_blocking(paths: PvPaths) -> Result<(), DaemonError> {
    let runtime = build_runtime()?;

    runtime.block_on(async {
        let daemon = RunningDaemon::start(paths).await?;
        wait_for_shutdown(daemon, termination_signal()).await
    })
}

fn build_runtime() -> io::Result<Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
}

async fn wait_for_shutdown(
    daemon: RunningDaemon,
    shutdown_signal: impl Future<Output = io::Result<()>>,
) -> Result<(), DaemonError> {
    let RunningDaemon {
        paths,
        shutdown,
        mut task,
        mut dns,
    } = daemon;
    tokio::pin!(shutdown_signal);

    tokio::select! {
        signal_result = &mut shutdown_signal => {
            signal_result?;
            let _ = shutdown.send(());
            let join_result = task.await;
            let dns_result = dns.shutdown().await;
            let socket_result = ipc::remove_endpoint(&paths);

            socket_result?;
            dns_result?;
            let task_result = join_result?;
            task_result?;
            structured_log::daemon_stopped(&paths);

            Ok(())
        }
        task_result = &mut task => {
            let dns_result = dns.shutdown().await;
            let socket_result = ipc::remove_endpoint(&paths);
            socket_result?;
            dns_result?;
            let result = task_result?;
            if result.is_ok() {
                structured_log::daemon_stopped(&paths);
            }
            result
        }
        dns_result = dns.wait_for_completion() => {
            let _ = shutdown.send(());
            let join_result = task.await;
            let socket_result = ipc::remove_endpoint(&paths);

            socket_result?;
            let task_result = join_result?;
            task_result?;
            let result = dns_result;
            if result.is_ok() {
                structured_log::daemon_stopped(&paths);
            }
            result
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
    use std::{future, io, time::Duration};

    use camino_tempfile::tempdir;
    use state::PvPaths;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use super::{
        DaemonError, RunningDaemon, build_runtime, startup_error_after_endpoint_cleanup,
        wait_for_shutdown,
    };

    #[test]
    fn daemon_runtime_enables_tokio_timers() -> anyhow::Result<()> {
        let runtime = build_runtime()?;

        runtime.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        });

        Ok(())
    }

    #[test]
    fn startup_cleanup_failure_keeps_startup_error_primary() {
        let startup_error = DaemonError::DnsBind {
            protocol: "UDP",
            port: 5353,
            source: io::Error::new(io::ErrorKind::AddrInUse, "dns port is busy"),
        };
        let cleanup_error = DaemonError::Io(io::Error::other("socket cleanup failed"));

        let error = startup_error_after_endpoint_cleanup(startup_error, Err(cleanup_error));
        let message = error.to_string();

        assert!(matches!(
            error,
            DaemonError::StartupCleanupFailed { source, .. }
                if matches!(
                    *source,
                    DaemonError::DnsBind {
                        protocol: "UDP",
                        port: 5353,
                        ..
                    }
                )
        ));
        assert!(
            message.starts_with("DNS resolver failed to bind UDP on 127.0.0.1:5353"),
            "{message}"
        );
        assert!(message.contains("socket cleanup failed"), "{message}");
    }

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
            dns: super::dns::RunningDnsResolver::pending_for_test(),
        };

        let result = wait_for_shutdown(daemon, future::pending::<io::Result<()>>()).await;

        assert!(matches!(
            result,
            Err(DaemonError::Io(error)) if error.to_string() == "server stopped early"
        ));
    }

    #[tokio::test]
    async fn shutdown_wait_returns_when_dns_task_fails_before_signal() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths)?;
        let stale_listener = tokio::net::UnixListener::bind(paths.daemon_socket())?;
        drop(stale_listener);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async {
            let _ = shutdown_receiver.await;
            Ok(())
        });
        let daemon = RunningDaemon {
            paths: paths.clone(),
            shutdown,
            task,
            dns: super::dns::RunningDnsResolver::failed_for_test(io::Error::other(
                "dns stopped early",
            )),
        };

        let result = timeout(
            Duration::from_millis(100),
            wait_for_shutdown(daemon, future::pending::<io::Result<()>>()),
        )
        .await;

        assert!(matches!(
            result,
            Ok(Err(DaemonError::Io(error))) if error.to_string() == "dns stopped early"
        ));
        assert!(!paths.daemon_socket().exists());

        Ok(())
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
            dns: super::dns::RunningDnsResolver::aborted_for_test(),
        };

        let result = daemon.shutdown().await;

        assert!(matches!(result, Err(DaemonError::Task(error)) if error.is_cancelled()));
        assert!(!paths.daemon_socket().exists());

        Ok(())
    }

    #[tokio::test]
    async fn daemon_start_and_shutdown_write_structured_daemon_log() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let daemon = RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;

        daemon.shutdown().await?;

        let content = state::fs::read_to_string(&paths.daemon_log())?;
        let events = content
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?;

        assert!(
            events
                .iter()
                .any(|event| event["event"] == "daemon_started")
        );
        assert!(
            events
                .iter()
                .any(|event| event["event"] == "daemon_stopped")
        );
        assert!(events.iter().all(|event| event["target"] == "daemon"));

        Ok(())
    }
}
