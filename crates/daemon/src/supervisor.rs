use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, future::Future, io};

use camino::{Utf8Path, Utf8PathBuf};
use platform::PlatformCapability;
#[cfg(target_os = "macos")]
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use state::{PvPaths, StateError, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::time::{Instant, sleep, timeout};
use tokio_rustls::TlsConnector;

use crate::DaemonError;

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const READINESS_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const PRIVATE_ENVIRONMENT_REDACTION: &str = "<redacted>";
const PRIVATE_ENVIRONMENT_FINGERPRINT_PREFIX: &str = "sha256:v1:";
const PHP_INI_ENVIRONMENT_KEYS: [&str; 2] = ["PHPRC", "PHP_INI_SCAN_DIR"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessSignal {
    Reload,
    Terminate,
    Kill,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProcessSpec {
    pub name: String,
    pub command: Utf8PathBuf,
    pub arguments: Vec<String>,
    pub private_environment: BTreeMap<String, String>,
    pub config_path: Utf8PathBuf,
    pub log_path: Utf8PathBuf,
    pub pid_path: Utf8PathBuf,
    pub metadata_path: Utf8PathBuf,
    pub resource_name: String,
    pub track: String,
}

impl fmt::Debug for ProcessSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("ProcessSpec");
        debug.field("name", &self.name);
        debug.field("command", &self.command);
        debug.field("arguments", &self.arguments);
        if !self.private_environment.is_empty() {
            debug.field(
                "private_environment",
                &PrivateEnvironmentDebug(&self.private_environment),
            );
        }
        debug.field("config_path", &self.config_path);
        debug.field("log_path", &self.log_path);
        debug.field("pid_path", &self.pid_path);
        debug.field("metadata_path", &self.metadata_path);
        debug.field("resource_name", &self.resource_name);
        debug.field("track", &self.track);
        debug.finish()
    }
}

struct PrivateEnvironmentDebug<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for PrivateEnvironmentDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(
                self.0
                    .keys()
                    .map(|name| (name, PRIVATE_ENVIRONMENT_REDACTION)),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct ProcessSupervisor {
    paths: PvPaths,
}

pub struct ManagedProcess {
    pid: u32,
    child: Child,
    log_path: Utf8PathBuf,
    pid_path: Utf8PathBuf,
    metadata_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRuntime {
    pid: u32,
    command: Utf8PathBuf,
    arguments: Vec<String>,
    process_start_identity: platform::ProcessStartIdentity,
    log_path: Utf8PathBuf,
    pid_path: Utf8PathBuf,
    metadata_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdoptedProcess {
    owned: OwnedRuntime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessCheck {
    Tcp {
        host: String,
        port: u16,
    },
    GatewayHttps {
        http_host: String,
        http_port: u16,
        https_host: String,
        https_port: u16,
        server_name: String,
        ca_certificate_path: Utf8PathBuf,
    },
    RedisPing {
        host: String,
        port: u16,
    },
    Http {
        host: String,
        port: u16,
        path: String,
    },
}

#[derive(Deserialize, Serialize)]
struct RuntimeMetadata {
    name: String,
    pid: u32,
    command: String,
    arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    private_environment_fingerprint: Option<String>,
    #[serde(default)]
    config_path: String,
    #[serde(default)]
    resource_name: String,
    #[serde(default)]
    track: String,
    log_path: String,
    started_at: String,
    #[serde(default)]
    process_start_identity: Option<platform::ProcessStartIdentity>,
}

impl ProcessSupervisor {
    pub fn new(paths: PvPaths) -> Self {
        Self { paths }
    }

    pub async fn start(&self, spec: ProcessSpec) -> Result<ManagedProcess, DaemonError> {
        self.start_inner(spec, |_pid| async {}).await
    }

    async fn start_inner<PostSpawn, PostSpawnFuture>(
        &self,
        spec: ProcessSpec,
        post_spawn: PostSpawn,
    ) -> Result<ManagedProcess, DaemonError>
    where
        PostSpawn: FnOnce(u32) -> PostSpawnFuture,
        PostSpawnFuture: Future<Output = ()>,
    {
        require_process_containment()?;
        state::fs::ensure_layout(&self.paths)?;

        let stdout = fs::open_append_file(&spec.log_path)?;
        let stderr = fs::open_append_file(&spec.log_path)?;
        let mut command = process_command(&spec);
        command.stdout(Stdio::from(stdout));
        command.stderr(Stdio::from(stderr));

        let mut child = command.spawn()?;
        let Some(pid) = child.id() else {
            return Err(DaemonError::MissingProcessId { name: spec.name });
        };
        post_spawn(pid).await;

        if let Err(error) = persist_runtime_files(&spec, pid) {
            terminate_spawned_child(pid, &mut child).await;

            return Err(error);
        }

        Ok(ManagedProcess {
            pid,
            child,
            log_path: spec.log_path,
            pid_path: spec.pid_path,
            metadata_path: spec.metadata_path,
        })
    }

    pub fn verify_ownership(
        &self,
        spec: &ProcessSpec,
    ) -> Result<Option<OwnedRuntime>, DaemonError> {
        require_process_containment()?;
        let Some(pid) = read_pid_file(&spec.pid_path)? else {
            return Ok(None);
        };
        let Some(metadata) = read_runtime_metadata(&spec.metadata_path)? else {
            return Ok(None);
        };

        let Some(process_start_identity) = metadata.process_start_identity else {
            return Ok(None);
        };

        if metadata.matches(spec, pid)
            && live_process_matches(pid, &spec.command, &spec.arguments, process_start_identity)?
        {
            return Ok(Some(OwnedRuntime {
                pid,
                command: spec.command.clone(),
                arguments: spec.arguments.clone(),
                process_start_identity,
                log_path: spec.log_path.clone(),
                pid_path: spec.pid_path.clone(),
                metadata_path: spec.metadata_path.clone(),
            }));
        }

        Ok(None)
    }

    pub fn adopt(&self, spec: &ProcessSpec) -> Result<Option<AdoptedProcess>, DaemonError> {
        require_process_containment()?;
        Ok(self
            .verify_ownership(spec)?
            .map(|owned| AdoptedProcess { owned }))
    }

    pub fn adopt_recorded(
        &self,
        pid_path: &Utf8Path,
        metadata_path: &Utf8Path,
    ) -> Result<Option<AdoptedProcess>, DaemonError> {
        require_process_containment()?;
        let Some(pid) = read_pid_file(pid_path)? else {
            return Ok(None);
        };
        let Some(metadata) = read_runtime_metadata(metadata_path)? else {
            return Ok(None);
        };
        let spec = metadata.process_spec(pid_path.to_path_buf(), metadata_path.to_path_buf());

        let Some(process_start_identity) = metadata.process_start_identity else {
            return Ok(None);
        };

        if metadata.matches_recorded(&spec, pid)
            && live_process_matches(pid, &spec.command, &spec.arguments, process_start_identity)?
        {
            return Ok(Some(AdoptedProcess {
                owned: OwnedRuntime {
                    pid,
                    command: spec.command,
                    arguments: spec.arguments,
                    process_start_identity,
                    log_path: spec.log_path,
                    pid_path: spec.pid_path,
                    metadata_path: spec.metadata_path,
                },
            }));
        }

        Ok(None)
    }

    pub fn reload(&self, spec: &ProcessSpec) -> Result<bool, DaemonError> {
        require_process_containment()?;
        let Some(owned) = self.verify_ownership(spec)? else {
            return Ok(false);
        };

        owned.signal(ProcessSignal::Reload)
    }
}

impl ManagedProcess {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn log_path(&self) -> &Utf8Path {
        &self.log_path
    }

    pub fn pid_path(&self) -> &Utf8Path {
        &self.pid_path
    }

    pub fn metadata_path(&self) -> &Utf8Path {
        &self.metadata_path
    }

    pub fn has_exited(&mut self) -> Result<bool, DaemonError> {
        Ok(self.child.try_wait()?.is_some())
    }

    pub async fn stop(mut self, grace_period: Duration) -> Result<(), DaemonError> {
        require_process_containment()?;
        if self.child.try_wait()?.is_some() && !process_group_exists(self.pid)? {
            return Ok(());
        }

        signal_process_group(self.pid, ProcessSignal::Terminate)?;

        match timeout(
            grace_period,
            wait_for_managed_process_group_exit(&mut self.child, self.pid),
        )
        .await
        {
            Ok(result) => return result,
            Err(_elapsed) => {
                signal_process_group(self.pid, ProcessSignal::Kill)?;
            }
        }

        match timeout(
            Duration::from_secs(1),
            wait_for_managed_process_group_exit(&mut self.child, self.pid),
        )
        .await
        {
            Ok(result) => result,
            Err(_elapsed) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process group {} did not exit after signal", self.pid),
            )
            .into()),
        }
    }
}

impl OwnedRuntime {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn matches_live(&self) -> Result<bool, DaemonError> {
        live_process_matches(
            self.pid,
            &self.command,
            &self.arguments,
            self.process_start_identity,
        )
    }

    fn signal(&self, signal: ProcessSignal) -> Result<bool, DaemonError> {
        if !self.matches_live()? {
            return Ok(false);
        }

        signal_process_group(self.pid, signal)?;

        Ok(true)
    }
}

impl AdoptedProcess {
    pub fn pid(&self) -> u32 {
        self.owned.pid()
    }

    pub async fn stop(self, grace_period: Duration) -> Result<(), DaemonError> {
        require_process_containment()?;
        if !self.owned.matches_live()? {
            return Ok(());
        }

        stop_process_group_by_pid(self.owned.pid, grace_period).await
    }
}

pub async fn wait_for_readiness(
    check: ReadinessCheck,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        let probe_timeout = remaining.min(READINESS_PROBE_TIMEOUT);
        match timeout(probe_timeout, check_once(&check)).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
            }
            Err(elapsed) => {
                last_error = Some(elapsed.to_string());
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: check.name(),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
}

pub(crate) async fn probe_readiness_once(check: &ReadinessCheck) -> Result<(), DaemonError> {
    check_once(check).await
}

pub async fn wait_for_custom_readiness<F, Fut>(
    name: &str,
    readiness_timeout: Duration,
    mut check: F,
) -> Result<(), DaemonError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        match timeout(remaining, check()).await {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last_error = Some("custom readiness returned false".to_string());
                sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
            }
            Err(elapsed) => {
                last_error = Some(elapsed.to_string());
                break;
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: format!("custom:{name}"),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
}

fn remaining_timeout(started_at: Instant, readiness_timeout: Duration) -> Option<Duration> {
    readiness_timeout
        .checked_sub(started_at.elapsed())
        .filter(|remaining| !remaining.is_zero())
}

impl ReadinessCheck {
    fn name(&self) -> String {
        match self {
            Self::Tcp { host, port } => format!("tcp:{host}:{port}"),
            Self::GatewayHttps {
                http_host,
                http_port,
                https_host,
                https_port,
                server_name,
                ..
            } => {
                format!(
                    "gateway:https:{server_name}:{https_host}:{https_port};tcp:{http_host}:{http_port}"
                )
            }
            Self::RedisPing { host, port } => format!("redis-ping:{host}:{port}"),
            Self::Http { host, port, path } => format!("http:{host}:{port}{path}"),
        }
    }
}

async fn check_once(check: &ReadinessCheck) -> Result<(), DaemonError> {
    match check {
        ReadinessCheck::Tcp { host, port } => check_tcp_once(host, *port).await,
        ReadinessCheck::GatewayHttps {
            http_host,
            http_port,
            https_host,
            https_port,
            server_name,
            ca_certificate_path,
        } => {
            check_tcp_once(http_host, *http_port).await?;
            check_https_once(https_host, *https_port, server_name, ca_certificate_path).await
        }
        ReadinessCheck::RedisPing { host, port } => {
            let url = format!("redis://{host}:{port}/");
            let client = redis::Client::open(url)?;
            let mut connection = client.get_multiplexed_async_connection().await?;
            let pong: String = redis::cmd("PING").query_async(&mut connection).await?;
            if pong == "PONG" {
                return Ok(());
            }

            Err(DaemonError::DaemonRejected {
                message: format!("Redis PING returned {pong}"),
            })
        }
        ReadinessCheck::Http { host, port, path } => {
            let mut stream = TcpStream::connect((host.as_str(), *port)).await?;
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
            stream.write_all(request.as_bytes()).await?;

            let mut response = [0_u8; 12];
            let bytes = stream.read(&mut response).await?;
            if http_status_is_success(&response, bytes) {
                return Ok(());
            }

            Err(io::Error::other("HTTP readiness returned non-success status").into())
        }
    }
}

async fn check_tcp_once(host: &str, port: u16) -> Result<(), DaemonError> {
    let _stream = TcpStream::connect((host, port)).await?;

    Ok(())
}

async fn check_https_once(
    host: &str,
    port: u16,
    server_name: &str,
    ca_certificate_path: &Utf8Path,
) -> Result<(), DaemonError> {
    let tcp_stream = TcpStream::connect((host, port)).await?;
    let connector = TlsConnector::from(tls_client_config(ca_certificate_path)?);
    let server_name_text = server_name.to_owned();
    let server_name = ServerName::try_from(server_name_text.clone()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid TLS server name `{server_name_text}`: {error}"),
        )
    })?;
    let _stream = connector.connect(server_name, tcp_stream).await?;

    Ok(())
}

fn http_status_is_success(response: &[u8], bytes: usize) -> bool {
    bytes >= 10 && (response.starts_with(b"HTTP/1.1 2") || response.starts_with(b"HTTP/1.0 2"))
}

fn tls_client_config(
    ca_certificate_path: &Utf8Path,
) -> Result<Arc<rustls::ClientConfig>, DaemonError> {
    let certificate_pem = fs::read_to_string(ca_certificate_path)?;
    let mut reader = certificate_pem.as_bytes();
    let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    let mut root_store = rustls::RootCertStore::empty();
    let (added, _ignored) = root_store.add_parsable_certificates(certificates);
    if added == 0 {
        return Err(io::Error::other(format!(
            "no CA certificates could be loaded from {ca_certificate_path}"
        ))
        .into());
    }

    Ok(Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|error| io::Error::other(format!("TLS protocol configuration failed: {error}")))?
        .with_root_certificates(root_store)
        .with_no_client_auth(),
    ))
}

