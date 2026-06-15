use anyhow::{Result, anyhow};
use camino_tempfile::tempdir;
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use insta::{Settings, assert_debug_snapshot};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use state::{
    DNS_PREFERRED_PORT, Database, JobRecord, JobStatus, LinkProjectInput, PortOwner, PortRequest,
    PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, UpdateLock,
};
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UdpSocket, UnixListener, UnixStream};
use tokio::time::{sleep, timeout};

const EXPECTED_DNS_TTL_SECONDS: u32 = 5;
const TEST_ARTIFACT_MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";
const EMPTY_ARTIFACT_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": []
}
"#;

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
        (resource_lines, database.recent_jobs()?),
    )?;

    Ok(())
}

#[tokio::test]
async fn update_lock_rejects_mutating_jobs_but_keeps_health_available() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let update_lock = UpdateLock::acquire(&paths)?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    let run_job_lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "system",
        }),
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
    let update_check_lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "managed_resource_update_check",
        }),
    )
    .await?;

    daemon.shutdown().await?;
    drop(update_lock);

    let database = Database::open(&paths)?;
    let run_job_lines = normalize_update_lock_path(run_job_lines, paths.update_lock().as_str());
    let update_check_lines =
        normalize_update_lock_path(update_check_lines, paths.update_lock().as_str());

    assert_with_normalized_timestamps(
        "update_lock_rejects_mutating_jobs_but_keeps_health_available",
        (
            run_job_lines,
            health_lines,
            update_check_lines,
            database.recent_jobs()?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn daemon_start_writes_migration_failed_startup_marker() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    Database::open(&paths)?;
    let connection = Connection::open(paths.db())?;
    connection.execute(
        "UPDATE pv_migrations SET name = ?1 WHERE version = ?2",
        params!["wrong_name", 1_i64],
    )?;

    let result =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await;

    assert!(result.is_err());
    let marker = state::fs::read_to_string(&paths.daemon_startup_error())?;
    assert_debug_snapshot!(serde_json::from_str::<Value>(&marker)?);

    Ok(())
}

#[tokio::test]
async fn daemon_start_removes_stale_startup_marker_before_health() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    state::fs::write_sensitive_file(
        &paths.daemon_startup_error(),
        r#"{"kind":"startup_failed","message":"stale"}"#,
    )?;

    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    let health_lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "health",
        }),
    )
    .await?;
    daemon.shutdown().await?;

    assert_eq!(health_lines[0]["status"], json!("ok"));
    assert!(!state::fs::path_entry_exists(
        &paths.daemon_startup_error()
    )?);

    Ok(())
}

#[tokio::test]
async fn daemon_start_marks_abandoned_running_jobs_failed() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    database.start_job("reconcile", "system")?;
    database.start_job("update", "system")?;
    drop(database);

    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "daemon_start_marks_abandoned_running_jobs_failed",
        database.recent_jobs()?,
    )?;

    Ok(())
}

#[tokio::test]
async fn duplicate_daemon_start_does_not_fail_live_running_jobs() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    database.start_job("reconcile", "system")?;
    drop(database);

    let _listener = UnixListener::bind(paths.daemon_socket())?;

    let result =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await;

    assert!(matches!(
        result,
        Err(daemon::DaemonError::SocketInUse { path }) if path == paths.daemon_socket()
    ));

    let database = Database::open(&paths)?;
    let jobs = database.recent_jobs()?;

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, JobStatus::Running);
    assert!(jobs[0].finished_at.is_none());
    assert!(jobs[0].error.is_none());

    Ok(())
}

#[tokio::test]
async fn managed_resource_update_check_returns_success_response() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let manifest_client = ScriptedManifestClient::new(EMPTY_ARTIFACT_MANIFEST);
    let manifest_requests = manifest_client.request_count();
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters_with_manifest_client(
            paths.clone(),
            TEST_ARTIFACT_MANIFEST_URL,
            manifest_client,
        )
        .await?;

    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "managed_resource_update_check",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    assert_with_normalized_timestamps(
        "managed_resource_update_check_returns_success_response",
        (lines, manifest_request_count(&manifest_requests)?),
    )?;

    Ok(())
}

