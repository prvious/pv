use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use daemon::gateway::{GatewayPfRoutingState, persisted_gateway_is_ready_with_pf_state_for_test};
use daemon::{
    DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_custom_readiness,
    wait_for_readiness,
};
use insta::{Settings, assert_debug_snapshot};
#[cfg(target_os = "macos")]
use pv_fake::{EventKind, InstalledFake, Persona};
use rustix::process::{Pid, test_kill_process};
use rustls::pki_types::PrivateKeyDer;
use serde_json::json;
use state::PvPaths;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tokio_rustls::TlsAcceptor;

#[expect(
    clippy::disallowed_types,
    reason = "regression tests spawn a nested test process to control inherited env without unsafe mutation"
)]
type TestProcessCommand = std::process::Command;

#[cfg(target_os = "macos")]
const OWNED_PYTHON_RUNTIME_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/supervisor/owned-python-runtime.py"
));
/// Keeps a shell alive until it is stopped, but for 150 s at most, so a test that dies before
/// stopping it leaves nothing running for long. That outlasts the CI profile's 120 s limit on a
/// test, so a stalled test can't pass because its fixture exited on its own.
const IDLE_SHELL_LOOP: &str = "i=0; while [ $i -lt 150 ]; do sleep 1; i=$((i + 1)); done";

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
        Duration::from_secs(1),
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
async fn gateway_https_readiness_accepts_non_success_status_lines() -> Result<()> {
    let tempdir = tempdir()?;
    let ca_certificate_path = tempdir.path().join("ca.pem");
    let certified_key = rcgen::generate_simple_self_signed(vec!["acme.test".to_owned()])?;
    state::fs::write_sensitive_file(&ca_certificate_path, &certified_key.cert.pem())?;
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| anyhow!("TLS protocol configuration failed: {error}"))?
    .with_no_client_auth()
    .with_single_cert(
        vec![certified_key.cert.der().clone()],
        PrivateKeyDer::Pkcs8(certified_key.signing_key.serialize_der().into()),
    )?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let http_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let http_port = http_listener.local_addr()?.port();
    let https_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let https_port = https_listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let result: Result<(), std::io::Error> = async {
            loop {
                let (stream, _address) = https_listener.accept().await?;
                let mut stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(_error) => continue,
                };
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

    wait_for_readiness(
        ReadinessCheck::GatewayHttps {
            http_host: "127.0.0.1".to_owned(),
            http_port,
            https_host: "127.0.0.1".to_owned(),
            https_port,
            server_name: "acme.test".to_owned(),
            ca_certificate_path,
        },
        Duration::from_secs(1),
    )
    .await?;

    server.abort();
    drop(http_listener);

    Ok(())
}

#[tokio::test]
async fn gateway_https_readiness_accepts_tls_handshake_without_app_response() -> Result<()> {
    let tempdir = tempdir()?;
    let ca_certificate_path = tempdir.path().join("ca.pem");
    let certified_key = rcgen::generate_simple_self_signed(vec!["acme.test".to_owned()])?;
    state::fs::write_sensitive_file(&ca_certificate_path, &certified_key.cert.pem())?;
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| anyhow!("TLS protocol configuration failed: {error}"))?
    .with_no_client_auth()
    .with_single_cert(
        vec![certified_key.cert.der().clone()],
        PrivateKeyDer::Pkcs8(certified_key.signing_key.serialize_der().into()),
    )?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let http_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let http_port = http_listener.local_addr()?.port();
    let https_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let https_port = https_listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let (stream, _address) = https_listener.accept().await?;
        let _stream = acceptor.accept(stream).await?;
        sleep(Duration::from_secs(1)).await;

        Ok::<(), std::io::Error>(())
    });

    wait_for_readiness(
        ReadinessCheck::GatewayHttps {
            http_host: "127.0.0.1".to_owned(),
            http_port,
            https_host: "127.0.0.1".to_owned(),
            https_port,
            server_name: "acme.test".to_owned(),
            ca_certificate_path,
        },
        Duration::from_millis(100),
    )
    .await?;

    server.abort();
    drop(http_listener);

    Ok(())
}