fn require_process_containment() -> Result<(), DaemonError> {
    platform::require_capability(PlatformCapability::ProcessContainment)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn process_group_pid(pid: u32) -> Result<Pid, DaemonError> {
    let raw_pid =
        i32::try_from(pid).map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;

    Pid::from_raw(raw_pid).ok_or_else(|| {
        DaemonError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process id must be positive",
        ))
    })
}

#[cfg(target_os = "macos")]
fn signal_process_group(pid: u32, signal: ProcessSignal) -> Result<(), DaemonError> {
    let process_group = process_group_pid(pid)?;
    let signal = match signal {
        ProcessSignal::Reload => Signal::USR1,
        ProcessSignal::Terminate => Signal::TERM,
        ProcessSignal::Kill => Signal::KILL,
    };

    match kill_process_group(process_group, signal) {
        Ok(()) => Ok(()),
        Err(source) => {
            let error = io::Error::from(source);
            if process_not_found(&error) {
                return Ok(());
            }

            Err(error.into())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn signal_process_group(_pid: u32, _signal: ProcessSignal) -> Result<(), DaemonError> {
    require_process_containment()
}

async fn terminate_spawned_child(pid: u32, child: &mut Child) {
    #[cfg(target_os = "macos")]
    let _group_kill_result = signal_process_group(pid, ProcessSignal::Kill);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let _pid = pid;

    let _result = child.kill().await;

    let _result = child.wait().await;
}

async fn stop_process_group_by_pid(pid: u32, grace_period: Duration) -> Result<(), DaemonError> {
    signal_process_group(pid, ProcessSignal::Terminate)?;

    if wait_for_process_group_exit(pid, grace_period).await? {
        return Ok(());
    }

    signal_process_group(pid, ProcessSignal::Kill)?;

    if wait_for_process_group_exit(pid, Duration::from_secs(1)).await? {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("process {pid} did not exit after signal"),
    )
    .into())
}

async fn wait_for_managed_process_group_exit(
    child: &mut Child,
    pid: u32,
) -> Result<(), DaemonError> {
    child.wait().await?;

    while process_group_exists(pid)? {
        sleep(READINESS_POLL_INTERVAL).await;
    }

    Ok(())
}

async fn wait_for_process_group_exit(
    pid: u32,
    readiness_timeout: Duration,
) -> Result<bool, DaemonError> {
    let started_at = Instant::now();

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        if !process_group_exists(pid)? {
            return Ok(true);
        }

        sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
    }

    Ok(!process_group_exists(pid)?)
}

#[expect(
    clippy::disallowed_types,
    reason = "PV process supervisor owns child process spawning"
)]
fn process_command(spec: &ProcessSpec) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&spec.command);
    command.args(&spec.arguments);
    for key in PHP_INI_ENVIRONMENT_KEYS {
        command.env_remove(key);
    }
    command.envs(&spec.private_environment);
    #[cfg(target_os = "macos")]
    command.process_group(0);

    command
}