#[tokio::test]
async fn update_job_refreshes_manifest_without_installed_tracks_and_persists_success() -> Result<()>
{
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let manifest_client = ScriptedManifestClient::new(EMPTY_ARTIFACT_MANIFEST);
    let manifest_requests = manifest_client.request_count();
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters_with_manifest_client(
            paths.clone(),
            TEST_ARTIFACT_MANIFEST_URL,
            manifest_client,
        )
        .await?;
    let client_paths = paths.clone();

    let completed = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "update", "system")
    })
    .await??;
    daemon.shutdown().await?;

    let database = Database::open(&paths)?;
    let job = wait_for_succeeded_job_id(&paths, &completed.id).await?;

    assert_eq!(completed.summary, "none installed");
    assert_eq!(job.kind, "update");
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(manifest_request_count(&manifest_requests)?, 1);
    assert_eq!(database.recent_jobs()?.len(), 1);

    Ok(())
}

#[tokio::test]
async fn update_lock_rejects_update_jobs_before_manifest_refresh() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let update_lock = UpdateLock::acquire(&paths)?;
    let manifest_client = ScriptedManifestClient::new(EMPTY_ARTIFACT_MANIFEST);
    let manifest_requests = manifest_client.request_count();
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters_with_manifest_client(
            paths.clone(),
            TEST_ARTIFACT_MANIFEST_URL,
            manifest_client,
        )
        .await?;

    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "update",
            "scope": "system",
        }),
    )
    .await?;

    daemon.shutdown().await?;
    drop(update_lock);

    let lines = normalize_update_lock_path(lines, paths.update_lock().as_str());
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "update_lock_rejects_update_jobs_before_manifest_refresh",
        (
            lines,
            database.recent_jobs()?,
            manifest_request_count(&manifest_requests)?,
        ),
    )?;

    Ok(())
}

fn normalize_update_lock_path(mut lines: Vec<Value>, update_lock_path: &str) -> Vec<Value> {
    for line in &mut lines {
        let Some(message) = line.get_mut("message") else {
            continue;
        };
        let Some(message_text) = message.as_str() else {
            continue;
        };

        *message = json!(message_text.replace(update_lock_path, "<update-lock>"));
    }

    lines
}

#[tokio::test]
async fn blocking_client_submits_reconciliation_jobs() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let client_paths = paths.clone();

    let submitted = tokio::task::spawn_blocking(move || {
        daemon::submit_job_blocking(client_paths, "reconcile", "system")
    })
    .await??;
    let job = wait_for_succeeded_job_id(&paths, &submitted.id).await?;
    daemon.shutdown().await?;

    assert_eq!(job.kind, "reconcile");
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Succeeded);

    Ok(())
}

#[tokio::test]
async fn blocking_client_waits_for_reconciliation_stream_completion() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let client_paths = paths.clone();

    let completed = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "reconcile", "system")
    })
    .await??;
    let job = wait_for_succeeded_job_id(&paths, &completed.id).await?;
    daemon.shutdown().await?;

    assert_eq!(
        completed.summary,
        "Gateway runtime skipped; FrankenPHP is not installed"
    );
    assert_eq!(job.kind, "reconcile");
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Succeeded);

    Ok(())
}

#[tokio::test]
async fn blocking_client_checks_managed_resource_updates() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let manifest_client = ScriptedManifestClient::new(EMPTY_ARTIFACT_MANIFEST);
    let manifest_requests = manifest_client.request_count();
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters_with_manifest_client(
            paths.clone(),
            TEST_ARTIFACT_MANIFEST_URL,
            manifest_client,
        )
        .await?;
    let client_paths = paths.clone();

    let update_check = tokio::task::spawn_blocking(move || {
        daemon::managed_resource_update_check_blocking(client_paths)
    })
    .await??;
    daemon.shutdown().await?;

    assert!(update_check.managed_resources.is_empty());
    assert_eq!(manifest_request_count(&manifest_requests)?, 1);

    Ok(())
}

