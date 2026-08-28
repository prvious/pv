use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt as _;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use camino_tempfile::tempdir;
use daemon::{
    CaddyAdminClient, CaddyAdminEndpoint, CaddyAdminError, CaddyAdminTimeouts,
    MAX_RESPONSE_DETAIL_BYTES,
};
use tokio::task::JoinHandle;

#[test]
fn transaction_errors_preserve_ownership_and_both_rollback_failures() -> Result<()> {
    let ownership = CaddyAdminError::runtime_ownership_changed("gateway");
    assert!(matches!(
        ownership,
        CaddyAdminError::RuntimeOwnershipChanged { runtime } if runtime == "gateway"
    ));

    let original_error = CaddyAdminError::AdminReadinessTimedOut {
        endpoint: CaddyAdminEndpoint::new("/tmp/pv-caddy-admin.sock"),
        timeout_ms: 15_000,
        last_error: Some("HTTP status 503".to_owned()),
    };
    let restored_error = CaddyAdminError::LoadRejected {
        endpoint: CaddyAdminEndpoint::new("/tmp/pv-caddy-admin.sock"),
        status: 422,
        detail: "restored config rejected".to_owned(),
    };
    let rollback_error =
        CaddyAdminError::restored_config_reload_failed(original_error, restored_error);

    let CaddyAdminError::RestoredConfigReloadFailed {
        original_error,
        restored_error,
    } = rollback_error
    else {
        bail!("expected a compound restored-config reload error");
    };
    assert!(matches!(
        original_error.as_ref(),
        CaddyAdminError::AdminReadinessTimedOut { .. }
    ));
    assert!(matches!(
        restored_error.as_ref(),
        CaddyAdminError::LoadRejected { .. }
    ));

    Ok(())
}

#[derive(Debug)]
struct RecordedRequest {
    method: String,
    target: String,
    content_type: Option<String>,
    body: Vec<u8>,
}

#[tokio::test]
async fn load_caddyfile_sends_exact_request_and_accepts_success() -> Result<()> {
    let (endpoint, server) = spawn_response_server(vec![http_response(204, b"")])?;
    let body = b"{
    apps {}
}\n";

    test_client().load_caddyfile(&endpoint, body).await?;
    let requests = server.await??;
    let request = requests
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("fixture did not receive a request"))?;

    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/load");
    assert_eq!(request.content_type.as_deref(), Some("text/caddyfile"));
    assert_eq!(request.body, body);

    Ok(())
}

#[tokio::test]
async fn load_caddyfile_maps_rejection_and_caps_response_detail() -> Result<()> {
    let response_body = vec![b'x'; MAX_RESPONSE_DETAIL_BYTES + 128];
    let (endpoint, server) = spawn_response_server(vec![http_response(422, &response_body)])?;

    let result = test_client()
        .load_caddyfile(&endpoint, b"candidate\n")
        .await;
    let requests = server.await??;
    assert_eq!(requests.len(), 1);

    let Err(CaddyAdminError::LoadRejected { status, detail, .. }) = result else {
        bail!("expected a typed load rejection, got {result:?}");
    };
    assert_eq!(status, 422);
    assert_eq!(detail.len(), MAX_RESPONSE_DETAIL_BYTES + 3);
    assert!(detail.ends_with("..."));

    Ok(())
}

#[tokio::test]
async fn load_caddyfile_distinguishes_connection_failure() -> Result<()> {
    let temp_dir = tempdir()?;
    let endpoint = CaddyAdminEndpoint::new(temp_dir.path().join("missing.sock"));

    let result = test_client()
        .load_caddyfile(&endpoint, b"candidate\n")
        .await;

    let Err(CaddyAdminError::RequestNotSent {
        endpoint: actual,
        operation: daemon::CaddyAdminOperation::Load,
        reason,
    }) = result
    else {
        bail!("expected a not-sent endpoint error, got {result:?}");
    };
    assert_eq!(actual, endpoint);
    assert!(matches!(
        reason.as_ref(),
        CaddyAdminError::EndpointUnavailable { endpoint: actual, .. } if *actual == endpoint
    ));

    Ok(())
}