fn persist_runtime_files(spec: &ProcessSpec, pid: u32) -> Result<(), DaemonError> {
    let Some(process_identity) = platform::inspect_process_identity(pid)? else {
        return Err(DaemonError::MissingProcessIdentity {
            name: spec.name.clone(),
            pid,
        });
    };

    fs::write_sensitive_file(&spec.pid_path, &format!("{pid}\n"))?;
    write_runtime_metadata(spec, pid, process_identity.start_identity)
}

fn write_runtime_metadata(
    spec: &ProcessSpec,
    pid: u32,
    process_start_identity: platform::ProcessStartIdentity,
) -> Result<(), DaemonError> {
    let started_at = timestamp()?;
    let metadata = RuntimeMetadata {
        name: spec.name.clone(),
        pid,
        command: spec.command.to_string(),
        arguments: spec.arguments.clone(),
        private_environment_fingerprint: private_environment_fingerprint(&spec.private_environment),
        config_path: spec.config_path.to_string(),
        resource_name: spec.resource_name.clone(),
        track: spec.track.clone(),
        log_path: spec.log_path.to_string(),
        started_at,
        process_start_identity: Some(process_start_identity),
    };
    let encoded = serde_json::to_string(&metadata)?;

    fs::write_sensitive_file(&spec.metadata_path, &encoded)?;

    Ok(())
}

