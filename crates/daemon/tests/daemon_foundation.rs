use anyhow::{Result, anyhow};
use camino::Utf8PathBuf;
use camino_tempfile::tempdir;
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use insta::{Settings, assert_debug_snapshot};
use rcgen::generate_simple_self_signed;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use state::{
    AppReleaseLayout, DNS_PREFERRED_PORT, Database, GatewayPort, JobRecord, JobStatus, JobsLock,
    LinkProjectInput, PortOwner, PortRequest, PvPaths, RUNTIME_PORT_FALLBACK_END,
    RUNTIME_PORT_FALLBACK_START, UpdateLock,
};
use std::io::{self, ErrorKind, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UdpSocket, UnixListener, UnixStream};
use tokio::time::{sleep, timeout};

const EXPECTED_DNS_TTL_SECONDS: u32 = 5;
const JOB_STATUS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const JOB_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TEST_ARTIFACT_MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";
const FAKE_CADDY_SCRIPT: &str = r#"#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  python3 - "$3" < "$0.server.py" &
  child="$!"

  cleanup() {
    trap - TERM INT
    kill "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
    exit 0
  }

  trap cleanup TERM INT
  wait "$child"
  exit "$?"
fi

exit 2
"#;
const FAKE_CADDY_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp-server.py"
));
const EMPTY_ARTIFACT_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": []
}
"#;
const CADDY_ARTIFACT_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {
      "name": "caddy",
      "default_track": "2",
      "tracks": [
        {
          "name": "2",
          "artifacts": [
            {
              "artifact_version": "2.11.4-pv1",
              "upstream_version": "2.11.4",
              "pv_build_revision": "1",
              "platform": "any",
              "url": "https://artifacts.example.test/caddy-2.11.4-pv1-any.tar.gz",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "size": 1,
              "published_at": "2026-06-08T00:00:00Z"
            }
          ]
        }
      ]
    }
  ]
}
"#;
const FOUNDATION_FAKE_CADDY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy.sh"
));
const FOUNDATION_FAKE_CADDY_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-caddy-server.py"
));
const SEEDED_GATEWAY_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
const SEEDED_GATEWAY_CLEANUP_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[tokio::test]
async fn socket_protocol_streams_job_progress_and_persists_final_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);

    let lines_result = request_lines(
        &paths,
        json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "system",
        }),
    )
    .await;

    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let lines = propagate_after_cleanup(lines_result, cleanup_result)?;

    let database = Database::open(&paths)?;
    let jobs = database.recent_jobs()?;
    let job_id = jobs
        .iter()
        .find(|job| job.kind == "reconcile" && job.scope == "system")
        .map(|job| job.id.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing system reconciliation job"))?;
    let log = state::fs::read_to_string(&paths.daemon_log())?;
    let mut phase_events = log
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|event| {
            event["event"] == "reconciliation_phase_completed" && event["job_id"] == job_id
        })
        .collect::<Vec<_>>();
    for event in &mut phase_events {
        assert!(event["elapsed_ms"].as_u64().is_some());
        if event["phase"] == "finalization" {
            assert!(event["total_execution_ms"].as_u64().is_some());
        }
        if let Some(record) = event.as_object_mut() {
            for field in ["timestamp", "job_id", "elapsed_ms", "total_execution_ms"] {
                record.remove(field);
            }
        }
    }
    assert_debug_snapshot!("system_reconciliation_phases", phase_events);

    assert_with_normalized_timestamps(
        "socket_protocol_streams_job_progress_and_persists_final_status",
        (lines, jobs),
    )?;

    Ok(())
}

#[tokio::test]
async fn unsupported_job_streams_failure_event_and_persists_failed_status() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let jobs_lock = JobsLock::acquire(&paths)?;
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
    drop(jobs_lock);

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
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);

    let result = async {
        wait_for_succeeded_job_id(&paths, "job_000001").await?;
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

        Ok::<_, anyhow::Error>((resource_lines, Database::open(&paths)?.recent_jobs()?))
    }
    .await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let snapshot = propagate_after_cleanup(result, cleanup_result)?;

    assert_with_normalized_timestamps(
        "valid_reconciliation_scopes_stream_stub_completion",
        snapshot,
    )?;

    Ok(())
}

