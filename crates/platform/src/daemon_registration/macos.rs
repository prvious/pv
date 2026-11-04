#[cfg(target_os = "macos")]
use std::{io, process::Output};

#[cfg(target_os = "macos")]
use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::PlatformError;

pub const DAEMON_REGISTRATION_LABEL: &str = "com.prvious.pv.daemon";
pub const DAEMON_REGISTRATION_FILE_NAME: &str = "com.prvious.pv.daemon.plist";
pub(super) const PV_MARKER: &str = "<!-- Managed by PV -->";
const DAEMON_COMMAND: &str = "daemon:run";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonRegistrationConfig {
    pub program_path: Utf8PathBuf,
    pub stdout_path: Utf8PathBuf,
    pub stderr_path: Utf8PathBuf,
}

impl DaemonRegistrationConfig {
    pub fn for_paths(paths: &state::PvPaths) -> Self {
        Self::new(
            paths.active_pv_binary(),
            paths.daemon_registration_stdout_log(),
            paths.daemon_registration_stderr_log(),
        )
    }

    pub fn new(
        program_path: impl Into<Utf8PathBuf>,
        stdout_path: impl Into<Utf8PathBuf>,
        stderr_path: impl Into<Utf8PathBuf>,
    ) -> Self {
        Self {
            program_path: program_path.into(),
            stdout_path: stdout_path.into(),
            stderr_path: stderr_path.into(),
        }
    }

    pub fn render(&self) -> Result<String, PlatformError> {
        let mut content = Vec::new();
        plist::to_writer_xml(&mut content, &LaunchAgentPlist::from(self))
            .map_err(|error| PlatformError::LaunchAgent(error.to_string()))?;
        let content = String::from_utf8(content)
            .map_err(|error| PlatformError::LaunchAgent(error.to_string()))?;

        Ok(insert_pv_marker(&content))
    }

    pub fn parse(content: &str) -> Option<Self> {
        if !content.lines().any(|line| line.trim() == PV_MARKER) {
            return None;
        }

        plist::from_bytes::<LaunchAgentPlist>(content.as_bytes())
            .ok()
            .and_then(DaemonRegistrationConfig::try_from_plist)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct LaunchAgentPlist {
    #[serde(rename = "Label")]
    label: String,
    #[serde(rename = "ProgramArguments")]
    program_arguments: Vec<String>,
    #[serde(rename = "KeepAlive")]
    keep_alive: bool,
    #[serde(rename = "RunAtLoad")]
    run_at_load: bool,
    #[serde(rename = "StandardOutPath")]
    stdout_path: String,
    #[serde(rename = "StandardErrorPath")]
    stderr_path: String,
}

impl From<&DaemonRegistrationConfig> for LaunchAgentPlist {
    fn from(config: &DaemonRegistrationConfig) -> Self {
        Self {
            label: DAEMON_REGISTRATION_LABEL.to_string(),
            program_arguments: vec![config.program_path.to_string(), DAEMON_COMMAND.to_string()],
            keep_alive: true,
            run_at_load: true,
            stdout_path: config.stdout_path.to_string(),
            stderr_path: config.stderr_path.to_string(),
        }
    }
}

impl DaemonRegistrationConfig {
    fn try_from_plist(plist: LaunchAgentPlist) -> Option<Self> {
        if plist.label != DAEMON_REGISTRATION_LABEL || !plist.keep_alive || !plist.run_at_load {
            return None;
        }

        let [program_path, daemon_command] = plist.program_arguments.as_slice() else {
            return None;
        };
        if daemon_command != DAEMON_COMMAND {
            return None;
        }

        Some(Self::new(
            program_path.as_str(),
            plist.stdout_path.as_str(),
            plist.stderr_path.as_str(),
        ))
    }
}

fn insert_pv_marker(content: &str) -> String {
    if let Some((declaration, body)) = content.split_once('\n') {
        format!("{declaration}\n{PV_MARKER}\n{body}")
    } else {
        format!("{PV_MARKER}\n{content}")
    }
}

#[cfg(target_os = "macos")]
pub(super) fn enable(path: &Utf8Path) -> Result<(), PlatformError> {
    let target = launchctl_gui_target();
    let plist_path = path.to_string();
    run_launchctl(&["bootstrap", &target, &plist_path])
}

#[cfg(target_os = "macos")]
pub(super) fn stop() -> Result<(), PlatformError> {
    let service = launchctl_service_target();
    run_launchctl(&["bootout", &service])
}

#[cfg(target_os = "macos")]
pub(super) fn restart() -> Result<(), PlatformError> {
    let service = launchctl_service_target();
    run_launchctl(&["kickstart", "-k", &service])
}

#[cfg(target_os = "macos")]
fn launchctl_gui_target() -> String {
    format!("gui/{}", rustix::process::getuid().as_raw())
}

#[cfg(target_os = "macos")]
fn launchctl_service_target() -> String {
    format!("{}/{}", launchctl_gui_target(), DAEMON_REGISTRATION_LABEL)
}

#[cfg(target_os = "macos")]
fn run_launchctl(args: &[&str]) -> Result<(), PlatformError> {
    let command = format!("/bin/launchctl {}", args.join(" "));
    let output = launchctl_output(args).map_err(|source| PlatformError::LaunchAgentCommand {
        command: command.clone(),
        source,
    })?;

    if output.status.success() {
        Ok(())
    } else if output.stderr.is_empty() {
        Err(PlatformError::LaunchAgentCommandStatus {
            command,
            status: output.status.to_string(),
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        Err(PlatformError::LaunchAgent(format!(
            "{command} exited with {status}: {stderr}",
            status = output.status,
        )))
    }
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_types,
    reason = "platform LaunchAgent helper owns launchctl process execution"
)]
type StdCommand = std::process::Command;

#[cfg(target_os = "macos")]
fn launchctl_output(args: &[&str]) -> io::Result<Output> {
    StdCommand::new("/bin/launchctl").args(args).output()
}