#[tokio::test]
async fn system_reconciliation_reconciles_linked_project_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project_path = tempdir.path().join("project");
    let config_path = project_path.join("pv.yml");
    state::fs::write_sensitive_file(
        &config_path,
        "env:\n  APP_URL: \"${project_url}\"\n  APP_NAME: setup\n",
    )?;
    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_path.clone(),
        original_path: project_path.clone(),
        primary_hostname: "project.test".to_owned(),
        config_path,
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    drop(database);

    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let client_paths = paths.clone();
    let completed = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "reconcile", "system")
    })
    .await??;
    daemon.shutdown().await?;

    let database = Database::open(&paths)?;
    let job = wait_for_succeeded_job_id(&paths, &completed.id).await?;
    let project = database
        .projects()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(completed.summary, "Project env rendered");
    assert_eq!(job.summary.as_deref(), Some("Project env rendered"));
    assert_eq!(
        state::fs::read_to_string(&project_path.join(".env"))?,
        "# >>> PV MANAGED\nAPP_NAME=setup\nAPP_URL=https://project.test\n# <<< PV MANAGED\n"
    );
    assert!(database.project_env_observed_state(&project.id)?.is_some());

    Ok(())
}

#[tokio::test]
async fn blocking_client_reports_failed_job_streams() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let client_paths = paths.clone();

    let result = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "unsupported", "system")
    })
    .await?;
    daemon.shutdown().await?;
    let database = Database::open(&paths)?;
    let jobs = database.recent_jobs()?;

    assert!(matches!(
        result,
        Err(daemon::DaemonError::DaemonRejected { message })
            if message == "unsupported daemon job `unsupported` with scope `system`"
    ));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].kind, "unsupported");
    assert_eq!(jobs[0].scope, "system");
    assert_eq!(jobs[0].status, JobStatus::Failed);
    assert_eq!(
        jobs[0].error.as_deref(),
        Some("unsupported daemon job `unsupported` with scope `system`")
    );

    Ok(())
}

#[tokio::test]
async fn blocking_client_rejects_protocol_mismatch_response() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let listener = UnixListener::bind(paths.daemon_socket())?;
    let server = tokio::spawn(async move {
        let (mut stream, _address) = listener.accept().await?;
        let mut request = String::new();
        let mut reader = BufReader::new(&mut stream);
        reader.read_line(&mut request).await?;
        drop(reader);
        stream
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "type": "response",
                        "protocol_version": daemon::PROTOCOL_VERSION + 1,
                        "status": "accepted",
                        "message": "job accepted",
                        "job_id": "job_1",
                    })
                )
                .as_bytes(),
            )
            .await?;

        Ok::<(), anyhow::Error>(())
    });
    let client_paths = paths.clone();

    let result = tokio::task::spawn_blocking(move || {
        daemon::submit_job_blocking(client_paths, "reconcile", "system")
    })
    .await?;

    server.await??;
    assert!(matches!(
        result,
        Err(daemon::DaemonError::ProtocolMismatch {
            expected: daemon::PROTOCOL_VERSION,
            actual,
        }) if actual == daemon::PROTOCOL_VERSION + 1
    ));

    Ok(())
}

#[tokio::test]
async fn blocking_client_times_out_when_daemon_withholds_response() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let listener = UnixListener::bind(paths.daemon_socket())?;
    let server = tokio::spawn(async move {
        let (mut stream, _address) = listener.accept().await?;
        let mut request = String::new();
        let mut reader = BufReader::new(&mut stream);
        reader.read_line(&mut request).await?;
        tokio::time::sleep(Duration::from_secs(6)).await;

        Ok::<(), anyhow::Error>(())
    });
    let client_paths = paths.clone();
    let client = tokio::task::spawn_blocking(move || {
        daemon::submit_job_blocking(client_paths, "reconcile", "system")
    });

    let result = timeout(Duration::from_secs(5), client).await??;

    server.abort();
    assert!(matches!(
        result,
        Err(daemon::DaemonError::ProtocolTimedOut { phase }) if phase == "response"
    ));

    Ok(())
}