#[tokio::test]
async fn update_locks_delay_startup_reconciliation_but_keep_health_available() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let update_lock = UpdateLock::acquire(&paths)?;
    let jobs_lock = JobsLock::acquire(&paths)?;
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);

    let result = async {
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

        assert!(Database::open(&paths)?.recent_jobs()?.is_empty());

        drop(jobs_lock);
        drop(update_lock);
        wait_for_succeeded_job_scope(&paths, "system").await?;

        let database = Database::open(&paths)?;
        let run_job_lines =
            normalize_lock_path(run_job_lines, paths.jobs_lock().as_str(), "<jobs-lock>");
        let update_check_lines = normalize_lock_path(
            update_check_lines,
            paths.update_lock().as_str(),
            "<update-lock>",
        );

        Ok::<_, anyhow::Error>((
            run_job_lines,
            health_lines,
            update_check_lines,
            database.recent_jobs()?,
        ))
    }
    .await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let snapshot = propagate_after_cleanup(result, cleanup_result)?;

    assert_with_normalized_timestamps(
        "update_locks_delay_startup_reconciliation_but_keep_health_available",
        snapshot,
    )
}

#[tokio::test]
async fn daemon_shutdown_cancels_startup_reconciliation_waiting_for_jobs_lock() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let jobs_lock = JobsLock::acquire(&paths)?;
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;

    timeout(Duration::from_secs(1), daemon.shutdown()).await??;
    drop(jobs_lock);
    sleep(Duration::from_millis(100)).await;

    let _jobs_lock = JobsLock::acquire(&paths)?;
    assert!(Database::open(&paths)?.recent_jobs()?.is_empty());

    Ok(())
}

#[tokio::test]
async fn daemon_shutdown_cancels_active_startup_reconciliation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let [validation_started, _release_validation] = seed_barrier_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());

    let result = async {
        let daemon =
            daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
        gateway_guard.attach_daemon(daemon);
        timeout(Duration::from_secs(5), async {
            loop {
                if state::fs::path_entry_exists(&validation_started)? {
                    return Ok::<(), anyhow::Error>(());
                }
                sleep(JOB_STATUS_POLL_INTERVAL).await;
            }
        })
        .await??;
        wait_for_job_scope_status(&paths, "system", JobStatus::Running).await?;

        timeout(Duration::from_secs(1), gateway_guard.shutdown_daemon()).await??;

        let job = wait_for_job_scope_status(&paths, "system", JobStatus::Failed).await?;
        assert_eq!(
            job.error.as_deref(),
            Some("reconciliation was abandoned before completion")
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    propagate_after_cleanup(result, cleanup_result)
}

#[tokio::test]
async fn startup_reconciliation_records_non_contention_enqueue_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_user_dir(&paths.jobs_lock())?;
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
    let job = wait_for_job_scope_status(&paths, "system", JobStatus::Failed).await?;

    daemon.shutdown().await?;

    assert_eq!(health_lines[0]["status"], json!("ok"));
    assert_eq!(job.kind, "reconcile");
    assert!(
        job.error
            .as_deref()
            .is_some_and(|error| error.contains(paths.jobs_lock().as_str()))
    );

    Ok(())
}

#[tokio::test]
async fn startup_reconciliation_starts_then_adopts_gateway_across_daemon_restart() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());

    let result = async {
        let daemon =
            daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
        gateway_guard.attach_daemon(daemon);
        wait_for_succeeded_job_id(&paths, "job_000001").await?;
        let initial_pid = state::fs::read_to_string(&paths.gateway_pid())?;

        gateway_guard.shutdown_daemon().await?;

        let daemon =
            daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
        gateway_guard.attach_daemon(daemon);
        wait_for_succeeded_job_id(&paths, "job_000002").await?;
        let adopted_pid = state::fs::read_to_string(&paths.gateway_pid())?;
        let jobs = Database::open(&paths)?.recent_jobs()?;

        Ok::<_, anyhow::Error>((initial_pid, adopted_pid, jobs))
    }
    .await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let (initial_pid, adopted_pid, jobs) = propagate_after_cleanup(result, cleanup_result)?;

    assert_eq!(adopted_pid, initial_pid);
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| {
        job.kind == "reconcile" && job.scope == "system" && job.status == JobStatus::Succeeded
    }));

    Ok(())
}

