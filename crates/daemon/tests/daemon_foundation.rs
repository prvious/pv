use anyhow::Result;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use serde_json::{Value, json};
use state::{Database, PvPaths};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

#[tokio::test]
async fn socket_protocol_streams_job_progress_and_persists_final_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "system",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "socket_protocol_streams_job_progress_and_persists_final_status",
        (lines, database.recent_jobs()?),
    )?;

    Ok(())
}

#[tokio::test]
async fn unsupported_job_streams_failure_event_and_persists_failed_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "unsupported",
            "scope": "system",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "unsupported_job_streams_failure_event_and_persists_failed_status",
        (lines, database.recent_jobs()?),
    )?;

    Ok(())
}

#[tokio::test]
async fn valid_reconciliation_scopes_stream_stub_completion() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    let project_lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "project:project_1",
        }),
    )
    .await?;
    let resource_lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "resource:mysql:8.4",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "valid_reconciliation_scopes_stream_stub_completion",
        (project_lines, resource_lines, database.recent_jobs()?),
    )?;

    Ok(())
}

#[tokio::test]
async fn protocol_mismatch_returns_restart_guidance_without_creating_a_job() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION + 1,
            "command": "health",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "protocol_mismatch_returns_restart_guidance_without_creating_a_job",
        (lines, database.recent_jobs()?),
    )?;

    Ok(())
}

#[tokio::test]
async fn malformed_request_does_not_stop_accepting_connections() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    send_raw_request(&paths, "not-json\n").await?;
    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "health",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    assert_debug_snapshot!(lines);

    Ok(())
}

#[tokio::test]
async fn idle_client_without_newline_does_not_block_health_requests() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let mut idle_stream = UnixStream::connect(paths.daemon_socket()).await?;

    idle_stream.write_all(b"{").await?;

    let lines = timeout(
        Duration::from_secs(2),
        request_lines(
            &paths,
            json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "health",
            }),
        ),
    )
    .await??;

    daemon.shutdown().await?;

    assert_debug_snapshot!(lines);

    Ok(())
}

#[tokio::test]
async fn start_removes_stale_socket_before_binding() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    state::fs::ensure_layout(&paths)?;
    let stale_listener = tokio::net::UnixListener::bind(paths.daemon_socket())?;
    drop(stale_listener);

    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "health",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    assert_debug_snapshot!(lines);

    Ok(())
}

#[tokio::test]
async fn disconnected_job_stream_still_persists_final_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    send_raw_request(
        &paths,
        &format!(
            "{}\n",
            json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "run_job",
                "kind": "reconcile",
                "scope": "system",
            })
        ),
    )
    .await?;
    let health_lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "health",
        }),
    )
    .await?;
    assert_eq!(health_lines.len(), 1);
    assert_eq!(health_lines[0]["type"], json!("response"));
    assert_eq!(
        health_lines[0]["protocol_version"],
        json!(daemon::PROTOCOL_VERSION)
    );
    assert_eq!(health_lines[0]["status"], json!("ok"));
    assert_eq!(health_lines[0]["message"], json!("daemon healthy"));

    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "disconnected_job_stream_still_persists_final_status",
        database.recent_jobs()?,
    )?;

    Ok(())
}

fn assert_with_normalized_timestamps(
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");

    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}

async fn send_raw_request(paths: &PvPaths, request: &str) -> Result<()> {
    let mut stream = UnixStream::connect(paths.daemon_socket()).await?;
    stream.write_all(request.as_bytes()).await?;
    stream.shutdown().await?;

    Ok(())
}

async fn request_lines(paths: &PvPaths, request: Value) -> Result<Vec<Value>> {
    let mut stream = UnixStream::connect(paths.daemon_socket()).await?;
    let request = serde_json::to_string(&request)?;
    stream.write_all(request.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let mut reader = BufReader::new(stream);
    let mut lines = Vec::new();

    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await?;

        if bytes == 0 {
            break;
        }

        lines.push(serde_json::from_str(line.trim_end())?);
    }

    Ok(lines)
}