#[tokio::test]
async fn invalid_reconciliation_scope_reports_scope_parse_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    let lines = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "project:",
        }),
    )
    .await?;

    daemon.shutdown().await?;

    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "invalid_reconciliation_scope_reports_scope_parse_failure",
        (lines, database.recent_jobs()?),
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

#[tokio::test]
async fn project_config_watcher_enqueues_project_reconciliation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project_path = tempdir.path().join("project");
    let config_path = project_path.join("pv.yml");
    state::fs::write_sensitive_file(&config_path, "php: '8.3'\n")?;
    let mut database = Database::open(&paths)?;
    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO projects (id, path, primary_hostname, config_path, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "project_1",
                project_path.as_str(),
                "project.test",
                config_path.as_str(),
                "2026-05-24T00:00:00Z",
                "2026-05-24T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;

    write_file_after_modified_time_tick(
        &config_path,
        "env:\n  APP_URL: \"${project_url}\"\n  APP_NAME: watched\n",
    )
    .await?;

    let job = wait_for_succeeded_job_scope(&paths, "project:project_1").await?;

    daemon.shutdown().await?;

    assert_eq!(job.kind, "reconcile");
    assert_eq!(job.scope, "project:project_1");
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.summary.as_deref(), Some("Project env rendered"));
    assert_eq!(
        state::fs::read_to_string(&project_path.join(".env"))?,
        "# >>> PV MANAGED\nAPP_NAME=watched\nAPP_URL=https://project.test\n# <<< PV MANAGED\n"
    );

    Ok(())
}

#[tokio::test]
async fn dns_resolver_answers_udp_a_and_aaaa_for_test_hostnames() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    let a_response = udp_dns_query(port, &dns_query("acme.test.", RecordType::A)?).await?;
    assert_common_dns_response(&a_response, "acme.test.", RecordType::A)?;
    assert_loopback_answer(
        &a_response,
        "acme.test.",
        RecordType::A,
        RData::A(A::new(127, 0, 0, 1)),
    )?;

    let aaaa_response = udp_dns_query(port, &dns_query("acme.test.", RecordType::AAAA)?).await?;
    assert_common_dns_response(&aaaa_response, "acme.test.", RecordType::AAAA)?;
    assert_loopback_answer(
        &aaaa_response,
        "acme.test.",
        RecordType::AAAA,
        RData::AAAA(AAAA::new(0, 0, 0, 0, 0, 0, 0, 1)),
    )?;

    daemon.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn dns_resolver_returns_nodata_and_survives_malformed_udp() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    socket
        .send_to(b"not-a-dns-query", dns_address(port))
        .await?;

    let mx_response = udp_dns_query(port, &dns_query("acme.test.", RecordType::MX)?).await?;
    assert_common_dns_response(&mx_response, "acme.test.", RecordType::MX)?;
    assert!(mx_response.answers.is_empty());

    let external_response = udp_dns_query(port, &dns_query("example.com.", RecordType::A)?).await?;
    assert_common_dns_response(&external_response, "example.com.", RecordType::A)?;
    assert!(external_response.answers.is_empty());

    daemon.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn dns_resolver_answers_tcp_queries() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    let response = tcp_dns_query(port, &dns_query("acme.test.", RecordType::A)?).await?;
    assert_common_dns_response(&response, "acme.test.", RecordType::A)?;
    assert_loopback_answer(
        &response,
        "acme.test.",
        RecordType::A,
        RData::A(A::new(127, 0, 0, 1)),
    )?;

    daemon.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn dns_resolver_falls_back_when_preferred_port_is_unavailable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let (_tcp_blocker, _udp_blocker) = bind_preferred_dns_port_pair().await?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    assert_ne!(port, DNS_PREFERRED_PORT);
    assert!((RUNTIME_PORT_FALLBACK_START..=RUNTIME_PORT_FALLBACK_END).contains(&port));

    daemon.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn dns_resolver_start_does_not_reassign_persisted_port_on_bind_conflict() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let (bound_dns_port, _tcp_listener, _udp_socket) = bind_loopback_tcp_udp_pair()?;
    let mut database = Database::open(&paths)?;
    database.assign_port(
        PortRequest::dns(bound_dns_port, bound_dns_port, bound_dns_port),
        |candidate| candidate == bound_dns_port,
    )?;
    drop(database);

    let result = daemon::RunningDaemon::start(paths.clone()).await;
    if let Ok(daemon) = result {
        daemon.shutdown().await?;
        return Err(anyhow!(
            "daemon started after persisted DNS port bind conflict"
        ));
    }
    let error = result
        .err()
        .ok_or_else(|| anyhow!("missing daemon error"))?;
    let persisted_port = dns_port(&paths)?;

    assert!(matches!(
        error,
        daemon::DaemonError::DnsBind {
            port,
            ..
        } if port == bound_dns_port
    ));
    assert_eq!(persisted_port, bound_dns_port);
    assert!(!state::fs::path_entry_exists(&paths.daemon_socket())?);

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