#[tokio::test]
async fn repeated_system_requests_during_startup_create_one_trailing_job() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let [validation_started, release_validation] = seed_barrier_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());

    let result = async {
        let daemon =
            daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
        gateway_guard.attach_daemon(daemon);
        timeout(Duration::from_secs(5), async {
            loop {
                if state::fs::path_entry_exists(&validation_started)? {
                    return Ok::<(), anyhow::Error>(());
                }
                sleep(JOB_STATUS_POLL_INTERVAL).await;
            }
        })
        .await??;

        let request = serde_json::to_string(&json!({
            "protocol_version": daemon::PROTOCOL_VERSION,
            "command": "run_job",
            "kind": "reconcile",
            "scope": "system",
        }))?;
        let mut readers = Vec::new();
        let mut responses = Vec::new();
        for _ in 0..3 {
            let mut stream = UnixStream::connect(paths.daemon_socket()).await?;
            stream.write_all(request.as_bytes()).await?;
            stream.write_all(b"\n").await?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            responses.push(serde_json::from_str::<Value>(line.trim_end())?);
            readers.push(reader);
        }

        assert!(responses.iter().all(|response| {
            response["job_id"] == "job_000002"
                && response["status"]
                    .as_str()
                    .is_some_and(|status| status == "accepted" || status == "coalesced")
        }));
        state::fs::write_sensitive_file(&release_validation, "release\n")?;

        for mut reader in readers {
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await? == 0 {
                    break;
                }
            }
        }
        wait_for_succeeded_job_count(&paths, "system", 2).await?;

        let jobs = Database::open(&paths)?.recent_jobs()?;
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|job| job.status == JobStatus::Succeeded));

        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    propagate_after_cleanup(result, cleanup_result)
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
    let jobs_lock = JobsLock::acquire(&paths)?;

    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    daemon.shutdown().await?;
    drop(jobs_lock);

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
    let jobs_lock = JobsLock::acquire(&paths)?;
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
    drop(jobs_lock);

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
    let caddy_path = paths.resources().join("caddy/2/releases/2.11.4-pv1");
    let caddy_executable = caddy_path.join("bin/caddy");
    state::fs::write_sensitive_file(&caddy_executable, FAKE_CADDY_SCRIPT)?;
    state::fs::write_sensitive_file(
        &caddy_path.join("bin/caddy.server.py"),
        FAKE_CADDY_SERVER_SCRIPT,
    )?;
    let executable_install =
        AppReleaseLayout::new(paths.clone()).install_release_binary("0.0.0", &caddy_executable)?;
    state::fs::rename(executable_install.binary_path(), &caddy_executable)?;
    state::fs::symlink_file(
        camino::Utf8Path::new("releases/2.11.4-pv1"),
        &paths.resources().join("caddy/2/current"),
    )?;
    let certified_key = generate_simple_self_signed(vec!["pv-gateway.localhost".to_owned()])?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &certified_key.cert.pem())?;
    state::fs::write_sensitive_file(
        &paths.ca_private_key(),
        &certified_key.signing_key.serialize_pem(),
    )?;
    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed("caddy", "2", "2.11.4-pv1", &caddy_path)?;
    drop(database);

    let manifest_client = ScriptedManifestClient::new(CADDY_ARTIFACT_MANIFEST);
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
    let supervisor = daemon::ProcessSupervisor::new(paths.clone());
    if let Some(gateway) =
        supervisor.adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
    {
        gateway.stop(Duration::from_secs(1)).await?;
    }
    daemon.shutdown().await?;

    let database = Database::open(&paths)?;
    let job = wait_for_succeeded_job_id(&paths, &completed.id).await?;
    let caddy_record = database.managed_resource_track("caddy", "2")?;

    assert_eq!(completed.summary, "current");
    assert!(!completed.summary.contains("updated"));
    assert_eq!(job.kind, "update");
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(manifest_request_count(&manifest_requests)?, 1);
    assert_eq!(database.recent_jobs()?.len(), 2);
    assert_with_normalized_timestamps(
        "update_job_refreshes_manifest_without_installed_tracks_and_persists_success",
        (
            completed.summary,
            job.kind,
            job.scope,
            job.status,
            job.summary,
            manifest_request_count(&manifest_requests)?,
            caddy_record.desired_state,
            caddy_record.installed_version,
            caddy_record.current_artifact_path.is_some(),
            database.recent_jobs()?.len(),
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn jobs_lock_rejects_update_jobs_before_manifest_refresh() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let jobs_lock = JobsLock::acquire(&paths)?;
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
    drop(jobs_lock);

    let lines = normalize_lock_path(lines, paths.jobs_lock().as_str(), "<jobs-lock>");
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "jobs_lock_rejects_update_jobs_before_manifest_refresh",
        (
            lines,
            database.recent_jobs()?,
            manifest_request_count(&manifest_requests)?,
        ),
    )?;

    Ok(())
}

