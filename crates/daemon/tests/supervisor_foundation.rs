use std::time::Duration;

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use daemon::{
    ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_custom_readiness, wait_for_readiness,
};
use insta::{Settings, assert_debug_snapshot};
use rustix::process::{Pid, test_kill_process};
use serde_json::json;
use state::PvPaths;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};

#[tokio::test]
async fn tcp_readiness_succeeds_for_listening_ports_and_times_out() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();

    wait_for_readiness(
        ReadinessCheck::Tcp {
            host: "127.0.0.1".to_string(),
            port,
        },
        Duration::from_secs(1),
    )
    .await?;

    drop(listener);
    let result = wait_for_readiness(
        ReadinessCheck::Tcp {
            host: "127.0.0.1".to_string(),
            port,
        },
        Duration::from_millis(10),
    )
    .await;

    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn readiness_timeout_reports_the_last_probe_failure() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let result: Result<(), std::io::Error> = async {
            loop {
                let (mut stream, _address) = listener.accept().await?;
                let mut request = [0_u8; 1024];
                let _bytes = stream.read(&mut request).await?;
                stream
                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                    .await?;
            }

            #[expect(unreachable_code, reason = "test server runs until aborted")]
            Ok(())
        }
        .await;

        result
    });

    let result = wait_for_readiness(
        ReadinessCheck::Http {
            host: "127.0.0.1".to_string(),
            port,
            path: "/health".to_string(),
        },
        Duration::from_millis(30),
    )
    .await;

    assert!(matches!(
        result,
        Err(daemon::DaemonError::ReadinessTimedOut {
            last_error: Some(reason),
            ..
        }) if reason.contains("HTTP readiness returned non-success status")
    ));
    server.abort();

    Ok(())
}

#[tokio::test]
async fn http_readiness_succeeds_for_successful_responses() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let (mut stream, _address) = listener.accept().await?;
        let mut request = [0_u8; 1024];
        let _bytes = stream.read(&mut request).await?;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await?;

        Ok::<(), std::io::Error>(())
    });

    wait_for_readiness(
        ReadinessCheck::Http {
            host: "127.0.0.1".to_string(),
            port,
            path: "/health".to_string(),
        },
        Duration::from_secs(1),
    )
    .await?;
    server.await??;

    Ok(())
}

#[tokio::test]
async fn http_readiness_times_out_even_when_the_server_keeps_the_socket_open() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let (_stream, _address) = listener.accept().await?;
        sleep(Duration::from_secs(1)).await;

        Ok::<(), std::io::Error>(())
    });

    let result = timeout(
        Duration::from_millis(250),
        wait_for_readiness(
            ReadinessCheck::Http {
                host: "127.0.0.1".to_string(),
                port,
                path: "/health".to_string(),
            },
            Duration::from_millis(30),
        ),
    )
    .await?;

    assert!(result.is_err());
    server.abort();

    Ok(())
}

#[tokio::test]
async fn custom_readiness_retries_until_the_check_succeeds() -> Result<()> {
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let check_attempts = std::sync::Arc::clone(&attempts);

    wait_for_custom_readiness("test-custom", Duration::from_secs(1), move || {
        let attempts = std::sync::Arc::clone(&check_attempts);
        async move { attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 }
    })
    .await?;

    assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 2);

    Ok(())
}