#[tokio::test]
async fn verifier_failure_sends_no_http_request_bytes() -> Result<()> {
    let temp_dir = tempdir()?;
    let listener = UnixListener::bind(temp_dir.path().join("admin.sock"))?;
    let endpoint = CaddyAdminEndpoint::new(temp_dir.path().join("admin.sock"));
    let verifier = Arc::new(|operation| {
        Err(CaddyAdminError::runtime_ownership_changed(format!(
            "{operation} verifier"
        )))
    });

    let result = test_client()
        .load_caddyfile_with(&endpoint, b"candidate\n", verifier)
        .await;

    let Err(CaddyAdminError::RequestNotSent {
        endpoint: actual,
        operation: daemon::CaddyAdminOperation::Load,
        reason,
    }) = result
    else {
        bail!("expected a verifier rejection before contact, got {result:?}");
    };
    assert_eq!(actual, endpoint);
    assert!(matches!(
        reason.as_ref(),
        CaddyAdminError::RuntimeOwnershipChanged { runtime }
            if runtime == "load verifier"
    ));
    let (mut stream, _) = listener.accept()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    assert!(bytes.is_empty());

    Ok(())
}

#[tokio::test]
async fn readiness_verifier_failure_is_not_masked_by_poll_timeout() -> Result<()> {
    let temp_dir = tempdir()?;
    let listener = UnixListener::bind(temp_dir.path().join("admin.sock"))?;
    let endpoint = CaddyAdminEndpoint::new(temp_dir.path().join("admin.sock"));
    let verifier = Arc::new(|operation| {
        Err(CaddyAdminError::runtime_ownership_changed(format!(
            "{operation} verifier"
        )))
    });

    let result = test_client()
        .wait_until_ready_with(&endpoint, Duration::from_millis(500), verifier)
        .await;

    assert!(matches!(
        result,
        Err(CaddyAdminError::RequestNotSent {
            operation: daemon::CaddyAdminOperation::Readiness,
            reason,
            ..
        }) if matches!(reason.as_ref(), CaddyAdminError::RuntimeOwnershipChanged { .. })
    ));
    let (mut stream, _) = listener.accept()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    assert!(bytes.is_empty());

    Ok(())
}

#[tokio::test]
#[expect(
    clippy::disallowed_types,
    reason = "peer-credential fixture needs an unrelated process group"
)]
async fn mismatched_peer_process_group_receives_no_http_request_bytes() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("admin.sock");
    let listener = UnixListener::bind(&socket_path)?;
    let endpoint = CaddyAdminEndpoint::new(socket_path);
    let server = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let _temp_dir = temp_dir;
        let (mut stream, _) = listener.accept()?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes)?;

        Ok(bytes)
    });
    let mut foreign_process = std::process::Command::new("sleep")
        .arg("5")
        .process_group(0)
        .spawn()?;
    let foreign_pid = foreign_process.id();
    let verifier = Arc::new(move |_operation| Ok(Some(foreign_pid)));

    let result = test_client()
        .load_caddyfile_with(&endpoint, b"candidate\n", verifier)
        .await;
    let _kill_result = foreign_process.kill();
    let _wait_result = foreign_process.wait();
    let received = server.await??;

    assert!(matches!(
        result,
        Err(CaddyAdminError::RequestNotSent { reason, .. })
            if matches!(reason.as_ref(), CaddyAdminError::PeerProcessGroupMismatch { .. })
    ));
    assert!(received.is_empty());

    Ok(())
}

#[tokio::test]
async fn load_caddyfile_has_a_bounded_request_timeout() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("admin.sock");
    let listener = UnixListener::bind(&socket_path)?;
    let endpoint = CaddyAdminEndpoint::new(socket_path);
    let server = tokio::task::spawn_blocking(move || -> Result<()> {
        let _temp_dir = temp_dir;
        let (mut stream, _) = listener.accept()?;
        let _request = read_request(&mut stream)?;
        thread::sleep(Duration::from_millis(250));
        let _result = stream.write_all(&http_response(200, b"ok"));

        Ok(())
    });

    let client = CaddyAdminClient::with_timeouts(CaddyAdminTimeouts::new(
        Duration::from_millis(50),
        Duration::from_millis(50),
        Duration::from_millis(50),
        Duration::from_millis(75),
        Duration::from_millis(5),
    ));
    let result = client.load_caddyfile(&endpoint, b"candidate\n").await;

    let Err(CaddyAdminError::RequestOutcomeUnknown {
        endpoint: actual, ..
    }) = result
    else {
        bail!("expected an unknown post-contact request outcome, got {result:?}");
    };
    assert_eq!(actual, endpoint);
    server.await??;

    Ok(())
}