fn normalize_lock_path(mut lines: Vec<Value>, lock_path: &str, placeholder: &str) -> Vec<Value> {
    for line in &mut lines {
        let Some(message) = line.get_mut("message") else {
            continue;
        };
        let Some(message_text) = message.as_str() else {
            continue;
        };

        *message = json!(message_text.replace(lock_path, placeholder));
    }

    lines
}

fn seed_foundation_caddy(paths: &PvPaths) -> Result<()> {
    let release_path = paths.home().join("fake-caddy-release");
    let executable = release_path.join("bin/caddy");
    let server_script = camino::Utf8PathBuf::from(format!("{executable}.server.py"));
    state::fs::write_sensitive_file(&executable, FOUNDATION_FAKE_CADDY_SCRIPT)?;
    state::fs::write_sensitive_file(&server_script, FOUNDATION_FAKE_CADDY_SERVER_SCRIPT)?;
    let executable_install =
        AppReleaseLayout::new(paths.clone()).install_release_binary("0.0.0", &executable)?;
    state::fs::rename(executable_install.binary_path(), &executable)?;

    let certified_key = generate_simple_self_signed(vec!["pv-gateway.localhost".to_owned()])?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &certified_key.cert.pem())?;
    state::fs::write_sensitive_file(
        &paths.ca_private_key(),
        &certified_key.signing_key.serialize_pem(),
    )?;

    let mut database = Database::open(paths)?;
    let [http_port, https_port] = available_foundation_gateway_ports()?;
    database.record_managed_resource_track_installed(
        "caddy",
        "2",
        "fake-caddy-pv1",
        &release_path,
    )?;
    database.assign_port(
        PortRequest::gateway(GatewayPort::Http, http_port, http_port, http_port),
        |_port| true,
    )?;
    database.assign_port(
        PortRequest::gateway(GatewayPort::Https, https_port, https_port, https_port),
        |_port| true,
    )?;
    Ok(())
}

fn seed_barrier_foundation_caddy(paths: &PvPaths) -> Result<[Utf8PathBuf; 2]> {
    seed_foundation_caddy(paths)?;
    let executable = paths.home().join("fake-caddy-release/bin/caddy");
    let validation_started = paths.run().join("startup-validation-started");
    let release_validation = paths.run().join("release-startup-validation");
    let wrapper_source = paths.home().join("caddy-startup-barrier");
    let caddy_script = FOUNDATION_FAKE_CADDY_SCRIPT
        .strip_prefix("#!/bin/sh\n")
        .ok_or_else(|| anyhow!("fake Caddy script is missing its shebang"))?;
    state::fs::write_sensitive_file(
        &wrapper_source,
        &format!(
            "#!/bin/sh\nset -eu\nif [ \"${{1:-}}\" = \"validate\" ]; then\n  : > \"{validation_started}\"\n  while [ ! -f \"{release_validation}\" ]; do sleep 0.01; done\nfi\n{caddy_script}"
        ),
    )?;
    let wrapper_install =
        AppReleaseLayout::new(paths.clone()).install_release_binary("0.0.1", &wrapper_source)?;
    state::fs::rename(wrapper_install.binary_path(), &executable)?;

    Ok([validation_started, release_validation])
}

fn available_foundation_gateway_ports() -> Result<[u16; 2]> {
    let mut listeners = Vec::with_capacity(2);
    let mut ports = Vec::with_capacity(2);

    while ports.len() < 2 {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        if ports.contains(&port) {
            continue;
        }

        ports.push(port);
        listeners.push(listener);
    }

    drop(listeners);

    ports
        .try_into()
        .map_err(|_| anyhow!("expected two available gateway ports"))
}

