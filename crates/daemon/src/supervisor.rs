use std::process::Stdio;
use std::time::Duration;
use std::{future::Future, io};

use camino::{Utf8Path, Utf8PathBuf};
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process};
use serde::{Deserialize, Serialize};
use state::{PvPaths, StateError, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::time::{Instant, sleep, timeout};

use crate::DaemonError;

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[expect(
    clippy::disallowed_types,
    reason = "PV process supervisor verifies live process ownership"
)]
type StdCommand = std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpec {
    pub name: String,
    pub command: Utf8PathBuf,
    pub arguments: Vec<String>,
    pub config_path: Utf8PathBuf,
    pub log_path: Utf8PathBuf,
    pub pid_path: Utf8PathBuf,
    pub metadata_path: Utf8PathBuf,
    pub resource_name: String,
    pub track: String,
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
    #[serde(default)]
    config_path: String,
    #[serde(default)]
    resource_name: String,
    #[serde(default)]
    track: String,
    log_path: String,
    started_at: String,
}

impl ProcessSupervisor {
    pub fn new(paths: PvPaths) -> Self {
        Self { paths }
    }

    pub async fn start(&self, spec: ProcessSpec) -> Result<ManagedProcess, DaemonError> {
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
        let Some(pid) = read_pid_file(&spec.pid_path)? else {
            return Ok(None);
        };
        let Some(metadata) = read_runtime_metadata(&spec.metadata_path)? else {
            return Ok(None);
        };

        if metadata.matches(spec, pid) && live_process_matches_spec(pid, spec)? {
            return Ok(Some(OwnedRuntime {
                pid,
                log_path: spec.log_path.clone(),
                pid_path: spec.pid_path.clone(),
                metadata_path: spec.metadata_path.clone(),
            }));
        }

        Ok(None)
    }

    pub fn adopt(&self, spec: &ProcessSpec) -> Result<Option<AdoptedProcess>, DaemonError> {
        Ok(self
            .verify_ownership(spec)?
            .map(|owned| AdoptedProcess { owned }))
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

    pub async fn stop(mut self, grace_period: Duration) -> Result<(), DaemonError> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }

        let process_group = process_group_pid(self.pid)?;
        signal_process_group(process_group, Signal::TERM)?;

        if timeout(grace_period, self.child.wait()).await.is_err() {
            signal_process_group(process_group, Signal::KILL)?;
            self.child.wait().await?;
        }

        Ok(())
    }
}

impl OwnedRuntime {
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl AdoptedProcess {
    pub fn pid(&self) -> u32 {
        self.owned.pid()
    }
}

pub async fn wait_for_readiness(
    check: ReadinessCheck,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        match timeout(remaining, check_once(&check)).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
            }
            Err(elapsed) => {
                last_error = Some(elapsed.to_string());
                break;
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: check.name(),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
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
            Self::Http { host, port, path } => format!("http:{host}:{port}{path}"),
        }
    }
}

async fn check_once(check: &ReadinessCheck) -> Result<(), DaemonError> {
    match check {
        ReadinessCheck::Tcp { host, port } => {
            let _stream = TcpStream::connect((host.as_str(), *port)).await?;
            Ok(())
        }
        ReadinessCheck::Http { host, port, path } => {
            let mut stream = TcpStream::connect((host.as_str(), *port)).await?;
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
            stream.write_all(request.as_bytes()).await?;

            let mut response = [0_u8; 12];
            let bytes = stream.read(&mut response).await?;
            let status_is_success = bytes >= 10
                && (response.starts_with(b"HTTP/1.1 2") || response.starts_with(b"HTTP/1.0 2"));

            if status_is_success {
                return Ok(());
            }

            Err(io::Error::other("HTTP readiness returned non-success status").into())
        }
    }
}

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

fn signal_process_group(process_group: Pid, signal: Signal) -> Result<(), DaemonError> {
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

async fn terminate_spawned_child(pid: u32, child: &mut Child) {
    if let Ok(process_group) = process_group_pid(pid) {
        let _result = signal_process_group(process_group, Signal::KILL);
    }

    let _result = child.wait().await;
}

#[expect(
    clippy::disallowed_types,
    reason = "PV process supervisor owns child process spawning"
)]
fn process_command(spec: &ProcessSpec) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&spec.command);
    command.args(&spec.arguments);
    #[cfg(unix)]
    command.process_group(0);

    command
}

fn persist_runtime_files(spec: &ProcessSpec, pid: u32) -> Result<(), DaemonError> {
    fs::write_sensitive_file(&spec.pid_path, &format!("{pid}\n"))?;
    write_runtime_metadata(spec, pid)
}

fn write_runtime_metadata(spec: &ProcessSpec, pid: u32) -> Result<(), DaemonError> {
    let started_at = timestamp()?;
    let metadata = RuntimeMetadata {
        name: spec.name.clone(),
        pid,
        command: spec.command.to_string(),
        arguments: spec.arguments.clone(),
        config_path: spec.config_path.to_string(),
        resource_name: spec.resource_name.clone(),
        track: spec.track.clone(),
        log_path: spec.log_path.to_string(),
        started_at,
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

fn process_exists(pid: u32) -> Result<bool, DaemonError> {
    let pid = process_group_pid(pid)?;

    match test_kill_process(pid) {
        Ok(()) => Ok(true),
        Err(source) => {
            let error = io::Error::from(source);
            if process_not_found(&error) {
                return Ok(false);
            }

            Err(error.into())
        }
    }
}

fn live_process_matches_spec(pid: u32, spec: &ProcessSpec) -> Result<bool, DaemonError> {
    if !process_exists(pid)? {
        return Ok(false);
    }

    let Some(command_line) = live_process_command_line(pid)? else {
        return Ok(false);
    };

    let command_matches = command_line.contains(spec.command.as_str())
        || spec.command.file_name().is_some_and(|file_name| {
            command_line
                .split_whitespace()
                .next()
                .is_some_and(|command| command.ends_with(file_name))
        });
    Ok(command_matches)
}

fn live_process_command_line(pid: u32) -> Result<Option<String>, DaemonError> {
    let output = StdCommand::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()?;

    if !output.status.success() {
        return Ok(None);
    }

    let command_line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if command_line.is_empty() {
        return Ok(None);
    }

    Ok(Some(command_line))
}

fn process_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(3)
}

impl RuntimeMetadata {
    fn matches(&self, spec: &ProcessSpec, pid: u32) -> bool {
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

fn timestamp() -> Result<String, DaemonError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}