#[derive(Debug)]
struct ScriptedManifestClient {
    body: &'static str,
    request_count: Arc<Mutex<usize>>,
}

impl ScriptedManifestClient {
    fn new(body: &'static str) -> Self {
        Self {
            body,
            request_count: Arc::new(Mutex::new(0)),
        }
    }

    fn request_count(&self) -> Arc<Mutex<usize>> {
        Arc::clone(&self.request_count)
    }
}

impl resources::ResourceHttpClient for ScriptedManifestClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        let mut request_count = self.request_count.lock().map_err(|_poison| {
            resources::ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: "manifest request count lock poisoned".to_string(),
            }
        })?;
        *request_count += 1;

        Ok(self.body.to_string())
    }

    fn download(&self, url: &str, _writer: &mut dyn std::io::Write) -> resources::Result<()> {
        Err(resources::ResourcesError::HttpRequestFailed {
            url: url.to_string(),
            reason: "downloads are not used by update checks".to_string(),
        })
    }
}

fn manifest_request_count(request_count: &Arc<Mutex<usize>>) -> Result<usize> {
    request_count
        .lock()
        .map(|count| *count)
        .map_err(|_poison| anyhow!("manifest request count lock poisoned"))
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

async fn wait_for_succeeded_job_id(paths: &PvPaths, id: &str) -> Result<JobRecord> {
    for _attempt in 0..50 {
        let database = Database::open(paths)?;
        if let Some(job) = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == id && job.status == JobStatus::Succeeded)
        {
            return Ok(job);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Err(anyhow!("succeeded job with id {id:?} was not recorded"))
}

async fn wait_for_succeeded_job_scope(paths: &PvPaths, scope: &str) -> Result<JobRecord> {
    for _attempt in 0..50 {
        let database = Database::open(paths)?;
        if let Some(job) = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == scope && job.status == JobStatus::Succeeded)
        {
            return Ok(job);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Err(anyhow::anyhow!(
        "succeeded job with scope {scope:?} was not recorded"
    ))
}

async fn write_file_after_modified_time_tick(path: &camino::Utf8Path, content: &str) -> Result<()> {
    let before = state::fs::modified_at(path)?;

    for _attempt in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        state::fs::write_sensitive_file(path, content)?;

        if state::fs::modified_at(path)? != before {
            return Ok(());
        }
    }

    Err(anyhow!("modified time did not advance for {path}"))
}

fn dns_query(name: &str, record_type: RecordType) -> Result<Vec<u8>> {
    let query = Query::query(Name::from_str(name)?, record_type);
    let mut message = Message::new(42, MessageType::Query, OpCode::Query);
    message.metadata.recursion_desired = true;
    message.add_query(query);

    Ok(message.to_bytes()?)
}

fn dns_port(paths: &PvPaths) -> Result<u16> {
    let database = Database::open(paths)?;
    let port = database
        .assigned_ports()?
        .into_iter()
        .find_map(|assignment| match assignment.owner {
            PortOwner::Dns => Some(assignment.port),
            _ => None,
        });

    port.ok_or_else(|| anyhow!("DNS port was not assigned"))
}

async fn udp_dns_query(port: u16, query: &[u8]) -> Result<Message> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    socket.send_to(query, dns_address(port)).await?;
    let mut response = vec![0; 512];
    let (length, _address) =
        timeout(Duration::from_secs(2), socket.recv_from(&mut response)).await??;
    response.truncate(length);

    Ok(Message::from_vec(&response)?)
}

async fn tcp_dns_query(port: u16, query: &[u8]) -> Result<Message> {
    let query_length = u16::try_from(query.len())?;
    let mut stream = TcpStream::connect(dns_address(port)).await?;
    stream.write_all(&query_length.to_be_bytes()).await?;
    stream.write_all(query).await?;

    let mut length_prefix = [0; 2];
    timeout(
        Duration::from_secs(2),
        stream.read_exact(&mut length_prefix),
    )
    .await??;
    let response_length = usize::from(u16::from_be_bytes(length_prefix));
    let mut response = vec![0; response_length];
    timeout(Duration::from_secs(2), stream.read_exact(&mut response)).await??;

    Ok(Message::from_vec(&response)?)
}

fn dns_address(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

async fn bind_preferred_dns_port_pair() -> Result<(StdTcpListener, StdUdpSocket)> {
    for _attempt in 0..100 {
        match bind_loopback_tcp_udp_at(DNS_PREFERRED_PORT) {
            Ok(blockers) => return Ok(blockers),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow!(
        "could not bind preferred DNS port {DNS_PREFERRED_PORT} after waiting for parallel tests"
    ))
}

fn bind_loopback_tcp_udp_at(port: u16) -> io::Result<(StdTcpListener, StdUdpSocket)> {
    let tcp_listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let udp_socket = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, port))?;

    Ok((tcp_listener, udp_socket))
}

fn bind_loopback_tcp_udp_pair() -> Result<(u16, StdTcpListener, StdUdpSocket)> {
    for _attempt in 0..100 {
        let tcp_listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = tcp_listener.local_addr()?.port();
        let Ok(udp_socket) = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, port)) else {
            continue;
        };

        return Ok((port, tcp_listener, udp_socket));
    }

    Err(anyhow!("could not bind a loopback TCP/UDP port pair"))
}