#[tokio::test]
async fn load_caddyfile_maps_an_unclassified_transport_failure_to_unknown() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("admin.sock");
    let listener = UnixListener::bind(&socket_path)?;
    let endpoint = CaddyAdminEndpoint::new(socket_path);
    let server = tokio::task::spawn_blocking(move || -> Result<()> {
        let _temp_dir = temp_dir;
        let (mut stream, _) = listener.accept()?;
        let _request = read_request(&mut stream)?;
        stream.write_all(b"not-an-http-response")?;
        Ok(())
    });

    let result = test_client()
        .load_caddyfile(&endpoint, b"candidate\n")
        .await;
    server.await??;

    assert!(matches!(
        result,
        Err(CaddyAdminError::RequestOutcomeUnknown {
            endpoint: actual,
            operation: daemon::CaddyAdminOperation::Load,
            ..
        }) if actual == endpoint
    ));

    Ok(())
}

#[tokio::test]
async fn wait_until_ready_uses_config_endpoint_until_admin_is_ready() -> Result<()> {
    let (endpoint, server) = spawn_response_server(vec![
        http_response(503, b"starting"),
        http_response(200, b"{}"),
    ])?;

    test_client()
        .wait_until_ready(&endpoint, Duration::from_millis(500))
        .await?;
    let requests = server.await??;

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].target, "/config/");
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].target, "/config/");

    Ok(())
}

#[tokio::test]
async fn wait_until_ready_reports_a_bounded_readiness_timeout() -> Result<()> {
    let (endpoint, server) = spawn_response_server(vec![http_response(503, b"starting")])?;

    let result = test_client()
        .wait_until_ready(&endpoint, Duration::from_millis(80))
        .await;
    let requests = server.await??;
    assert_eq!(requests.len(), 1);

    let Err(CaddyAdminError::AdminReadinessTimedOut {
        endpoint: actual,
        timeout_ms,
        ..
    }) = result
    else {
        bail!("expected a typed readiness timeout, got {result:?}");
    };
    assert_eq!(actual, endpoint);
    assert_eq!(timeout_ms, 80);

    Ok(())
}

fn test_client() -> CaddyAdminClient {
    CaddyAdminClient::with_timeouts(CaddyAdminTimeouts::new(
        Duration::from_millis(50),
        Duration::from_millis(50),
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(5),
    ))
}

fn spawn_response_server(
    responses: Vec<Vec<u8>>,
) -> Result<(CaddyAdminEndpoint, JoinHandle<Result<Vec<RecordedRequest>>>)> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("admin.sock");
    let listener = UnixListener::bind(&socket_path)?;
    let endpoint = CaddyAdminEndpoint::new(socket_path);
    let server = tokio::task::spawn_blocking(move || -> Result<Vec<RecordedRequest>> {
        let _temp_dir = temp_dir;
        let mut requests = Vec::with_capacity(responses.len());

        for response in responses {
            let (mut stream, _) = listener.accept()?;
            let request = read_request(&mut stream)?;
            stream.write_all(&response)?;
            stream.shutdown(Shutdown::Both)?;
            requests.push(request);
        }

        Ok(requests)
    });

    Ok((endpoint, server))
}

fn read_request(stream: &mut UnixStream) -> Result<RecordedRequest> {
    const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;

    let mut request_bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0_u8; 1024];
        let bytes_read = stream.read(&mut buffer)?;
        if bytes_read == 0 {
            bail!("fixture connection closed before request headers");
        }
        request_bytes.extend_from_slice(&buffer[..bytes_read]);

        if let Some(position) = request_bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        {
            break position + 4;
        }
        if request_bytes.len() > MAX_REQUEST_HEADER_BYTES {
            bail!("fixture request headers exceeded the test limit");
        }
    };

    let header = std::str::from_utf8(&request_bytes[..header_end])?;
    let mut lines = header.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("fixture request did not include a request line"))?;
    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts
        .next()
        .ok_or_else(|| anyhow!("fixture request did not include a method"))?
        .to_owned();
    let target = request_line_parts
        .next()
        .ok_or_else(|| anyhow!("fixture request did not include a target"))?
        .to_owned();

    let mut content_length = 0_usize;
    let mut content_type = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse()?;
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.trim().to_owned());
        }
    }

    let mut body = request_bytes[header_end..].to_vec();
    while body.len() < content_length {
        let mut buffer = [0_u8; 1024];
        let bytes_read = stream.read(&mut buffer)?;
        if bytes_read == 0 {
            bail!("fixture connection closed before request body");
        }
        body.extend_from_slice(&buffer[..bytes_read]);
    }
    body.truncate(content_length);

    Ok(RecordedRequest {
        method,
        target,
        content_type,
        body,
    })
}

fn http_response(status: u16, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}