#[tokio::test]
async fn inactive_pf_does_not_make_gateway_identity_readiness_unhealthy() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let ca_certificate_path = paths.ca_certificate();
    let certified_key =
        rcgen::generate_simple_self_signed(vec!["pv-gateway.localhost".to_owned()])?;
    state::fs::write_sensitive_file(&ca_certificate_path, &certified_key.cert.pem())?;
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| anyhow!("TLS protocol configuration failed: {error}"))?
    .with_no_client_auth()
    .with_single_cert(
        vec![certified_key.cert.der().clone()],
        PrivateKeyDer::Pkcs8(certified_key.signing_key.serialize_der().into()),
    )?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let http_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let http_port = http_listener.local_addr()?.port();
    let https_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let https_port = https_listener.local_addr()?.port();
    let expected_body = format!("pv-gateway-health-v1:{http_port}:{https_port}");
    let http_body = expected_body.clone();
    let http_server = tokio::spawn(async move {
        let (mut stream, _address) = http_listener.accept().await?;
        write_gateway_identity_response(&mut stream, &http_body).await
    });
    let https_server = tokio::spawn(async move {
        let (stream, _address) = https_listener.accept().await?;
        let mut stream = acceptor.accept(stream).await?;

        write_gateway_identity_response(&mut stream, &expected_body).await
    });

    assert!(
        persisted_gateway_is_ready_with_pf_state_for_test(
            &paths,
            http_port,
            https_port,
            GatewayPfRoutingState::Inactive,
        )
        .await?
    );
    http_server.await??;
    https_server.await??;

    Ok(())
}

#[tokio::test]
async fn gateway_identity_readiness_preserves_probe_error() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let http_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let http_port = http_listener.local_addr()?.port();
    let https_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let https_port = https_listener.local_addr()?.port();
    let http_server = tokio::spawn(async move {
        let (mut stream, _address) = http_listener.accept().await?;
        write_gateway_identity_response(&mut stream, "wrong identity").await
    });

    let result = persisted_gateway_is_ready_with_pf_state_for_test(
        &paths,
        http_port,
        https_port,
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::Io(error))
            if error.to_string() == "Gateway identity readiness returned an unexpected response"
    ));
    http_server.await??;
    drop(https_listener);

    Ok(())
}

#[tokio::test]
async fn gateway_identity_readiness_preserves_timeout_diagnostic() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let http_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let http_port = http_listener.local_addr()?.port();
    let https_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let https_port = https_listener.local_addr()?.port();

    let result = persisted_gateway_is_ready_with_pf_state_for_test(
        &paths,
        http_port,
        https_port,
        GatewayPfRoutingState::Inactive,
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::ReadinessTimedOut {
            timeout_ms: 1_000,
            last_error: Some(error),
            ..
        }) if error == "deadline has elapsed"
    ));

    Ok(())
}

#[tokio::test]
async fn gateway_identity_readiness_rejects_generic_tcp_listeners() -> Result<()> {
    let tempdir = tempdir()?;
    let ca_certificate_path = tempdir.path().join("ca.pem");
    let http_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let http_port = http_listener.local_addr()?.port();
    let https_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let https_port = https_listener.local_addr()?.port();

    let result = wait_for_readiness(
        ReadinessCheck::GatewayIdentity {
            http_host: "127.0.0.1".to_owned(),
            http_port,
            https_host: "127.0.0.1".to_owned(),
            https_port,
            server_name: "pv-gateway.localhost".to_owned(),
            path: "/__pv/health".to_owned(),
            expected_body: "pv-gateway-health-v1:48080:48443".to_owned(),
            ca_certificate_path,
        },
        Duration::from_millis(50),
    )
    .await;

    assert!(result.is_err());
    drop(http_listener);
    drop(https_listener);

    Ok(())
}

