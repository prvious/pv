//! The `caddy` and `frankenphp` personas: the parts of Caddy's CLI, admin API, and HTTP(S)
//! serving that PV uses. Behavior follows recordings of Caddy 2.11.4; FrankenPHP embeds Caddy, so
//! both personas behave the same.

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use camino::{Utf8Path, Utf8PathBuf};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rcgen::{CertificateParams, DistinguishedName, Issuer, KeyPair};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde_json::json;
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

use crate::events::{EventKind, EventLog};

mod control;

pub use control::write_gateway_control;
use control::{AdminControl, LoadControl, Records, record_validation};

const GATE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(10);
const EXIT_AFTER_LOAD_DELAY: Duration = Duration::from_millis(50);
/// Caddy's adapter warning for every config PV renders, which indents with spaces.
const UNFORMATTED_WARNING: &str = r#"[{"file":"Caddyfile","line":2,"message":"Caddyfile input is not formatted; run 'caddy fmt --overwrite' to fix inconsistencies"}]"#;

/// Handles PV's `caddy`/`frankenphp` command lines: `<subcommand> --config <path> --adapter
/// caddyfile`. Returns an exit code to exit with now, or `None` once `run` is serving.
pub(crate) async fn start(argv: &[String], events: &EventLog) -> Result<Option<u8>> {
    let subcommand = argv.get(1).map(String::as_str);
    let Some(config_path) = option_value(argv, "--config") else {
        bail!(
            "expected `{}` arguments: --config <path> --adapter caddyfile",
            subcommand.unwrap_or("run")
        );
    };
    let config_path = Utf8PathBuf::from(config_path);

    match subcommand {
        Some("validate") => Ok(Some(validate(&config_path)?)),
        Some("run") => {
            serve(&config_path, events).await?;
            Ok(None)
        }
        _ => {
            let _write_result = writeln!(io::stderr(), "pv-fake: unsupported command {argv:?}");
            Ok(Some(2))
        }
    }
}

fn option_value<'a>(argv: &'a [String], name: &str) -> Option<&'a str> {
    argv.iter()
        .position(|argument| argument == name)
        .and_then(|index| argv.get(index + 1))
        .map(String::as_str)
}

/// Accepts any existing config, as the Python fixtures did, and records it in
/// `fake-validator-spawns.log`; tests that need a rejected config install a validator scenario
/// instead.
fn validate(config_path: &Utf8Path) -> Result<u8> {
    if state::fs::path_is_file(config_path)? {
        record_validation(config_path)?;
        return Ok(0);
    }
    // Caddy's message for a missing config file.
    let _write_result = writeln!(
        io::stderr(),
        "Error: reading config from file: open {config_path}: no such file or directory"
    );

    Ok(1)
}

async fn serve(config_path: &Utf8Path, events: &EventLog) -> Result<()> {
    let source = state::fs::read_to_string(config_path)?;
    let config = GatewayConfig::parse(&source)?;
    let plan = config.listener_plan()?;
    let admin_socket = config
        .admin_socket
        .clone()
        .ok_or_else(|| anyhow!("{config_path} has no `admin \"unix/<path>|0600\"` setting"))?;
    let runtime = Arc::new(Runtime {
        records: Records::beside(config_path)?,
        events: events.clone(),
        readiness_gate: Utf8PathBuf::from(format!("{config_path}.readiness-gate")),
        readiness_probed: Utf8PathBuf::from(format!("{config_path}.readiness-probed")),
        readiness_failure: Utf8PathBuf::from(format!("{config_path}.readiness-fail")),
        health_body: Mutex::new(None),
        listeners: tokio::sync::Mutex::new(Listeners::default()),
    });

    runtime
        .apply(config, &plan, &source, false)
        .await
        .map_err(|error| anyhow!("loading initial config: {error}"))?;
    let admin = UnixListener::bind(admin_socket.as_std_path())
        .with_context(|| format!("binding admin socket {admin_socket}"))?;
    state::fs::secure_sensitive_file(&admin_socket)?;
    tokio::spawn(runtime.accept_admin(admin));

    Ok(())
}

