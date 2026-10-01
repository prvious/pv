//! The `mailpit` persona takes Mailpit's real command line and follows recordings of Mailpit
//! 1.30.1 started by PV. The `pv_fake_mailpit` persona is the program PV's test-only fake Mailpit
//! adapter starts: positional ports and a `/ready` route.

use std::convert::Infallible;
use std::io::{self, ErrorKind, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use camino::Utf8Path;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use crate::events::{EventKind, EventLog};
use crate::{FakeSettings, accept};

const BIND_RETRY_DELAY: Duration = Duration::from_millis(50);
/// Mailpit greets with the machine's hostname; PV never reads it.
const MAILPIT_GREETING: &str = "220 localhost Mailpit ESMTP Service ready\r\n";
const PV_FAKE_MAILPIT_GREETING: &str = "220 fake mailpit\r\n";
/// PV only checks the dashboard's status.
const DASHBOARD_PAGE: &str = "<!DOCTYPE html>\n<html><head><title>Mailpit</title></head></html>\n";

/// Handles `mailpit --smtp <address> --listen <address> --database <path>
/// [--disable-version-check]`. Returns an exit code to exit with now, or `None` once serving.
pub(crate) async fn start(argv: &[String]) -> Result<Option<u8>> {
    let (mut smtp, mut listen, mut database) = (None, None, None);
    let mut arguments = argv.iter().skip(1);
    while let Some(flag) = arguments.next() {
        let slot = match flag.as_str() {
            "--disable-version-check" => continue,
            "--smtp" => &mut smtp,
            "--listen" => &mut listen,
            "--database" => &mut database,
            _ => {
                print_error(&format!("Error: unknown flag: {flag}"));
                return Ok(Some(1));
            }
        };
        let Some(value) = arguments.next() else {
            print_error(&format!("Error: flag needs an argument: {flag}"));
            return Ok(Some(1));
        };
        *slot = Some(value.as_str());
    }
    let (Some(smtp), Some(listen), Some(database)) = (smtp, listen, database) else {
        bail!("expected `mailpit --smtp <address> --listen <address> --database <path>`");
    };
    if let Some(parent) = Utf8Path::new(database).parent()
        && !state::fs::path_is_directory(parent)?
    {
        log_error(&format!("[db] open {database}: no such file or directory"));
        return Ok(Some(1));
    }
    let Some(smtp) = bind(smtp).await? else {
        return Ok(Some(1));
    };
    let Some(dashboard) = bind(listen).await? else {
        return Ok(Some(1));
    };
    tokio::spawn(greet(smtp, MAILPIT_GREETING));
    tokio::spawn(serve_http(dashboard, mailpit_route, None));

    Ok(None)
}

/// Handles `pv-fake-mailpit <smtp port> <dashboard port>`, ignoring further arguments. A busy
/// port is retried until it's free, so a test can release its reservation while PV starts the
/// runtime.
pub(crate) async fn start_pv_fake(
    argv: &[String],
    settings: &FakeSettings,
    events: &EventLog,
) -> Result<Option<u8>> {
    let [_executable, smtp_port, dashboard_port, ..] = argv else {
        bail!("expected `pv-fake-mailpit <smtp port> <dashboard port>`");
    };
    let smtp = bind_retrying(smtp_port).await?;
    let dashboard = bind_retrying(dashboard_port).await?;
    tokio::spawn(greet(smtp, PV_FAKE_MAILPIT_GREETING));
    let exit_after_response = settings
        .exit_after_first_http_response
        .then(|| events.clone());
    tokio::spawn(serve_http(dashboard, pv_fake_route, exit_after_response));

    Ok(None)
}

fn mailpit_route(path: &str) -> Response<Full<Bytes>> {
    match path {
        "/" => response(
            StatusCode::OK,
            Some("text/html; charset=utf-8"),
            DASHBOARD_PAGE,
        ),
        "/readyz" | "/livez" => response(StatusCode::OK, None, ""),
        // Go's default not-found reply.
        _ => response(
            StatusCode::NOT_FOUND,
            Some("text/plain; charset=utf-8"),
            "404 page not found\n",
        ),
    }
}

fn pv_fake_route(path: &str) -> Response<Full<Bytes>> {
    let status = if path == "/ready" {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };

    response(status, None, "")
}

/// Binds `address`, or logs Mailpit's error and returns `None` if it's busy.
async fn bind(address: &str) -> Result<Option<TcpListener>> {
    match TcpListener::bind(address).await {
        Ok(listener) => Ok(Some(listener)),
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            log_error(&format!(
                "listen tcp {address}: bind: address already in use"
            ));
            Ok(None)
        }
        Err(error) => Err(error).with_context(|| format!("binding {address}")),
    }
}

async fn bind_retrying(port: &str) -> Result<TcpListener> {
    let port = port
        .parse::<u16>()
        .with_context(|| format!("port {port}"))?;
    loop {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == ErrorKind::AddrInUse => {
                tokio::time::sleep(BIND_RETRY_DELAY).await;
            }
            Err(error) => return Err(error).with_context(|| format!("binding port {port}")),
        }
    }
}

/// Sends the SMTP greeting, then holds each connection until the client closes it, as Mailpit
/// waits for commands.
async fn greet(listener: TcpListener, greeting: &'static str) {
    loop {
        let mut stream = accept(&listener).await;
        tokio::spawn(async move {
            if stream.write_all(greeting.as_bytes()).await.is_ok() {
                let _copy_result = tokio::io::copy(&mut stream, &mut tokio::io::sink()).await;
            }
        });
    }
}

/// Serves `route`. With `exit_after_response`, the fake exits as soon as hyper has written a
/// response and finished the connection, which it does without waiting for the client to hang up.
async fn serve_http(
    listener: TcpListener,
    route: fn(&str) -> Response<Full<Bytes>>,
    exit_after_response: Option<EventLog>,
) {
    loop {
        let stream = accept(&listener).await;
        let exit_after_response = exit_after_response.clone();
        tokio::spawn(async move {
            let answered = Arc::new(AtomicBool::new(false));
            let service = {
                let (answered, exit_after_response) =
                    (answered.clone(), exit_after_response.clone());
                service_fn(move |request: Request<Incoming>| {
                    // Record the exit before answering, so nothing but `exit` follows the response.
                    if let Some(events) = &exit_after_response {
                        let _record_result = events.record(EventKind::Exit { code: 0 });
                    }
                    answered.store(true, Ordering::Relaxed);
                    let response = route(request.uri().path());
                    async move { Ok::<_, Infallible>(response) }
                })
            };
            let _connection_result = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
            if exit_after_response.is_some() && answered.load(Ordering::Relaxed) {
                std::process::exit(0);
            }
        });
    }
}

fn response(
    status: StatusCode,
    content_type: Option<&'static str>,
    body: &'static str,
) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from_static(body.as_bytes())));
    *response.status_mut() = status;
    if let Some(content_type) = content_type {
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    }

    response
}

fn print_error(line: &str) {
    let _write_result = writeln!(io::stderr(), "{line}");
}

/// Prints `message` as Mailpit logs errors, without the timestamp.
fn log_error(message: &str) {
    print_error(&format!("level=error msg=\"{message}\""));
}