fn read_pid_file(path: &Utf8Path) -> Result<Option<u32>, DaemonError> {
    let Some(content) = read_optional_file(path)? else {
        return Ok(None);
    };
    let pid = content
        .trim()
        .parse::<u32>()
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;

    Ok(Some(pid))
}

fn read_runtime_metadata(path: &Utf8Path) -> Result<Option<RuntimeMetadata>, DaemonError> {
    let Some(content) = read_optional_file(path)? else {
        return Ok(None);
    };

    Ok(Some(serde_json::from_str(&content)?))
}

fn read_optional_file(path: &Utf8Path) -> Result<Option<String>, DaemonError> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "macos")]
fn process_group_exists(pid: u32) -> Result<bool, DaemonError> {
    let process_group = process_group_pid(pid)?;

    match test_kill_process_group(process_group) {
        Ok(()) => Ok(true),
        Err(source) => {
            let error = io::Error::from(source);
            if process_not_found(&error) || error.kind() == io::ErrorKind::PermissionDenied {
                return Ok(false);
            }

            Err(error.into())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn process_group_exists(_pid: u32) -> Result<bool, DaemonError> {
    require_process_containment()?;

    Ok(false)
}

fn live_process_matches(
    pid: u32,
    command: &Utf8Path,
    arguments: &[String],
    process_start_identity: platform::ProcessStartIdentity,
) -> Result<bool, DaemonError> {
    let Some(process_identity) = platform::inspect_process_identity(pid)? else {
        return Ok(false);
    };
    if process_identity.start_identity != process_start_identity {
        return Ok(false);
    }

    Ok(process_identity_matches(
        &process_identity,
        command,
        arguments,
    ))
}

fn process_identity_matches(
    process_identity: &platform::ProcessIdentity,
    command: &Utf8Path,
    arguments: &[String],
) -> bool {
    let direct_arguments_match = process_identity.arguments == arguments;
    let direct_command_matches = process_identity.executable == command
        || (command == Utf8Path::new("/bin/sh")
            && process_identity.executable == Utf8Path::new("/bin/bash")
            && process_identity.argument_zero == command.as_str());
    let script_command_matches = process_identity
        .arguments
        .first()
        .is_some_and(|script| script == command.as_str())
        && fs::read_to_string(command).is_ok_and(|source| source.starts_with("#!"));
    let script_arguments_match = process_identity
        .arguments
        .get(1..)
        .is_some_and(|live_arguments| live_arguments == arguments);

    (direct_command_matches && direct_arguments_match)
        || (script_command_matches && script_arguments_match)
}

#[cfg(target_os = "macos")]
fn process_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(3)
}

impl RuntimeMetadata {
    fn process_spec(&self, pid_path: Utf8PathBuf, metadata_path: Utf8PathBuf) -> ProcessSpec {
        ProcessSpec {
            name: self.name.clone(),
            command: self.command.as_str().into(),
            arguments: self.arguments.clone(),
            private_environment: BTreeMap::new(),
            config_path: self.config_path.as_str().into(),
            log_path: self.log_path.as_str().into(),
            pid_path,
            metadata_path,
            resource_name: self.resource_name.clone(),
            track: self.track.clone(),
        }
    }

    fn matches(&self, spec: &ProcessSpec, pid: u32) -> bool {
        self.matches_recorded(spec, pid)
            && self.private_environment_fingerprint.as_deref()
                == private_environment_fingerprint(&spec.private_environment).as_deref()
    }

    fn matches_recorded(&self, spec: &ProcessSpec, pid: u32) -> bool {
        self.name == spec.name
            && self.pid == pid
            && self.command == spec.command.as_str()
            && self.arguments == spec.arguments
            && self.config_path == spec.config_path.as_str()
            && self.resource_name == spec.resource_name
            && self.track == spec.track
            && self.log_path == spec.log_path.as_str()
    }
}

fn private_environment_fingerprint(environment: &BTreeMap<String, String>) -> Option<String> {
    if environment.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    for (key, value) in environment {
        hasher.update((key.len() as u64).to_be_bytes());
        hasher.update(key.as_bytes());
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }

    Some(format!(
        "{PRIVATE_ENVIRONMENT_FINGERPRINT_PREFIX}{:x}",
        hasher.finalize()
    ))
}

fn timestamp() -> Result<String, DaemonError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use camino_tempfile::tempdir;
    use rustix::process::{Pid, Signal, kill_process, test_kill_process};
    use tokio::time::sleep;

    use super::{ProcessSpec, ProcessSupervisor};
    use state::PvPaths;

    #[tokio::test]
    async fn startup_persistence_failure_terminates_process_group_descendants() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths)?;
        let descendant_pid_path = paths.run().join("startup-descendant.pid");
        let metadata_parent_blocker = paths.run().join("metadata-parent");
        state::fs::write_sensitive_file(&metadata_parent_blocker, "not a directory")?;
        let descendant_pid_path_for_command = descendant_pid_path.clone();
        let descendant_pid_path_for_hook = descendant_pid_path.clone();
        let result = ProcessSupervisor::new(paths.clone())
            .start_inner(
                ProcessSpec {
                    name: "startup-descendant".to_string(),
                    command: "/bin/sh".into(),
                    arguments: vec![
                        "-c".to_string(),
                        format!(
                            "sh -c 'while true; do sleep 1; done' & echo $! > \"{descendant_pid_path_for_command}\"; while true; do sleep 1; done"
                        ),
                    ],
                    private_environment: Default::default(),
                    config_path: paths.config().join("startup-descendant.json"),
                    log_path: paths.logs().join("startup-descendant.log"),
                    pid_path: paths.run().join("startup-descendant-leader.pid"),
                    metadata_path: metadata_parent_blocker.join("metadata.json"),
                    resource_name: "startup-descendant".to_string(),
                    track: "test".to_string(),
                },
                move |_pid| async move {
                    wait_for_test_path(&descendant_pid_path_for_hook).await;
                },
            )
            .await;

        assert!(result.is_err());
        let descendant_pid = state::fs::read_to_string(&descendant_pid_path)?
            .trim()
            .parse::<u32>()?;
        let descendant_exited = wait_for_test_process_exit(descendant_pid).await?;
        if !descendant_exited {
            kill_test_process(descendant_pid)?;
            let _cleanup_complete = wait_for_test_process_exit(descendant_pid).await?;
        }

        assert!(
            descendant_exited,
            "startup persistence failure left descendant process {descendant_pid} alive"
        );

        Ok(())
    }

    async fn wait_for_test_path(path: &camino::Utf8Path) {
        for _attempt in 0..50 {
            if path.exists() {
                return;
            }

            sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_test_process_exit(pid: u32) -> Result<bool> {
        let pid = test_pid(pid)?;
        for _attempt in 0..50 {
            if test_kill_process(pid).is_err() {
                return Ok(true);
            }

            sleep(Duration::from_millis(20)).await;
        }

        Ok(false)
    }

    fn kill_test_process(pid: u32) -> Result<()> {
        kill_process(test_pid(pid)?, Signal::KILL)?;

        Ok(())
    }

    fn test_pid(pid: u32) -> Result<Pid> {
        let raw_pid = i32::try_from(pid)?;

        Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))
    }
}