/// The Caddyfile settings PV renders, read line by line.
#[derive(Debug, Default)]
struct GatewayConfig {
    admin_socket: Option<Utf8PathBuf>,
    http_port: Option<u16>,
    https_port: Option<u16>,
    ca_certificate: Option<Utf8PathBuf>,
    ca_private_key: Option<Utf8PathBuf>,
    health_body: Option<String>,
    import_glob: Option<Utf8PathBuf>,
}

impl GatewayConfig {
    fn parse(source: &str) -> Result<Self> {
        let mut config = Self::default();
        for line in source.lines().map(str::trim) {
            if let Some(path) =
                quoted_after(line, "admin \"unix/").and_then(|value| value.strip_suffix("|0600"))
            {
                config.admin_socket = Some(Utf8PathBuf::from(path));
            } else if let Some(port) = line.strip_prefix("http_port ") {
                config.http_port = Some(port.parse().with_context(|| format!("http_port {port}"))?);
            } else if let Some(port) = line.strip_prefix("https_port ") {
                config.https_port =
                    Some(port.parse().with_context(|| format!("https_port {port}"))?);
            } else if let Some(path) = quoted_after(line, "cert \"") {
                config.ca_certificate = Some(Utf8PathBuf::from(path));
            } else if let Some(path) = quoted_after(line, "key \"") {
                config.ca_private_key = Some(Utf8PathBuf::from(path));
            } else if let Some(body) = quoted_after(line, "respond /__pv/health \"") {
                config.health_body = Some(body.to_owned());
            } else if let Some(glob) = quoted_after(line, "import \"") {
                config.import_glob = Some(Utf8PathBuf::from(glob));
            }
        }

        Ok(config)
    }

    /// The listeners this config asks for. Imported fragments are read now, as Caddy's adapter
    /// reads them on every load, so a worker whose root config is unchanged still moves to a
    /// fragment's new port.
    fn listener_plan(&self) -> Result<ListenerPlan> {
        let http = match self.http_port {
            Some(port) => Some(port),
            None => self.first_imported_site_port()?,
        };
        let https = match (self.https_port, &self.ca_certificate, &self.ca_private_key) {
            (Some(port), Some(certificate), Some(private_key)) => Some(HttpsPlan {
                port,
                certificate: certificate.clone(),
                private_key: private_key.clone(),
            }),
            _ => None,
        };

        Ok(ListenerPlan { http, https })
    }

    /// The port of the first `http://host:port` site among the imported fragments, like a
    /// FrankenPHP worker whose root config only imports project sites.
    fn first_imported_site_port(&self) -> Result<Option<u16>> {
        let Some(glob) = &self.import_glob else {
            return Ok(None);
        };
        let (Some(directory), Some((prefix, suffix))) = (
            glob.parent(),
            glob.file_name().and_then(|name| name.split_once('*')),
        ) else {
            return Ok(None);
        };
        if !state::fs::path_is_directory(directory)? {
            return Ok(None);
        }
        let mut fragments = state::fs::read_dir_paths(directory)?;
        fragments.sort();
        for fragment in fragments {
            let matches = fragment
                .file_name()
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(suffix));
            if matches
                && let Some(port) = first_http_site_port(&state::fs::read_to_string(&fragment)?)
            {
                return Ok(Some(port));
            }
        }

        Ok(None)
    }
}

/// Returns the text between `prefix` and the next double quote.
fn quoted_after<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(prefix)?;
    rest.split_once('"').map(|(value, _rest)| value)
}