#[tokio::test]
async fn custom_readiness_timeout_bounds_a_hanging_check_future() -> Result<()> {
    let result = timeout(
        Duration::from_millis(250),
        wait_for_custom_readiness("hanging-custom", Duration::from_millis(30), || {
            std::future::pending::<bool>()
        }),
    )
    .await?;

    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn supervisor_captures_logs_and_runtime_metadata_then_stops_child() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let process = supervisor
        .start(process_spec(
            &paths,
            "test-runtime",
            "/bin/sh",
            vec![
                "-c".to_string(),
                "printf 'runtime ready\\n'; sleep 30".to_string(),
            ],
        ))
        .await?;

    let log = wait_for_file_contains(process.log_path(), "runtime ready").await?;
    let mut metadata: serde_json::Value =
        serde_json::from_str(&state::testing::read_to_string(process.metadata_path())?)?;
    let pid = process.pid();
    assert!(pid > 0);
    metadata["pid"] = json!("<pid>");
    metadata["config_path"] = json!("<home>/.pv/config/test-runtime.json");
    metadata["log_path"] = json!("<home>/.pv/logs/test-runtime.log");
    metadata["started_at"] = json!("<timestamp>");

    process.stop(Duration::from_secs(1)).await?;

    with_normalized_process_values(|| {
        assert_debug_snapshot!(("<pid>", log, metadata));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[tokio::test]
async fn supervisor_terminates_child_when_runtime_metadata_persistence_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let metadata_parent_blocker = paths.run().join("metadata-parent");
    let pid_path = paths.run().join("metadata-failure.pid");
    state::fs::write_sensitive_file(&metadata_parent_blocker, "not a directory")?;

    let result = supervisor
        .start(ProcessSpec {
            name: "metadata-failure".to_string(),
            command: "/bin/sh".into(),
            arguments: vec!["-c".to_string(), "sleep 30".to_string()],
            config_path: paths.config().join("metadata-failure.json"),
            log_path: paths.logs().join("metadata-failure.log"),
            pid_path: pid_path.clone(),
            metadata_path: metadata_parent_blocker.join("metadata.json"),
            resource_name: "metadata-failure".to_string(),
            track: "test".to_string(),
        })
        .await;

    assert!(result.is_err());
    let pid = wait_for_file_contains(&pid_path, "\n").await?;
    let pid = pid.trim().parse::<u32>()?;
    wait_for_process_exit(pid).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_verifies_and_adopts_owned_runtime_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let spec = process_spec(
        &paths,
        "adoptable-runtime",
        "/bin/sh",
        vec!["-c".to_string(), "while true; do sleep 1; done".to_string()],
    );
    let process = supervisor.start(spec.clone()).await?;

    let owned = supervisor
        .verify_ownership(&spec)?
        .ok_or_else(|| anyhow!("runtime was not verified as PV-owned"))?;
    let adopted = supervisor
        .adopt(&spec)?
        .ok_or_else(|| anyhow!("runtime was not adopted"))?;

    assert_eq!(owned.pid(), process.pid());
    assert_eq!(adopted.pid(), process.pid());

    process.stop(Duration::from_secs(1)).await?;

    assert!(supervisor.adopt(&spec)?.is_none());

    Ok(())
}

#[tokio::test]
async fn supervisor_sends_reload_signal_to_owned_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let marker = paths.run().join("reload-marker");
    let ready = paths.run().join("reload-ready");
    let spec = process_spec(
        &paths,
        "reloadable-runtime",
        "/bin/sh",
        vec![
            "-c".to_string(),
            format!(
                "trap 'touch \"{marker}\"' USR1; touch \"{ready}\"; while true; do sleep 1; done"
            ),
        ],
    );
    let process = ProcessSupervisor::new(paths.clone())
        .start(spec.clone())
        .await?;

    wait_for_path(&ready).await?;
    ProcessSupervisor::new(paths.clone()).reload(&spec)?;
    wait_for_path(&marker).await?;
    process.stop(Duration::from_secs(1)).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_stop_waits_for_process_group_descendants() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let child_pid_path = paths.run().join("descendant.pid");
    let process = ProcessSupervisor::new(paths.clone())
        .start(process_spec(
            &paths,
            "descendant-runtime",
            "/bin/sh",
            vec![
                "-c".to_string(),
                format!(
                    "trap 'exit 0' TERM; sh -c 'trap \"\" TERM; while true; do sleep 1; done' & echo $! > \"{child_pid_path}\"; while true; do sleep 1; done"
                ),
            ],
        ))
        .await?;
    wait_for_path(&child_pid_path).await?;
    let child_pid = wait_for_file_contains(&child_pid_path, "\n")
        .await?
        .trim()
        .parse::<u32>()?;

    process.stop(Duration::from_millis(50)).await?;

    wait_for_process_exit(child_pid).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_metadata_for_a_reused_pid_with_a_different_command() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let actual = supervisor
        .start(process_spec(
            &paths,
            "actual-runtime",
            "/bin/sh",
            vec!["-c".to_string(), "sleep 30".to_string()],
        ))
        .await?;
    let forged = process_spec(
        &paths,
        "forged-runtime",
        "/bin/echo",
        vec!["not-the-live-process".to_string()],
    );
    state::fs::write_sensitive_file(&forged.pid_path, &format!("{}\n", actual.pid()))?;
    state::fs::write_sensitive_file(
        &forged.metadata_path,
        &serde_json::to_string(&json!({
            "name": "forged-runtime",
            "pid": actual.pid(),
            "command": "/bin/echo",
            "arguments": ["not-the-live-process"],
            "config_path": forged.config_path.as_str(),
            "resource_name": "forged-runtime",
            "track": "test",
            "log_path": forged.log_path.as_str(),
            "started_at": "2026-05-25T00:00:00Z",
        }))?,
    )?;

    assert!(supervisor.verify_ownership(&forged)?.is_none());

    actual.stop(Duration::from_secs(1)).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_reused_pid_when_expected_command_only_appears_in_arguments()
-> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let fake_command = paths.root().join("fake-pv-runtime");
    let actual = supervisor
        .start(process_spec(
            &paths,
            "argument-runtime",
            "/bin/sh",
            vec!["-c".to_string(), format!("sleep 30 # {fake_command}")],
        ))
        .await?;
    let forged = process_spec(
        &paths,
        "forged-argument-runtime",
        fake_command.clone(),
        Vec::new(),
    );
    state::fs::write_sensitive_file(&forged.pid_path, &format!("{}\n", actual.pid()))?;
    state::fs::write_sensitive_file(
        &forged.metadata_path,
        &serde_json::to_string(&json!({
            "name": "forged-argument-runtime",
            "pid": actual.pid(),
            "command": fake_command.as_str(),
            "arguments": [],
            "config_path": forged.config_path.as_str(),
            "resource_name": "forged-argument-runtime",
            "track": "test",
            "log_path": forged.log_path.as_str(),
            "started_at": "2026-05-25T00:00:00Z",
        }))?,
    )?;

    assert!(supervisor.verify_ownership(&forged)?.is_none());

    actual.stop(Duration::from_secs(1)).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_reused_pid_with_same_binary_but_different_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let actual = supervisor
        .start(process_spec(
            &paths,
            "actual-argument-runtime",
            "/bin/sh",
            vec!["-c".to_string(), "while true; do sleep 1; done".to_string()],
        ))
        .await?;
    let forged = process_spec(
        &paths,
        "forged-argument-runtime",
        "/bin/sh",
        vec!["-c".to_string(), "echo wrong config".to_string()],
    );
    state::fs::write_sensitive_file(&forged.pid_path, &format!("{}\n", actual.pid()))?;
    state::fs::write_sensitive_file(
        &forged.metadata_path,
        &serde_json::to_string(&json!({
            "name": "forged-argument-runtime",
            "pid": actual.pid(),
            "command": "/bin/sh",
            "arguments": ["-c", "echo wrong config"],
            "config_path": forged.config_path.as_str(),
            "resource_name": "forged-argument-runtime",
            "track": "test",
            "log_path": forged.log_path.as_str(),
            "started_at": "2026-05-25T00:00:00Z",
        }))?,
    )?;

    assert!(supervisor.verify_ownership(&forged)?.is_none());

    actual.stop(Duration::from_secs(1)).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_reused_pid_with_same_binary_and_argument_prefix() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let expected_config = paths.config().join("Caddyfile");
    let actual_config = paths.config().join("Caddyfile.backup");
    let actual = supervisor
        .start(process_spec(
            &paths,
            "actual-prefix-runtime",
            "/bin/sh",
            vec![
                "-c".to_string(),
                format!("while true; do sleep 1; done # {actual_config}"),
            ],
        ))
        .await?;
    let forged = process_spec(
        &paths,
        "forged-prefix-runtime",
        "/bin/sh",
        vec!["-c".to_string(), expected_config.to_string()],
    );
    state::fs::write_sensitive_file(&forged.pid_path, &format!("{}\n", actual.pid()))?;
    state::fs::write_sensitive_file(
        &forged.metadata_path,
        &serde_json::to_string(&json!({
            "name": "forged-prefix-runtime",
            "pid": actual.pid(),
            "command": "/bin/sh",
            "arguments": ["-c", expected_config.as_str()],
            "config_path": forged.config_path.as_str(),
            "resource_name": "forged-prefix-runtime",
            "track": "test",
            "log_path": forged.log_path.as_str(),
            "started_at": "2026-05-25T00:00:00Z",
        }))?,
    )?;

    assert!(supervisor.verify_ownership(&forged)?.is_none());

    actual.stop(Duration::from_secs(1)).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_reused_pid_with_spaced_argument_prefix() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let runtime = paths.root().join("fake-runtime");
    let expected_config = paths.config().join("Alice Smith/Caddyfile");
    let actual_config = paths.config().join("Alice Smith/Caddyfile.backup");
    state::fs::write_sensitive_file(&runtime, "#!/bin/sh\nwhile true; do sleep 1; done\n")?;
    set_executable(&runtime)?;
    let actual = supervisor
        .start(process_spec(
            &paths,
            "actual-spaced-prefix-runtime",
            runtime.clone(),
            vec![actual_config.to_string()],
        ))
        .await?;
    let forged = process_spec(
        &paths,
        "forged-spaced-prefix-runtime",
        runtime.clone(),
        vec![expected_config.to_string()],
    );
    state::fs::write_sensitive_file(&forged.pid_path, &format!("{}\n", actual.pid()))?;
    state::fs::write_sensitive_file(
        &forged.metadata_path,
        &serde_json::to_string(&json!({
            "name": "forged-spaced-prefix-runtime",
            "pid": actual.pid(),
            "command": runtime.as_str(),
            "arguments": [expected_config.as_str()],
            "config_path": forged.config_path.as_str(),
            "resource_name": "forged-spaced-prefix-runtime",
            "track": "test",
            "log_path": forged.log_path.as_str(),
            "started_at": "2026-05-25T00:00:00Z",
        }))?,
    )?;

    assert!(supervisor.verify_ownership(&forged)?.is_none());

    actual.stop(Duration::from_secs(1)).await?;

    Ok(())
}

async fn wait_for_file_contains(path: &camino::Utf8Path, needle: &str) -> Result<String> {
    for _attempt in 0..50 {
        let content = state::testing::read_to_string(path)?;

        if content.contains(needle) {
            return Ok(content);
        }

        sleep(Duration::from_millis(20)).await;
    }

    Err(anyhow!("file {path} did not contain {needle:?}"))
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture marks fake runtime executable"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

async fn wait_for_process_exit(pid: u32) -> Result<()> {
    let raw_pid = i32::try_from(pid)?;
    let pid = Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))?;

    for _attempt in 0..50 {
        if test_kill_process(pid).is_err() {
            return Ok(());
        }

        sleep(Duration::from_millis(20)).await;
    }

    Err(anyhow!("process {pid:?} was still running"))
}

async fn wait_for_path(path: &camino::Utf8Path) -> Result<()> {
    for _attempt in 0..50 {
        if path.exists() {
            return Ok(());
        }

        sleep(Duration::from_millis(20)).await;
    }

    Err(anyhow!("file {path} did not exist"))
}

fn with_normalized_process_values(assertion: impl FnOnce() -> Result<()>) -> Result<()> {
    Settings::clone_current().bind(assertion)
}

fn process_spec(
    paths: &PvPaths,
    name: &str,
    command: impl Into<Utf8PathBuf>,
    arguments: Vec<String>,
) -> ProcessSpec {
    ProcessSpec {
        name: name.to_string(),
        command: command.into(),
        arguments,
        config_path: paths.config().join(format!("{name}.json")),
        log_path: paths.logs().join(format!("{name}.log")),
        pid_path: paths.run().join(format!("{name}.pid")),
        metadata_path: paths.run().join(format!("{name}.json")),
        resource_name: name.to_string(),
        track: "test".to_string(),
    }
}