fn assert_common_dns_response(
    response: &Message,
    name: &str,
    record_type: RecordType,
) -> Result<()> {
    assert_eq!(response.metadata.message_type, MessageType::Response);
    assert_eq!(response.metadata.op_code, OpCode::Query);
    assert!(response.metadata.recursion_desired);
    assert!(response.metadata.authoritative);
    assert!(!response.metadata.recursion_available);
    assert_eq!(response.metadata.response_code, ResponseCode::NoError);
    assert_eq!(response.queries.len(), 1);

    let Some(query) = response.queries.first() else {
        return Err(anyhow!("response did not preserve the query section"));
    };
    assert_eq!(query.name(), &Name::from_str(name)?);
    assert_eq!(query.query_type(), record_type);
    assert_eq!(query.query_class(), DNSClass::IN);

    Ok(())
}

fn assert_loopback_answer(
    response: &Message,
    name: &str,
    record_type: RecordType,
    expected_data: RData,
) -> Result<()> {
    assert_eq!(response.answers.len(), 1);

    let Some(answer) = response.answers.first() else {
        return Err(anyhow!("response did not include an answer"));
    };
    assert_eq!(&answer.name, &Name::from_str(name)?);
    assert_eq!(answer.record_type(), record_type);
    assert_eq!(answer.dns_class, DNSClass::IN);
    assert_eq!(answer.ttl, EXPECTED_DNS_TTL_SECONDS);
    assert_eq!(&answer.data, &expected_data);

    Ok(())
}
