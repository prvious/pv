use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use state::{PvPaths, StateError, UpdateLock};
use tokio::io::AsyncRead;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::DaemonError;
use crate::ipc::{LocalListener, LocalStream};
use crate::jobs::{
    record_background_reconciliation_error, run_background_reconciliation_job, run_job,
};
use crate::managed_resources::ManagedResourceRuntimeCatalog;
use crate::reconciliation::ReconciliationQueue;
use crate::watcher::ProjectConfigWatcher;
use protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, DaemonTransport, PROTOCOL_VERSION, write_line,
};

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);
const PROJECT_CONFIG_DEBOUNCE: Duration = Duration::from_millis(50);
const PROJECT_CONFIG_WATCH_INTERVAL: Duration = Duration::from_millis(100);
const REQUEST_LINE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn serve(
    paths: PvPaths,
    listener: LocalListener,
    mut shutdown: oneshot::Receiver<()>,
    runtime_catalog: Option<Arc<ManagedResourceRuntimeCatalog>>,
) -> Result<(), DaemonError> {
    let mut connections = JoinSet::new();
    let queue = ReconciliationQueue::new();
    let background_paths = paths.clone();
    let background_queue = queue.clone();
    let background_runtime_catalog = runtime_catalog.clone();
    let debouncer = crate::reconciliation::ReconciliationDebouncer::new(
        PROJECT_CONFIG_DEBOUNCE,
        move |scope| {
            let paths = background_paths.clone();
            let queue = background_queue.clone();
            let runtime_catalog = background_runtime_catalog.clone();
            let _task = tokio::spawn(async move {
                let scope_text = scope.to_string();
                if let Err(error) = run_background_reconciliation_job(
                    paths.clone(),
                    queue,
                    scope,
                    runtime_catalog.as_deref(),
                )
                .await
                {
                    let _result =
                        record_background_reconciliation_error(&paths, &scope_text, &error);
                }
            });
        },
    );
    let watcher =
        ProjectConfigWatcher::new(paths.clone(), debouncer, PROJECT_CONFIG_WATCH_INTERVAL);
    let mut watcher_task = tokio::spawn(watcher.run());

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                watcher_task.abort();
                let _join_result = watcher_task.await;
                connections.abort_all();
                while connections.join_next().await.is_some() {}

                return Ok(());
            }
            watcher_result = &mut watcher_task => {
                match watcher_result {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(error)) => return Err(error),
                    Err(error) if error.is_panic() => return Err(error.into()),
                    Err(_error) => return Ok(()),
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _address)) => {
                        let connection_paths = paths.clone();
                        let connection_queue = queue.clone();
                        let connection_runtime_catalog = runtime_catalog.clone();

                        connections.spawn(async move {
                            handle_connection(
                                connection_paths,
                                connection_queue,
                                stream,
                                connection_runtime_catalog,
                            )
                            .await
                        });
                    }
                    Err(_error) => {
                        sleep(ACCEPT_ERROR_BACKOFF).await;
                    }
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                match joined {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(_error))) => {}
                    Some(Err(error)) if error.is_panic() => return Err(error.into()),
                    Some(Err(_error)) => {}
                }
            }
        }
    }
}

async fn handle_connection(
    paths: PvPaths,
    queue: ReconciliationQueue,
    stream: LocalStream,
    runtime_catalog: Option<Arc<ManagedResourceRuntimeCatalog>>,
) -> Result<(), DaemonError> {
    let mut transport = protocol::transport(stream);
    let Some(line) = read_request_line(&mut transport, REQUEST_LINE_TIMEOUT).await? else {
        return Ok(());
    };
    let request = serde_json::from_str::<DaemonRequest>(&line)?;

    if request.protocol_version != PROTOCOL_VERSION {
        write_line(
            &mut transport,
            &DaemonResponse::error("daemon protocol mismatch; run `pv daemon:restart`"),
        )
        .await?;

        return Ok(());
    }

    match request.command {
        DaemonCommand::Health => {
            write_line(&mut transport, &DaemonResponse::ok("daemon healthy")).await?;

            Ok(())
        }
        DaemonCommand::RunJob { kind, scope } => {
            let update_lock = match UpdateLock::acquire(&paths) {
                Ok(update_lock) => update_lock,
                Err(error @ StateError::UpdateInProgress { .. }) => {
                    write_line(&mut transport, &DaemonResponse::error(error.to_string())).await?;

                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };

            let result = run_job(
                paths,
                queue,
                transport,
                &kind,
                &scope,
                runtime_catalog.as_deref(),
            )
            .await;
            drop(update_lock);

            result
        }
    }
}

async fn read_request_line<Stream>(
    transport: &mut DaemonTransport<Stream>,
    read_timeout: Duration,
) -> Result<Option<String>, DaemonError>
where
    Stream: AsyncRead + Unpin,
{
    match timeout(read_timeout, transport.next()).await {
        Ok(Some(line)) => Ok(Some(line?)),
        Ok(None) | Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::duplex;

    use super::read_request_line;
    use protocol::transport;

    #[tokio::test]
    async fn request_line_read_times_out_for_idle_connection() -> Result<(), crate::DaemonError> {
        let (_client, server) = duplex(1024);
        let mut transport = transport(server);

        let line = read_request_line(&mut transport, Duration::from_millis(10)).await?;

        assert!(line.is_none());

        Ok(())
    }
}