fn reserve_foundation_ports(count: usize, start: u16, end: u16) -> Result<Vec<StdTcpListener>> {
    let mut listeners = Vec::with_capacity(count);

    for port in start..=end {
        match StdTcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Ok(listener) => listeners.push(listener),
            Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error.into()),
        }

        if listeners.len() == count {
            return Ok(listeners);
        }
    }

    Err(anyhow!(
        "expected {count} available foundation ports in {start}..={end}"
    ))
}

struct SeededGatewayGuard {
    paths: PvPaths,
    daemon: Option<daemon::RunningDaemon>,
    worker_track: Option<String>,
    cleanup_complete: bool,
}

impl SeededGatewayGuard {
    fn new(paths: PvPaths) -> Self {
        Self {
            paths,
            daemon: None,
            worker_track: None,
            cleanup_complete: false,
        }
    }

    fn attach_daemon(&mut self, daemon: daemon::RunningDaemon) {
        self.daemon = Some(daemon);
    }

    fn attach_worker(&mut self, track: &str) {
        self.worker_track = Some(track.to_owned());
    }

    async fn shutdown_daemon(&mut self) -> Result<()> {
        let Some(daemon) = self.daemon.take() else {
            return Ok(());
        };

        daemon.shutdown().await.map_err(|error| anyhow!(error))
    }

    async fn shutdown_and_cleanup(&mut self) -> Result<()> {
        let result = shutdown_seeded_gateway(
            self.daemon.take(),
            &self.paths,
            self.worker_track.as_deref(),
        )
        .await;
        if result.is_ok() {
            self.cleanup_complete = true;
        }

        result
    }
}

impl Drop for SeededGatewayGuard {
    fn drop(&mut self) {
        if self.cleanup_complete {
            return;
        }

        let paths = self.paths.clone();
        let diagnostic_paths = paths.clone();
        let daemon = self.daemon.take();
        let worker_track = self.worker_track.clone();
        let cleanup_panicked = std::thread::scope(|scope| {
            let cleanup_thread = scope.spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        report_seeded_gateway_cleanup_failure(
                            &paths,
                            &format!("cleanup runtime construction failed: {error}"),
                        );
                        return;
                    }
                };

                if let Err(error) = runtime.block_on(shutdown_seeded_gateway(
                    daemon,
                    &paths,
                    worker_track.as_deref(),
                )) {
                    report_seeded_gateway_cleanup_failure(
                        &paths,
                        &format!("cleanup failed: {error}"),
                    );
                }
            });
            cleanup_thread.join().is_err()
        });
        if cleanup_panicked {
            report_seeded_gateway_cleanup_failure(&diagnostic_paths, "cleanup thread panicked");
        }
    }
}

fn report_seeded_gateway_cleanup_failure(paths: &PvPaths, message: &str) {
    let record = json!({
        "level": "error",
        "target": "daemon_foundation",
        "event": "seeded_gateway_cleanup_failed",
        "message": message,
    });
    if let Ok(mut log) = state::fs::open_append_file(&paths.daemon_log()) {
        let _write_result = log.write_all(format!("{record}\n").as_bytes());
    }
}

async fn shutdown_seeded_gateway(
    daemon: Option<daemon::RunningDaemon>,
    paths: &PvPaths,
    worker_track: Option<&str>,
) -> Result<()> {
    let shutdown_result = match daemon {
        Some(daemon) => daemon.shutdown().await.map_err(|error| anyhow!(error)),
        None => Ok(()),
    };
    let worker_cleanup_result = stop_seeded_worker(paths, worker_track).await;
    let gateway_cleanup_result = stop_seeded_gateway(paths).await;
    let cleanup_result = match (worker_cleanup_result, gateway_cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(worker_error), Ok(())) => {
            Err(anyhow!("seeded FrankenPHP cleanup failed: {worker_error}"))
        }
        (Ok(()), Err(gateway_error)) => Err(gateway_error),
        (Err(worker_error), Err(gateway_error)) => Err(anyhow!(
            "seeded FrankenPHP cleanup failed: {worker_error}; seeded Caddy cleanup failed: {gateway_error}"
        )),
    };

    match (shutdown_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(shutdown_error), Ok(())) => Err(anyhow!("daemon shutdown failed: {shutdown_error}")),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(shutdown_error), Err(cleanup_error)) => Err(anyhow!(
            "daemon shutdown failed: {shutdown_error}; seeded Caddy cleanup failed: {cleanup_error}"
        )),
    }
}

