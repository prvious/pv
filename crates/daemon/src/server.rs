use state::PvPaths;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::DaemonError;
use crate::jobs::run_job;
use crate::protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, PROTOCOL_VERSION, ResponseStatus, write_line,
};

pub(crate) async fn serve(
    paths: PvPaths,
    listener: UnixListener,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), DaemonError> {
    let mut connections = JoinSet::new();

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

                        connections.spawn(async move {
                            handle_connection(connection_paths, stream).await
                        });
                    }
                    Err(_error) => continue,
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

async fn handle_connection(paths: PvPaths, stream: UnixStream) -> Result<(), DaemonError> {
    use futures_util::StreamExt;

    let mut transport = crate::protocol::transport(stream);
    let Some(line) = transport.next().await else {
        return Ok(());
    };
    let request = serde_json::from_str::<DaemonRequest>(&line?)?;

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
        DaemonCommand::RunJob { kind, scope } => run_job(paths, transport, &kind, &scope).await,
    }
}