fn first_http_site_port(fragment: &str) -> Option<u16> {
    fragment.split("http://").skip(1).find_map(|site| {
        let address = site
            .split(|character: char| character.is_whitespace() || character == ',')
            .next()?;
        address.rsplit_once(':')?.1.parse().ok()
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ListenerPlan {
    http: Option<u16>,
    https: Option<HttpsPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpsPlan {
    port: u16,
    certificate: Utf8PathBuf,
    private_key: Utf8PathBuf,
}

/// The HTTP and HTTPS listeners a runtime is serving.
#[derive(Default)]
struct Listeners {
    http: Option<Serving<u16>>,
    https: Option<Serving<HttpsPlan>>,
}

/// A listener's plan and the task accepting its connections, which owns the socket.
struct Serving<P> {
    plan: P,
    task: JoinHandle<()>,
}

impl<P> Serving<P> {
    /// Returns once the socket is closed, as Caddy's old listeners are by the time `/load`
    /// responds.
    async fn stop(self) {
        self.task.abort();
        let _cancelled = self.task.await;
    }
}

/// A listener slot after a reload: unchanged, or replaced by a new listener or by none.
enum Slot<T> {
    Unchanged,
    Replace(Option<T>),
}

struct Runtime {
    records: Records,
    events: EventLog,
    readiness_gate: Utf8PathBuf,
    readiness_probed: Utf8PathBuf,
    readiness_failure: Utf8PathBuf,
    /// Served only when the config has a `respond /__pv/health` line, as with real Caddy.
    health_body: Mutex<Option<String>>,
    /// Held while switching listeners, so loads apply one at a time.
    listeners: tokio::sync::Mutex<Listeners>,
}

impl Runtime {
    /// Serves `config` as Caddy does on a load: binds the new ports first, keeps listeners whose
    /// settings didn't change, and leaves everything as it was if a new port can't be bound.
    /// Returns Caddy's error message on failure.
    async fn apply(
        self: &Arc<Self>,
        config: GatewayConfig,
        plan: &ListenerPlan,
        source: &str,
        retain_listeners: bool,
    ) -> Result<(), String> {
        let mut listeners = self.listeners.lock().await;
        let (http, https) = if retain_listeners {
            (Slot::Unchanged, Slot::Unchanged)
        } else {
            let http = match (plan.http, &listeners.http) {
                (port, Some(serving)) if port == Some(serving.plan) => Slot::Unchanged,
                (None, None) => Slot::Unchanged,
                (port, _) => Slot::Replace(
                    port.map(|port| bind(port).map(|listener| (port, listener)))
                        .transpose()?,
                ),
            };
            let https = match (&plan.https, &listeners.https) {
                (Some(https), Some(serving)) if *https == serving.plan => Slot::Unchanged,
                (None, None) => Slot::Unchanged,
                (https, _) => Slot::Replace(
                    https
                        .as_ref()
                        .map(|https| {
                            let acceptor = tls_acceptor(https)
                                .map_err(|error| format!("loading config: {error:#}"))?;
                            Ok::<_, String>((https.clone(), bind(https.port)?, acceptor))
                        })
                        .transpose()?,
                ),
            };
            (http, https)
        };

        // Everything the config needs is ready; switch over.
        *self
            .health_body
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = config.health_body;
        if let Slot::Replace(next) = http {
            if let Some(serving) = listeners.http.take() {
                serving.stop().await;
            }
            listeners.http = next.map(|(port, listener)| Serving {
                plan: port,
                task: tokio::spawn(self.clone().accept_http(listener)),
            });
        }
        if let Slot::Replace(next) = https {
            if let Some(serving) = listeners.https.take() {
                serving.stop().await;
            }
            listeners.https = next.map(|(plan, listener, acceptor)| Serving {
                plan,
                task: tokio::spawn(self.clone().accept_https(listener, acceptor)),
            });
        }
        self.records
            .current(source)
            .map_err(|error| format!("pv-fake: recording the applied config: {error:#}"))
    }

    async fn accept_http(self: Arc<Self>, listener: TcpListener) {
        loop {
            let Ok((stream, _address)) = listener.accept().await else {
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                continue;
            };
            if self
                .records
                .read_control()
                .is_ok_and(|control| control.stop_service == Some(true))
            {
                return;
            }
            tokio::spawn(self.clone().serve_connection(TokioIo::new(stream)));
        }
    }

    async fn accept_https(self: Arc<Self>, listener: TcpListener, acceptor: TlsAcceptor) {
        loop {
            let Ok((stream, _address)) = listener.accept().await else {
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                continue;
            };
            let acceptor = acceptor.clone();
            let runtime = self.clone();
            tokio::spawn(async move {
                if let Ok(stream) = acceptor.accept(stream).await {
                    runtime.serve_connection(TokioIo::new(stream)).await;
                }
            });
        }
    }

    async fn accept_admin(self: Arc<Self>, listener: UnixListener) {
        loop {
            let Ok((stream, _address)) = listener.accept().await else {
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                continue;
            };
            tokio::spawn(self.clone().serve_connection(TokioIo::new(stream)));
        }
    }

    /// Boxed to break the type cycle: a load starts listeners whose connections handle loads.
    fn serve_connection<I>(self: Arc<Self>, io: I) -> Pin<Box<dyn Future<Output = ()> + Send>>
    where
        I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
    {
        let service = service_fn(move |request| self.clone().handle(request));
        Box::pin(async move {
            let _connection_result = http1::Builder::new().serve_connection(io, service).await;
        })
    }

    async fn handle(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, Infallible> {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let result = match (&method, path.as_str()) {
            (&Method::GET, "/config/") => self.admin_config().await,
            (&Method::GET, "/__pv/health") => self.health(),
            (&Method::POST, "/load") => self.load(request).await,
            _ => self
                .records
                .request(&method, &path, 404, 0)
                .map(|()| response(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", "")),
        };

        Ok(result.unwrap_or_else(|error| {
            let message = format!("pv-fake: {method} {path}: {error:#}\n");
            let _write_result = io::stderr().write_all(message.as_bytes());
            response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain; charset=utf-8",
                message,
            )
        }))
    }

    async fn admin_config(&self) -> Result<Response<Full<Bytes>>> {
        let control = self.records.take_control(AdminControl::take)?;
        let status = StatusCode::from_u16(control.status)?;
        self.records
            .request(&Method::GET, "/config/", control.status, 0)?;
        self.hold_readiness().await;
        if let Some(gate) = &control.gate {
            wait_for_path(gate).await;
        }

        Ok(response(status, "application/json", "{}\n"))
    }

    fn health(&self) -> Result<Response<Full<Bytes>>> {
        let body = self
            .health_body
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let (status, body) = match body {
            Some(body) => (StatusCode::OK, body),
            None => (StatusCode::NOT_FOUND, String::new()),
        };
        self.records
            .request(&Method::GET, "/__pv/health", status.as_u16(), 0)?;

        Ok(response(status, "text/plain; charset=utf-8", body))
    }

    /// Records the load, then finishes it in its own task: hyper drops a request's handler when
    /// the client disconnects, and a late accept must still apply after PV has stopped waiting.
    async fn load(self: Arc<Self>, request: Request<Incoming>) -> Result<Response<Full<Bytes>>> {
        let body = request.into_body().collect().await?.to_bytes();
        let source = String::from_utf8_lossy(&body).into_owned();
        // Caddy adapts the config, reading its imports, as soon as it arrives.
        let adapted = adapt(&source);
        let control = self
            .records
            .take_control(|control| LoadControl::take(control, adapted.is_ok()))?;
        self.records
            .request(&Method::POST, "/load", control.status, body.len())?;
        self.records.load(&source)?;

        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let response = self.finish_load(control, adapted, &source).await;
            let _send_result = sender.send(response);
        });

        receiver.await?
    }

    async fn finish_load(
        self: &Arc<Self>,
        control: LoadControl,
        adapted: Result<(GatewayConfig, ListenerPlan), String>,
        source: &str,
    ) -> Result<Response<Full<Bytes>>> {
        if let Some(gate) = &control.gate {
            wait_for_path(gate).await;
        }
        let (applied, adapt_error) = match adapted {
            Ok((config, plan)) if control.apply => (
                self.apply_after(&control, config, &plan, source).await,
                None,
            ),
            Ok(_adapted) => {
                tokio::time::sleep(control.delay).await;
                (Ok(()), None)
            }
            Err(error) => {
                tokio::time::sleep(control.delay).await;
                (Ok(()), Some(error))
            }
        };
        let body = match (&control.response_body, adapt_error, applied) {
            (Some(body), _, _) => body.clone(),
            (None, Some(error), _) => {
                error_body(&format!("adapting config using caddyfile adapter: {error}"))
            }
            (None, None, _) if !control.accepted() => error_body(&format!(
                "pv-fake: fake-admin-control.json rejected this load with status {}",
                control.status
            )),
            (None, None, Ok(())) => UNFORMATTED_WARNING.to_owned(),
            // Caddy has already sent its adapter warnings with a 200 when applying fails.
            (None, None, Err(error)) => format!("{UNFORMATTED_WARNING}{}", error_body(&error)),
        };
        if control.accepted() {
            if let Some(marker) = &control.accepted_marker {
                state::fs::write_sensitive_file(marker, "accepted\n")?;
            }
            if control.exit_after {
                let events = self.events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(EXIT_AFTER_LOAD_DELAY).await;
                    let _record_result = events.record(EventKind::Exit { code: 0 });
                    std::process::exit(0);
                });
            }
        }

        Ok(response(
            StatusCode::from_u16(control.status)?,
            "application/json",
            body,
        ))
    }

    /// Applies a load just before responding, `load_delay_ms` after it arrived; a late accept
    /// applies `late_apply_delay_ms` after it arrived instead.
    async fn apply_after(
        self: &Arc<Self>,
        control: &LoadControl,
        config: GatewayConfig,
        plan: &ListenerPlan,
        source: &str,
    ) -> Result<(), String> {
        let apply_delay = if control.late_accept {
            control.late_apply_delay
        } else {
            control.delay
        };
        tokio::time::sleep(apply_delay).await;
        let applied = self
            .apply(config, plan, source, control.retain_listeners)
            .await;
        tokio::time::sleep(control.delay.saturating_sub(apply_delay)).await;

        applied
    }

    /// Test controls next to the runtime config: `.readiness-fail` makes the runtime exit during
    /// readiness, and `.readiness-gate` holds readiness until the test removes it.
    async fn hold_readiness(&self) {
        if state::fs::path_entry_exists(&self.readiness_failure).unwrap_or(false) {
            let _record_result = self.events.record(EventKind::Exit { code: 1 });
            std::process::exit(1);
        }
        if state::fs::path_entry_exists(&self.readiness_gate).unwrap_or(false) {
            let _probe_result = state::fs::write_sensitive_file(&self.readiness_probed, "probed\n");
            while state::fs::path_entry_exists(&self.readiness_gate).unwrap_or(false) {
                tokio::time::sleep(GATE_POLL_INTERVAL).await;
            }
        }
    }
}

fn adapt(source: &str) -> Result<(GatewayConfig, ListenerPlan), String> {
    let config = GatewayConfig::parse(source).map_err(|error| format!("{error:#}"))?;
    let plan = config
        .listener_plan()
        .map_err(|error| format!("{error:#}"))?;

    Ok((config, plan))
}

/// Binds a TCP port without awaiting, so a failed bind leaves nothing half switched. Errors read
/// like Caddy's.
fn bind(port: u16) -> Result<TcpListener, String> {
    let listening = || -> io::Result<TcpListener> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        TcpListener::from_std(listener)
    };

    listening().map_err(|error| {
        let reason = match error.kind() {
            io::ErrorKind::AddrInUse => "address already in use".to_owned(),
            _ => error.to_string(),
        };
        format!(
            "loading config: loading new config: http app module: start: listening on \
             127.0.0.1:{port}: listen tcp 127.0.0.1:{port}: bind: {reason}"
        )
    })
}

async fn wait_for_path(path: &Utf8Path) {
    while !state::fs::path_entry_exists(path).unwrap_or(false) {
        tokio::time::sleep(GATE_POLL_INTERVAL).await;
    }
}

/// Caddy's admin API error body.
fn error_body(message: &str) -> String {
    let mut body = json!({ "error": message }).to_string();
    body.push('\n');

    body
}

fn tls_acceptor(https: &HttpsPlan) -> Result<TlsAcceptor> {
    Ok(TlsAcceptor::from(Arc::new(tls_config(
        &https.certificate,
        &https.private_key,
    )?)))
}

/// Issues leaf certificates for each requested hostname from the configured CA, as Caddy's
/// internal issuer does. Caddy signs through its own intermediate; PV only verifies up to the CA,
/// so signing leaves with the CA directly verifies the same way.
fn tls_config(certificate: &Utf8Path, private_key: &Utf8Path) -> Result<rustls::ServerConfig> {
    let certificate_pem = state::fs::read_to_string(certificate)?;
    let private_key_pem = state::fs::read_to_string(private_key)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth();
    let certificate_der = CertificateDer::from_pem_slice(certificate_pem.as_bytes())?;
    if !is_certificate_authority(&certificate_der)? {
        // ponytail: daemon tests still seed a self-signed leaf as the "CA", shared with Python
        // Gateway fakes that present it as-is. Do the same until those fakes are gone, then seed a
        // real CA in every test and drop this branch.
        let private_key_der = PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes())?;
        return Ok(config.with_single_cert(vec![certificate_der], private_key_der)?);
    }
    let resolver = LeafIssuer {
        certificate_authority: Issuer::from_ca_cert_pem(
            &certificate_pem,
            KeyPair::from_pem(&private_key_pem)?,
        )?,
        provider,
        leaves: Mutex::new(HashMap::new()),
    };

    Ok(config.with_cert_resolver(Arc::new(resolver)))
}