async fn stop_seeded_worker(paths: &PvPaths, worker_track: Option<&str>) -> Result<()> {
    let Some(worker_track) = worker_track else {
        return Ok(());
    };

    let worker_pid_path = paths.worker_pid(worker_track);
    let worker_metadata_path = paths.worker_runtime_metadata(worker_track);
    let supervisor = daemon::ProcessSupervisor::new(paths.clone());
    let deadline = Instant::now() + SEEDED_GATEWAY_CLEANUP_TIMEOUT;

    loop {
        let has_pid = worker_pid_path.exists();
        let has_metadata = worker_metadata_path.exists();
        if has_pid || has_metadata {
            let Some(worker) =
                supervisor.adopt_recorded(&worker_pid_path, &worker_metadata_path)?
            else {
                if Instant::now() >= deadline {
                    return Err(anyhow!("seeded FrankenPHP runtime was not adoptable"));
                }

                sleep(SEEDED_GATEWAY_CLEANUP_POLL_INTERVAL).await;
                continue;
            };
            worker.stop(Duration::from_secs(1)).await?;

            state::fs::remove_file_if_exists(&worker_pid_path)?;
            state::fs::remove_file_if_exists(&worker_metadata_path)?;
            if worker_pid_path.exists() || worker_metadata_path.exists() {
                return Err(anyhow!(
                    "seeded FrankenPHP runtime files remained after cleanup"
                ));
            }

            return Ok(());
        }

        if Instant::now() >= deadline {
            return Ok(());
        }

        sleep(SEEDED_GATEWAY_CLEANUP_POLL_INTERVAL).await;
    }
}

fn propagate_after_cleanup<T>(
    operation_result: Result<T>,
    cleanup_result: Result<()>,
) -> Result<T> {
    match (operation_result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(operation_error), Ok(())) => Err(operation_error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(operation_error), Err(cleanup_error)) => Err(anyhow!(
            "operation failed: {operation_error}; seeded Caddy cleanup failed: {cleanup_error}"
        )),
    }
}

async fn stop_seeded_gateway(paths: &PvPaths) -> Result<()> {
    let supervisor = daemon::ProcessSupervisor::new(paths.clone());
    let deadline = Instant::now() + SEEDED_GATEWAY_CLEANUP_TIMEOUT;

    loop {
        let has_pid = paths.gateway_pid().exists();
        let has_metadata = paths.gateway_runtime_metadata().exists();
        if has_pid || has_metadata {
            let Some(gateway) = supervisor
                .adopt_recorded(&paths.gateway_pid(), &paths.gateway_runtime_metadata())?
            else {
                if Instant::now() >= deadline {
                    return Err(anyhow!("seeded Caddy runtime was not adoptable"));
                }

                sleep(SEEDED_GATEWAY_CLEANUP_POLL_INTERVAL).await;
                continue;
            };
            gateway.stop(Duration::from_secs(1)).await?;

            state::fs::remove_file_if_exists(&paths.gateway_pid())?;
            state::fs::remove_file_if_exists(&paths.gateway_runtime_metadata())?;
            if paths.gateway_pid().exists() || paths.gateway_runtime_metadata().exists() {
                return Err(anyhow!("seeded Caddy runtime files remained after cleanup"));
            }

            return Ok(());
        }

        if Instant::now() >= deadline {
            return Ok(());
        }

        sleep(SEEDED_GATEWAY_CLEANUP_POLL_INTERVAL).await;
    }
}

#[tokio::test]
async fn blocking_client_submits_reconciliation_jobs() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);
    let client_paths = paths.clone();

    let submitted = tokio::task::spawn_blocking(move || {
        daemon::submit_job_blocking(client_paths, "reconcile", "system")
    })
    .await??;
    let job_result = wait_for_succeeded_job_id(&paths, &submitted.id).await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let job = propagate_after_cleanup(job_result, cleanup_result)?;

    assert_eq!(job.kind, "reconcile");
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Succeeded);

    Ok(())
}

