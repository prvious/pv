use std::process::Stdio;
use std::time::Duration;
use std::{future::Future, io};

use camino::{Utf8Path, Utf8PathBuf};
use rustix::process::{Pid, Signal, kill_process_group};
use serde::Serialize;
use state::{PvPaths, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::time::{Instant, sleep, timeout};

use crate::DaemonError;

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpec {
    pub name: String,
    pub command: Utf8PathBuf,
    pub arguments: Vec<String>,
    pub log_path: Utf8PathBuf,
    pub pid_path: Utf8PathBuf,
    pub metadata_path: Utf8PathBuf,
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

#[derive(Serialize)]
struct RuntimeMetadata<'metadata> {
    name: &'metadata str,
    pid: u32,
    command: &'metadata str,
    arguments: &'metadata [String],
    log_path: &'metadata str,
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

        let child = command.spawn()?;
        let Some(pid) = child.id() else {
            return Err(DaemonError::MissingProcessId { name: spec.name });
        };

        fs::write_sensitive_file(&spec.pid_path, &format!("{pid}\n"))?;
        write_runtime_metadata(&spec, pid)?;

        Ok(ManagedProcess {
            pid,
            child,
            log_path: spec.log_path,
            pid_path: spec.pid_path,
            metadata_path: spec.metadata_path,
        })
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
        let process_group = process_group_pid(self.pid)?;
        kill_process_group(process_group, Signal::TERM).map_err(io::Error::from)?;

        if timeout(grace_period, self.child.wait()).await.is_err() {
            kill_process_group(process_group, Signal::KILL).map_err(io::Error::from)?;
            self.child.wait().await?;
        }

        Ok(())
    }
}

pub async fn wait_for_readiness(
    check: ReadinessCheck,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();

    while started_at.elapsed() < readiness_timeout {
        if check_once(&check).await.is_ok() {
            return Ok(());
        }

        sleep(READINESS_POLL_INTERVAL).await;
    }

    Err(DaemonError::ReadinessTimedOut {
        check: check.name(),
        timeout_ms: readiness_timeout.as_millis(),
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

    while started_at.elapsed() < readiness_timeout {
        if check().await {
            return Ok(());
        }

        sleep(READINESS_POLL_INTERVAL).await;
    }

    Err(DaemonError::ReadinessTimedOut {
        check: format!("custom:{name}"),
        timeout_ms: readiness_timeout.as_millis(),
    })
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

fn write_runtime_metadata(spec: &ProcessSpec, pid: u32) -> Result<(), DaemonError> {
    let started_at = timestamp()?;
    let metadata = RuntimeMetadata {
        name: &spec.name,
        pid,
        command: spec.command.as_str(),
        arguments: &spec.arguments,
        log_path: spec.log_path.as_str(),
        started_at,
    };
    let encoded = serde_json::to_string(&metadata)?;

    fs::write_sensitive_file(&spec.metadata_path, &encoded)?;

    Ok(())
}

fn timestamp() -> Result<String, DaemonError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}