fn is_certificate_authority(certificate: &CertificateDer<'_>) -> Result<bool> {
    let (_rest, certificate) = x509_parser::parse_x509_certificate(certificate)
        .map_err(|error| anyhow!("parsing the configured certificate: {error}"))?;

    Ok(certificate
        .basic_constraints()?
        .is_some_and(|constraints| constraints.value.ca))
}

struct LeafIssuer {
    certificate_authority: Issuer<'static, KeyPair>,
    provider: Arc<CryptoProvider>,
    leaves: Mutex<HashMap<String, Arc<CertifiedKey>>>,
}

impl fmt::Debug for LeafIssuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("LeafIssuer").finish_non_exhaustive()
    }
}

impl LeafIssuer {
    fn issue(&self, hostname: &str) -> Result<CertifiedKey> {
        let mut params = CertificateParams::new(vec![hostname.to_owned()])?;
        // Caddy's leaves carry only the SAN; the subject is empty.
        params.distinguished_name = DistinguishedName::new();
        let key = KeyPair::generate()?;
        let leaf = params.signed_by(&key, &self.certificate_authority)?;
        let signing_key = self
            .provider
            .key_provider
            .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                key.serialize_der(),
            )))?;

        Ok(CertifiedKey::new(vec![leaf.der().clone()], signing_key))
    }
}

impl ResolvesServerCert for LeafIssuer {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let hostname = client_hello.server_name()?.to_owned();
        let mut leaves = self.leaves.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(leaf) = leaves.get(&hostname) {
            return Some(leaf.clone());
        }
        let leaf = Arc::new(self.issue(&hostname).ok()?);
        leaves.insert(hostname, leaf.clone());

        Some(leaf)
    }
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Bytes>,
) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));

    response
}
