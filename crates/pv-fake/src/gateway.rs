//! The `caddy` and `frankenphp` personas: the parts of Caddy's CLI, admin API, and HTTP(S)
//! serving that PV uses. Behavior follows recordings of Caddy 2.11.4; FrankenPHP embeds Caddy, so
//! both personas behave the same.

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::io::{self, Write};
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
use tokio::net::{TcpListener, UnixListener};
use tokio_rustls::TlsAcceptor;

use crate::events::{EventKind, EventLog};

const READINESS_GATE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(10);

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

/// Accepts any existing config, as the Python fixtures did; tests that need a rejected config
/// install a validator scenario instead.
fn validate(config_path: &Utf8Path) -> Result<u8> {
    if state::fs::path_is_file(config_path)? {
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
    let config = GatewayConfig::parse(&state::fs::read_to_string(config_path)?)?;
    let http_port = match config.http_port {
        Some(port) => port,
        None => config.first_imported_site_port()?.ok_or_else(|| {
            anyhow!("{config_path} has no http_port and no imported http:// site")
        })?,
    };
    let admin_socket = config
        .admin_socket
        .clone()
        .ok_or_else(|| anyhow!("{config_path} has no `admin \"unix/<path>|0600\"` setting"))?;
    let https_port = config.https_port;
    let routes = Arc::new(Routes {
        health_body: config.health_body.clone(),
        readiness_gate: Utf8PathBuf::from(format!("{config_path}.readiness-gate")),
        readiness_probed: Utf8PathBuf::from(format!("{config_path}.readiness-probed")),
        readiness_failure: Utf8PathBuf::from(format!("{config_path}.readiness-fail")),
        events: events.clone(),
    });

    let http = TcpListener::bind(("127.0.0.1", http_port))
        .await
        .with_context(|| format!("binding HTTP port {http_port}"))?;
    let admin = UnixListener::bind(admin_socket.as_std_path())
        .with_context(|| format!("binding admin socket {admin_socket}"))?;
    state::fs::secure_sensitive_file(&admin_socket)?;
    let https = match (https_port, &config.ca_certificate, &config.ca_private_key) {
        (Some(port), Some(certificate), Some(private_key)) => Some((
            TcpListener::bind(("127.0.0.1", port))
                .await
                .with_context(|| format!("binding HTTPS port {port}"))?,
            TlsAcceptor::from(Arc::new(tls_config(certificate, private_key)?)),
        )),
        _ => None,
    };

    tokio::spawn(accept_tcp(http, None, routes.clone()));
    tokio::spawn(accept_unix(admin, routes.clone()));
    if let Some((listener, acceptor)) = https {
        tokio::spawn(accept_tcp(listener, Some(acceptor), routes));
    }

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

struct Routes {
    /// Served only when the config has a `respond /__pv/health` line, as with real Caddy.
    health_body: Option<String>,
    readiness_gate: Utf8PathBuf,
    readiness_probed: Utf8PathBuf,
    readiness_failure: Utf8PathBuf,
    events: EventLog,
}

impl Routes {
    async fn handle(
        &self,
        request: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, Infallible> {
        let response = match (request.method(), request.uri().path()) {
            (&Method::GET, "/config/") => {
                self.hold_readiness().await;
                response(StatusCode::OK, "application/json", "{}\n")
            }
            (&Method::GET, "/__pv/health") => match &self.health_body {
                Some(body) => response(StatusCode::OK, "text/plain; charset=utf-8", body.clone()),
                None => response(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", ""),
            },
            (&Method::POST, "/load") => {
                let _body = request.into_body().collect().await;
                response(StatusCode::OK, "text/plain; charset=utf-8", "")
            }
            _ => response(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", ""),
        };

        Ok(response)
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
                tokio::time::sleep(READINESS_GATE_POLL_INTERVAL).await;
            }
        }
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

async fn accept_tcp(listener: TcpListener, tls: Option<TlsAcceptor>, routes: Arc<Routes>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _address)) => stream,
            Err(_error) => {
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                continue;
            }
        };
        let tls = tls.clone();
        let routes = routes.clone();
        tokio::spawn(async move {
            match tls {
                Some(acceptor) => {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        serve_connection(TokioIo::new(stream), routes).await;
                    }
                }
                None => serve_connection(TokioIo::new(stream), routes).await,
            }
        });
    }
}

async fn accept_unix(listener: UnixListener, routes: Arc<Routes>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _address)) => stream,
            Err(_error) => {
                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                continue;
            }
        };
        tokio::spawn(serve_connection(TokioIo::new(stream), routes.clone()));
    }
}

async fn serve_connection<I>(io: I, routes: Arc<Routes>)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(move |request| {
        let routes = routes.clone();
        async move { routes.handle(request).await }
    });
    let _connection_result = http1::Builder::new().serve_connection(io, service).await;
}
