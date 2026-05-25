use std::time::Duration;

use futures_util::StreamExt;
use state::PvPaths;
use tokio::io::AsyncRead;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::DaemonError;
use crate::ipc::{LocalListener, LocalStream};
use crate::jobs::{run_background_reconciliation_job, run_job};
use crate::protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, DaemonTransport, PROTOCOL_VERSION,
    ResponseStatus, write_line,
};
use crate::reconciliation::ReconciliationQueue;
use crate::watcher::ProjectConfigWatcher;

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);
const PROJECT_CONFIG_DEBOUNCE: Duration = Duration::from_millis(50);
const PROJECT_CONFIG_WATCH_INTERVAL: Duration = Duration::from_millis(100);
const REQUEST_LINE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn serve(
    paths: PvPaths,
    listener: LocalListener,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), DaemonError> {
    let mut connections = JoinSet::new();
    let queue = ReconciliationQueue::new();
    let background_paths = paths.clone();
    let background_queue = queue.clone();
    let debouncer = crate::reconciliation::ReconciliationDebouncer::new(
        PROJECT_CONFIG_DEBOUNCE,
        move |scope| {
            let paths = background_paths.clone();
            let queue = background_queue.clone();
            let _task = tokio::spawn(async move {
                let _result = run_background_reconciliation_job(paths, queue, scope).await;
            });
        },
    );
    let watcher =
        ProjectConfigWatcher::new(paths.clone(), debouncer, PROJECT_CONFIG_WATCH_INTERVAL);
    connections.spawn(async move {
        watcher.run().await;

        Ok::<(), DaemonError>(())
    });

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                connections.abort_all();
                while connections.join_next().await.is_some() {}

                return Ok(());
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _address)) => {
                        let connection_paths = paths.clone();
                        let connection_queue = queue.clone();

                        connections.spawn(async move {
                            handle_connection(connection_paths, connection_queue, stream).await
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
) -> Result<(), DaemonError> {
    let mut transport = crate::protocol::transport(stream);
    let Some(line) = read_request_line(&mut transport, REQUEST_LINE_TIMEOUT).await? else {
        return Ok(());
    };
    let request = serde_json::from_str::<DaemonRequest>(&line)?;

    if request.protocol_version != PROTOCOL_VERSION {
        return write_line(
            &mut transport,
            &DaemonResponse {
                line_type: "response",
                protocol_version: PROTOCOL_VERSION,
                status: ResponseStatus::Error,
                message: "daemon protocol mismatch; run `pv daemon:restart`",
                job_id: None,
            },
        )
        .await;
    }

    match request.command {
        DaemonCommand::Health => {
            write_line(
                &mut transport,
                &DaemonResponse {
                    line_type: "response",
                    protocol_version: PROTOCOL_VERSION,
                    status: ResponseStatus::Ok,
                    message: "daemon healthy",
                    job_id: None,
                },
            )
            .await
        }
        DaemonCommand::RunJob { kind, scope } => {
            run_job(paths, queue, transport, &kind, &scope).await
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
    use crate::protocol::transport;

    #[tokio::test]
    async fn request_line_read_times_out_for_idle_connection() -> Result<(), crate::DaemonError> {
        let (_client, server) = duplex(1024);
        let mut transport = transport(server);

        let line = read_request_line(&mut transport, Duration::from_millis(10)).await?;

        assert!(line.is_none());

        Ok(())
    }
}