async fn write_gateway_identity_response<Stream>(
    stream: &mut Stream,
    body: &str,
) -> Result<(), std::io::Error>
where
    Stream: AsyncRead + AsyncWrite + Unpin,
{
    let mut request = [0_u8; 1024];
    let _bytes = stream.read(&mut request).await?;
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    stream.shutdown().await
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
async fn readiness_retries_after_one_probe_hangs() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let (_hanging_stream, _address) = listener.accept().await?;
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
        Duration::from_secs(2),
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
    pv_fake::install_monitor(&paths)?;
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
    metadata["monitor_instance"] = json!("<monitor-instance>");
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

#[test]
fn process_spec_debug_omits_empty_private_environment_and_redacts_values() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let empty_private_env = process_spec(
        &paths,
        "empty-private-env",
        "/bin/sh",
        vec!["-c".to_string(), "sleep 30".to_string()],
    );
    let mut private_env = process_spec(
        &paths,
        "private-env",
        "/bin/sh",
        vec!["-c".to_string(), "sleep 30".to_string()],
    );
    private_env.private_environment = BTreeMap::from([
        ("RUSTFS_ACCESS_KEY".to_string(), "pv-rustfs".to_string()),
        (
            "RUSTFS_SECRET_KEY".to_string(),
            "raw-secret-value".to_string(),
        ),
    ]);

    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(
            "process_spec_debug_omits_empty_private_environment_and_redacts_values",
            (empty_private_env, private_env)
        );
    });

    Ok(())
}

#[tokio::test]
async fn supervisor_stops_its_runtime_when_runtime_metadata_persistence_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let metadata_parent_blocker = paths.run().join("metadata-parent");
    let pid_path = paths.run().join("metadata-failure.pid");
    state::fs::write_sensitive_file(&metadata_parent_blocker, "not a directory")?;

    let result = supervisor
        .start(ProcessSpec {
            name: "metadata-failure".to_string(),
            command: "/bin/sh".into(),
            arguments: vec!["-c".to_string(), "sleep 30".to_string()],
            private_environment: Default::default(),
            config_path: paths.config().join("metadata-failure.json"),
            config_fingerprint: None,
            log_path: paths.logs().join("metadata-failure.log"),
            pid_path: pid_path.clone(),
            metadata_path: metadata_parent_blocker.join("metadata.json"),
            resource_name: "metadata-failure".to_string(),
            track: "test".to_string(),
        })
        .await;

    assert!(result.is_err());
    // A monitor is released only once its runtime's whole process group is proven gone.
    assert!(daemon::recorded_monitor_subjects(&paths)?.is_empty());

    Ok(())
}

