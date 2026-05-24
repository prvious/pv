use state::PvPaths;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

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
    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _address) = accepted?;
                // A malformed or disconnected client must not stop the daemon accept loop.
                if let Err(_error) = handle_connection(paths.clone(), stream).await {
                    continue;
                }
            }
        }
    }
}

async fn handle_connection(paths: PvPaths, stream: UnixStream) -> Result<(), DaemonError> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }

    let mut stream = reader.into_inner();
    let request = serde_json::from_str::<DaemonRequest>(line.trim_end())?;

    if request.protocol_version != PROTOCOL_VERSION {
        return write_line(
            &mut stream,
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
                &mut stream,
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
        DaemonCommand::RunJob { kind, scope } => run_job(paths, stream, &kind, &scope).await,
    }
}