#[tokio::test]
async fn blocking_client_waits_for_reconciliation_stream_completion() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);
    let client_paths = paths.clone();

    let completed_result = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "reconcile", "system")
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(|result| result.map_err(anyhow::Error::from));
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let completed = propagate_after_cleanup(completed_result, cleanup_result)?;
    let job = wait_for_succeeded_job_id(&paths, &completed.id).await?;

    assert_eq!(completed.summary, "Gateway runtime reconciled");
    assert_eq!(job.kind, "reconcile");
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Succeeded);

    Ok(())
}

#[tokio::test]
async fn blocking_client_checks_managed_resource_updates() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let jobs_lock = JobsLock::acquire(&paths)?;
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
    drop(jobs_lock);

    assert!(update_check.managed_resources.is_empty());
    assert_eq!(manifest_request_count(&manifest_requests)?, 1);

    Ok(())
}

#[tokio::test]
async fn system_reconciliation_reconciles_linked_project_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    seed_foundation_caddy(&paths)?;
    let manifest_cache = resources::ArtifactManifestCache::new(paths.downloads());
    let php_track = "8.4";
    let php_release = paths.home().join("8.4-php-release");
    state::fs::write_sensitive_file(&php_release.join("bin/php"), "#!/bin/sh\n")?;
    state::fs::write_sensitive_file(&php_release.join("share/pv/php-extensions.json"), "[]")?;

    let frankenphp_release = paths.home().join("8.4-frankenphp-release");
    let frankenphp_source = paths.home().join("fake-frankenphp-source");
    state::fs::write_sensitive_file(
        &frankenphp_source,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-fixtures/gateway/fake-frankenphp.sh"
        )),
    )?;
    state::fs::write_sensitive_file(
        &frankenphp_release.join("bin/frankenphp.server.py"),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-fixtures/gateway/fake-frankenphp-server.py"
        )),
    )?;
    let frankenphp_install =
        AppReleaseLayout::new(paths.clone()).install_release_binary("0.0.0", &frankenphp_source)?;
    state::fs::rename(
        frankenphp_install.binary_path(),
        &frankenphp_release.join("bin/frankenphp"),
    )?;
    state::fs::write_sensitive_file(
        &frankenphp_release.join("share/pv/php-extensions.json"),
        "[]",
    )?;

    state::fs::write_sensitive_file(manifest_cache.path(), CADDY_ARTIFACT_MANIFEST)?;

    let mut database = Database::open(&paths)?;
    database.record_managed_resource_track_installed(
        "php",
        php_track,
        "8.4.8-pv1",
        &php_release,
    )?;
    database.record_managed_resource_track_installed(
        "frankenphp",
        php_track,
        "8.4.8-pv1",
        &frankenphp_release,
    )?;
    let worker_port_reservations = reserve_foundation_ports(1, 40_000, 44_999)?;
    let worker_service_port = worker_port_reservations[0].local_addr()?.port();
    database.assign_port(
        PortRequest::php_worker(
            php_track,
            worker_service_port,
            worker_service_port,
            worker_service_port,
        ),
        |_port| true,
    )?;
    drop(database);

    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let project_path = tempdir.path().join("project");
    let config_path = project_path.join("pv.yml");
    state::fs::write_sensitive_file(
        &config_path,
        "php: \"8.4\"\nenv:\n  APP_URL: \"${project_url}\"\n  APP_NAME: setup\n",
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

    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);
    gateway_guard.attach_worker(php_track);
    drop(worker_port_reservations);
    let client_paths = paths.clone();
    let completed_result = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "reconcile", "system")
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(|result| result.map_err(anyhow::Error::from));
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    let completed = propagate_after_cleanup(completed_result, cleanup_result)?;

    let database = Database::open(&paths)?;
    let job = wait_for_succeeded_job_id(&paths, &completed.id).await?;
    let project = database
        .projects()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("expected linked project"))?;

    assert_eq!(
        completed.summary,
        "Project env rendered; Gateway runtime reconciled"
    );
    assert_eq!(
        job.summary.as_deref(),
        Some("Project env rendered; Gateway runtime reconciled")
    );
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
    let jobs_lock = JobsLock::acquire(&paths)?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let client_paths = paths.clone();

    let result = tokio::task::spawn_blocking(move || {
        daemon::run_job_blocking(client_paths, "unsupported", "system")
    })
    .await?;
    daemon.shutdown().await?;
    drop(jobs_lock);
    let database = Database::open(&paths)?;
    let jobs = database.recent_jobs()?;

    assert!(matches!(
        result,
        Err(daemon::DaemonError::DaemonRejected { message })
            if message == "unsupported daemon job `unsupported` with scope `system`"
    ));
    let job = jobs
        .iter()
        .find(|job| job.kind == "unsupported")
        .ok_or_else(|| anyhow!("missing unsupported job"))?;
    assert_eq!(job.scope, "system");
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(
        job.error.as_deref(),
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
    let jobs_lock = JobsLock::acquire(&paths)?;
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
    drop(jobs_lock);

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
    let _jobs_lock = JobsLock::acquire(&paths)?;
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
    seed_foundation_caddy(&paths)?;
    let mut gateway_guard = SeededGatewayGuard::new(paths.clone());
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await?;
    gateway_guard.attach_daemon(daemon);

    let request_result = async {
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

        wait_for_succeeded_job_count(&paths, "system", 2).await?;

        Ok::<(), anyhow::Error>(())
    }
    .await;
    let cleanup_result = gateway_guard.shutdown_and_cleanup().await;
    propagate_after_cleanup(request_result, cleanup_result)?;

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
            "INSERT INTO projects (id, project_slug, path, primary_hostname, config_path, created_at, updated_at)
            VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6)",
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
    let preferred_dns_port_blockers = bind_preferred_dns_port_pair().await?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    if preferred_dns_port_blockers.is_none() && port == DNS_PREFERRED_PORT {
        daemon.shutdown().await?;
        return Ok(());
    }

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
    let deadline = Instant::now() + JOB_STATUS_WAIT_TIMEOUT;

    loop {
        let database = Database::open(paths)?;
        if let Some(job) = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.id == id && job.status == JobStatus::Succeeded)
        {
            return Ok(job);
        }

        if Instant::now() >= deadline {
            break;
        }

        sleep(JOB_STATUS_POLL_INTERVAL).await;
    }

    Err(anyhow!("succeeded job with id {id:?} was not recorded"))
}