#[tokio::test]
async fn supervisor_verifies_and_adopts_owned_runtime_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let spec = process_spec(
        &paths,
        "adoptable-runtime",
        "/bin/sleep",
        vec!["30".to_string()],
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
    assert!(supervisor.record_applied_config(&spec, "sha256:v1:applied")?);
    let applied = supervisor
        .verify_ownership(&spec)?
        .ok_or_else(|| anyhow!("runtime with applied config lost ownership"))?;
    assert_eq!(
        applied.applied_config_fingerprint(),
        Some("sha256:v1:applied")
    );
    let mut invalid_metadata = runtime_metadata(process.metadata_path())?;
    invalid_metadata["staged_config_fingerprint"] = json!("sha256:v1:staged");
    state::fs::write_sensitive_file(
        process.metadata_path(),
        &serde_json::to_string(&invalid_metadata)?,
    )?;
    let invalid = supervisor
        .verify_ownership(&spec)?
        .ok_or_else(|| anyhow!("runtime with invalid config state lost ownership"))?;
    assert!(invalid.applied_config_fingerprint().is_none());
    assert!(supervisor.record_applied_config(&spec, "sha256:v1:applied")?);
    assert!(supervisor.mark_replacement_required(&spec, "sha256:v1:staged")?);
    let pending_metadata = runtime_metadata(process.metadata_path())?;
    assert_eq!(pending_metadata["replacement_required"], true);
    assert!(pending_metadata["applied_config_fingerprint"].is_null());
    assert_eq!(
        pending_metadata["staged_config_fingerprint"],
        "sha256:v1:staged"
    );
    assert_eq!(
        pending_metadata["desired_config_fingerprint"],
        "sha256:v1:staged"
    );
    let replacement = supervisor
        .verify_ownership(&spec)?
        .ok_or_else(|| anyhow!("replacement-required runtime lost ownership"))?;
    assert!(replacement.replacement_required());
    assert!(supervisor.adopt(&spec)?.is_some());
    let replacement = supervisor
        .adopt_recorded(&spec.pid_path, &spec.metadata_path)?
        .ok_or_else(|| anyhow!("replacement-required runtime was not adoptable by its record"))?;
    assert_eq!(replacement.pid(), process.pid());
    assert!(supervisor.clear_replacement_required(&spec)?);
    let cleared_metadata = runtime_metadata(process.metadata_path())?;
    assert!(cleared_metadata["staged_config_fingerprint"].is_null());
    assert_eq!(
        cleared_metadata["desired_config_fingerprint"],
        "sha256:v1:staged"
    );
    assert!(
        !supervisor
            .verify_ownership(&spec)?
            .ok_or_else(|| anyhow!("cleared runtime lost ownership"))?
            .replacement_required()
    );

    process.stop(Duration::from_secs(1)).await?;

    assert!(supervisor.adopt(&spec)?.is_none());

    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn supervisor_verifies_owned_python_shebang_script() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let runtime = paths.run().join("owned-python-runtime");
    state::fs::write_sensitive_file(&runtime, OWNED_PYTHON_RUNTIME_SCRIPT)?;
    set_executable(&runtime)?;

    let supervisor = ProcessSupervisor::new(paths.clone());
    let spec = process_spec(
        &paths,
        "owned-python-runtime",
        runtime,
        vec!["1025".to_string(), "8025".to_string()],
    );
    let process = supervisor.start(spec.clone()).await?;
    let pid = process.pid();
    let ownership = timeout(Duration::from_secs(1), async {
        loop {
            if let Some(owned) = supervisor.verify_ownership(&spec)? {
                return Ok::<_, daemon::DaemonError>(owned);
            }

            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    process.stop(Duration::from_secs(1)).await?;
    let owned = ownership??;

    assert_eq!(owned.pid(), pid);

    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn supervisor_owns_pv_fake_by_direct_identity_with_an_armed_lifeline() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let fake = pv_fake::install(
        &paths.root().join("fake-release/bin/mysqld"),
        Persona::LongRunning,
    )?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let spec = process_spec(&paths, "pv-fake-runtime", fake.executable(), Vec::new());
    let process = supervisor.start(spec.clone()).await?;
    let lifeline_armed = wait_for_fake_start(&fake).await;
    let owned = supervisor.verify_ownership(&spec);
    let metadata = runtime_metadata(&spec.metadata_path);
    process.stop(Duration::from_secs(1)).await?;

    assert!(lifeline_armed?);
    assert!(owned?.is_some());
    // Script runtimes record an executable identity; a directly matched binary does not.
    assert_eq!(metadata?.get("process_executable_identity"), None);

    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_owned_runtime_when_private_environment_changes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let mut spec = process_spec(
        &paths,
        "private-env-runtime",
        "/bin/sleep",
        vec!["30".to_string()],
    );
    spec.private_environment = BTreeMap::from([(
        "RUSTFS_SECRET_KEY".to_string(),
        "initial-secret".to_string(),
    )]);
    let process = supervisor.start(spec.clone()).await?;
    let mut changed_spec = spec.clone();
    changed_spec.private_environment = BTreeMap::from([(
        "RUSTFS_SECRET_KEY".to_string(),
        "changed-secret".to_string(),
    )]);

    assert!(supervisor.verify_ownership(&spec)?.is_some());
    assert!(supervisor.verify_ownership(&changed_spec)?.is_none());

    process.stop(Duration::from_secs(1)).await?;

    Ok(())
}

#[tokio::test]
async fn supervisor_start_strips_parent_php_ini_env_when_private_env_omits_it() -> Result<()> {
    let tempdir = tempdir()?;
    let output = run_ignored_test_with_parent_php_ini_env(
        "supervisor_start_strips_parent_php_ini_env_inner",
        tempdir.path(),
    )?;

    assert_nested_test_succeeded(output)
}

#[tokio::test]
#[ignore]
async fn supervisor_start_strips_parent_php_ini_env_inner() -> Result<()> {
    let root = Utf8Path::new(".");
    let paths = PvPaths::for_home(root.join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let runtime = root.join("env-runtime");
    let ready = root.join("runtime-ready");
    let observed_phprc = root.join("observed-phprc");
    let observed_scan_dir = root.join("observed-scan-dir");
    state::fs::write_sensitive_file(
        &runtime,
        &format!(
            r#"#!/bin/sh
set -eu
printf '%s' "${{PHPRC-}}" > {}
printf '%s' "${{PHP_INI_SCAN_DIR-}}" > {}
touch {}
{IDLE_SHELL_LOOP}
"#,
            shell_single_quoted(observed_phprc.as_str()),
            shell_single_quoted(observed_scan_dir.as_str()),
            shell_single_quoted(ready.as_str()),
        ),
    )?;
    set_executable(&runtime)?;
    let spec = process_spec(&paths, "env-runtime", runtime.clone(), Vec::new());
    let process = ProcessSupervisor::new(paths).start(spec).await?;

    wait_for_path(&ready).await?;
    let phprc = state::testing::read_to_string(&observed_phprc)?;
    let scan_dir = state::testing::read_to_string(&observed_scan_dir)?;
    process.stop(Duration::from_secs(1)).await?;

    assert_eq!(phprc, "");
    assert_eq!(scan_dir, "");

    Ok(())
}

#[tokio::test]
async fn supervisor_stop_waits_for_process_group_descendants() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let child_pid_path = paths.run().join("descendant.pid");
    let process = ProcessSupervisor::new(paths.clone())
        .start(process_spec(
            &paths,
            "descendant-runtime",
            "/bin/sh",
            vec![
                "-c".to_string(),
                format!(
                    "trap 'exit 0' TERM; sh -c 'trap \"\" TERM; {IDLE_SHELL_LOOP}' & echo $! > \"{child_pid_path}\"; {IDLE_SHELL_LOOP}"
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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn adopted_stop_accepts_runtime_already_stopped_by_its_owner() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    pv_fake::install_monitor(&paths)?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let spec = process_spec(
        &paths,
        "already-stopped-runtime",
        "/bin/sleep",
        vec!["30".to_owned()],
    );
    let process = supervisor.start(spec.clone()).await?;
    let adopted = supervisor
        .adopt_recorded(&spec.pid_path, &spec.metadata_path)?
        .ok_or_else(|| anyhow!("runtime was not adoptable before owner cleanup"))?;

    process.stop(Duration::from_secs(1)).await?;
    adopted.stop(Duration::from_secs(1)).await?;

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

/// Waits for the fake's `started` event and returns whether its lifeline is armed.
#[cfg(target_os = "macos")]
async fn wait_for_fake_start(fake: &InstalledFake) -> Result<bool> {
    for _attempt in 0..50 {
        let started = fake
            .events()?
            .into_iter()
            .find_map(|event| match event.kind {
                EventKind::Started { lifeline_armed, .. } => Some(lifeline_armed),
                _ => None,
            });
        if let Some(lifeline_armed) = started {
            return Ok(lifeline_armed);
        }

        sleep(Duration::from_millis(20)).await;
    }

    Err(anyhow!(
        "fake {} did not record a started event",
        fake.executable()
    ))
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

fn runtime_metadata(path: &Utf8Path) -> Result<serde_json::Value> {
    Ok(serde_json::from_str(&state::testing::read_to_string(
        path,
    )?)?)
}

fn with_normalized_process_values(assertion: impl FnOnce() -> Result<()>) -> Result<()> {
    Settings::clone_current().bind(assertion)
}

fn run_ignored_test_with_parent_php_ini_env(
    test_name: &str,
    working_dir: &Utf8Path,
) -> Result<Output> {
    let mut command = TestProcessCommand::new(current_test_binary()?);
    command
        .args(["--exact", test_name, "--ignored", "--nocapture"])
        .current_dir(working_dir)
        .env("PHPRC", "parent-phprc")
        .env("PHP_INI_SCAN_DIR", "parent-scan-dir");

    Ok(command.output()?)
}

fn current_test_binary() -> Result<OsString> {
    std::env::args_os()
        .next()
        .ok_or_else(|| anyhow!("test binary path was missing"))
}

fn assert_nested_test_succeeded(output: Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    anyhow::bail!(
        "nested test failed: status={}; stdout={}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
        private_environment: Default::default(),
        config_path: paths.config().join(format!("{name}.json")),
        config_fingerprint: None,
        log_path: paths.logs().join(format!("{name}.log")),
        pid_path: paths.run().join(format!("{name}.pid")),
        metadata_path: paths.run().join(format!("{name}.json")),
        resource_name: name.to_string(),
        track: "test".to_string(),
    }
}

fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
