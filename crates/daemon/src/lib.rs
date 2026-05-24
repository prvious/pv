use std::io;

use serde::{Deserialize, Serialize};
use state::{Database, PvPaths};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug)]
pub struct RunningDaemon {
    paths: PvPaths,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), DaemonError>>,
}

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("state error: {0}")]
    State(#[from] state::StateError),

    #[error("daemon task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

#[derive(Debug, Deserialize)]
struct DaemonRequest {
    protocol_version: u16,

    #[serde(flatten)]
    command: DaemonCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum DaemonCommand {
    Health,
    RunJob { kind: String, scope: String },
}

#[derive(Debug, Serialize)]
struct DaemonResponse<'message> {
    #[serde(rename = "type")]
    line_type: &'static str,
    protocol_version: u16,
    status: ResponseStatus,
    message: &'message str,

    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<&'message str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResponseStatus {
    Ok,
    Accepted,
    Error,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DaemonEvent<'message> {
    JobStarted {
        job_id: &'message str,
        kind: &'message str,
        scope: &'message str,
    },
    Progress {
        job_id: &'message str,
        message: &'message str,
    },
    JobCompleted {
        job_id: &'message str,
        summary: &'message str,
    },
}

impl RunningDaemon {
    pub async fn start(paths: PvPaths) -> Result<Self, DaemonError> {
        state::fs::ensure_layout(&paths)?;
        Database::open(&paths)?;
        let listener = UnixListener::bind(paths.daemon_socket())?;
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let server_paths = paths.clone();
        let task =
            tokio::spawn(async move { serve(server_paths, listener, shutdown_receiver).await });

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

async fn serve(
    paths: PvPaths,
    listener: UnixListener,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), DaemonError> {
    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _address) = accepted?;
                handle_connection(paths.clone(), stream).await?;
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

async fn run_job(
    paths: PvPaths,
    mut stream: UnixStream,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    let mut database = Database::open(&paths)?;
    let job = database.start_job(kind, scope)?;

    write_line(
        &mut stream,
        &DaemonResponse {
            line_type: "response",
            protocol_version: PROTOCOL_VERSION,
            status: ResponseStatus::Accepted,
            message: "job accepted",
            job_id: Some(&job.id),
        },
    )
    .await?;
    write_line(
        &mut stream,
        &DaemonEvent::JobStarted {
            job_id: &job.id,
            kind,
            scope,
        },
    )
    .await?;
    write_line(
        &mut stream,
        &DaemonEvent::Progress {
            job_id: &job.id,
            message: "stub job completed without reconciliation work",
        },
    )
    .await?;

    let summary = "stub job completed";
    database.complete_job(&job.id, summary)?;
    write_line(
        &mut stream,
        &DaemonEvent::JobCompleted {
            job_id: &job.id,
            summary,
        },
    )
    .await
}

async fn write_line(stream: &mut UnixStream, line: &impl Serialize) -> Result<(), DaemonError> {
    let encoded = serde_json::to_string(line)?;

    stream.write_all(encoded.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    Ok(())
}