async fn wait_for_succeeded_job_scope(paths: &PvPaths, scope: &str) -> Result<JobRecord> {
    wait_for_job_scope_status(paths, scope, JobStatus::Succeeded).await
}

async fn wait_for_job_scope_status(
    paths: &PvPaths,
    scope: &str,
    status: JobStatus,
) -> Result<JobRecord> {
    let deadline = Instant::now() + JOB_STATUS_WAIT_TIMEOUT;

    loop {
        let database = Database::open(paths)?;
        if let Some(job) = database
            .recent_jobs()?
            .into_iter()
            .find(|job| job.scope == scope && job.status == status)
        {
            return Ok(job);
        }

        if Instant::now() >= deadline {
            break;
        }

        sleep(JOB_STATUS_POLL_INTERVAL).await;
    }

    Err(anyhow::anyhow!(
        "{status:?} job with scope {scope:?} was not recorded"
    ))
}

async fn wait_for_succeeded_job_count(
    paths: &PvPaths,
    scope: &str,
    expected_count: usize,
) -> Result<()> {
    let deadline = Instant::now() + JOB_STATUS_WAIT_TIMEOUT;

    loop {
        let succeeded_count = Database::open(paths)?
            .recent_jobs()?
            .into_iter()
            .filter(|job| job.scope == scope && job.status == JobStatus::Succeeded)
            .count();
        if succeeded_count >= expected_count {
            return Ok(());
        }

        if Instant::now() >= deadline {
            break;
        }

        sleep(JOB_STATUS_POLL_INTERVAL).await;
    }

    Err(anyhow!(
        "expected {expected_count} succeeded jobs with scope {scope:?}"
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

async fn bind_preferred_dns_port_pair() -> Result<Option<(StdTcpListener, StdUdpSocket)>> {
    for _attempt in 0..100 {
        match bind_loopback_tcp_udp_at(DNS_PREFERRED_PORT) {
            Ok(blockers) => return Ok(Some(blockers)),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }

    if !daemon::dns_port_available(DNS_PREFERRED_PORT) {
        return Ok(None);
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
