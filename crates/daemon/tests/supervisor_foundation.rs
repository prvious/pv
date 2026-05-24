use std::time::Duration;

use anyhow::{Result, anyhow};
use camino_tempfile::tempdir;
use daemon::{
    ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_custom_readiness, wait_for_readiness,
};
use insta::{Settings, assert_debug_snapshot};
use serde_json::json;
use state::PvPaths;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::sleep;

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

fn with_normalized_process_values(assertion: impl FnOnce() -> Result<()>) -> Result<()> {
    Settings::clone_current().bind(assertion)
}
