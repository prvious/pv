use std::time::Duration;

use anyhow::{Result, anyhow};
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
use tokio::time::{Instant, sleep, timeout};

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
    let started_at = Instant::now();

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
    assert!(started_at.elapsed() < Duration::from_millis(200));
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
    let started_at = Instant::now();

    let result = timeout(
        Duration::from_millis(250),
        wait_for_custom_readiness("hanging-custom", Duration::from_millis(30), || {
            std::future::pending::<bool>()
        }),
    )
    .await?;

    assert!(result.is_err());
    assert!(started_at.elapsed() < Duration::from_millis(200));

    Ok(())
}

#[tokio::test]
async fn supervisor_captures_logs_and_runtime_metadata_then_stops_child() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let process = supervisor
        .start(ProcessSpec {
            name: "test-runtime".to_string(),
            command: "/bin/sh".into(),
            arguments: vec![
                "-c".to_string(),
                "printf 'runtime ready\\n'; sleep 30".to_string(),
            ],
            log_path: paths.logs().join("test-runtime.log"),
            pid_path: paths.run().join("test-runtime.pid"),
            metadata_path: paths.run().join("test-runtime.json"),
        })
        .await?;

    let log = wait_for_file_contains(process.log_path(), "runtime ready").await?;
    let mut metadata: serde_json::Value =
        serde_json::from_str(&state::testing::read_to_string(process.metadata_path())?)?;
    let pid = process.pid();
    assert!(pid > 0);
    metadata["pid"] = json!("<pid>");
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
            log_path: paths.logs().join("metadata-failure.log"),
            pid_path: pid_path.clone(),
            metadata_path: metadata_parent_blocker.join("metadata.json"),
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
    let spec = ProcessSpec {
        name: "adoptable-runtime".to_string(),
        command: "/bin/sh".into(),
        arguments: vec!["-c".to_string(), "sleep 30".to_string()],
        log_path: paths.logs().join("adoptable-runtime.log"),
        pid_path: paths.run().join("adoptable-runtime.pid"),
        metadata_path: paths.run().join("adoptable-runtime.json"),
    };
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

fn with_normalized_process_values(assertion: impl FnOnce() -> Result<()>) -> Result<()> {
    Settings::clone_current().bind(assertion)
}
