use std::io;
use std::process::Output;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::PlatformError;

pub const LAUNCH_AGENT_LABEL: &str = "com.prvious.pv.daemon";
pub const LAUNCH_AGENT_FILE_NAME: &str = "com.prvious.pv.daemon.plist";

const PV_MARKER: &str = "<!-- Managed by PV -->";
const DAEMON_COMMAND: &str = "daemon:run";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAgentConfig {
    pub program_path: Utf8PathBuf,
    pub stdout_path: Utf8PathBuf,
    pub stderr_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchAgentFileState {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        value: LaunchAgentConfig,
    },
    Stale {
        path: Utf8PathBuf,
        expected: Option<LaunchAgentConfig>,
        actual: Option<LaunchAgentConfig>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

impl LaunchAgentConfig {
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
            .and_then(LaunchAgentConfig::try_from_plist)
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

impl From<&LaunchAgentConfig> for LaunchAgentPlist {
    fn from(config: &LaunchAgentConfig) -> Self {
        Self {
            label: LAUNCH_AGENT_LABEL.to_string(),
            program_arguments: vec![config.program_path.to_string(), DAEMON_COMMAND.to_string()],
            keep_alive: true,
            run_at_load: true,
            stdout_path: config.stdout_path.to_string(),
            stderr_path: config.stderr_path.to_string(),
        }
    }
}

impl LaunchAgentConfig {
    fn try_from_plist(plist: LaunchAgentPlist) -> Option<Self> {
        if plist.label != LAUNCH_AGENT_LABEL || !plist.keep_alive || !plist.run_at_load {
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

pub fn inspect_launch_agent_file(
    path: &Utf8Path,
    expected: Option<&LaunchAgentConfig>,
) -> LaunchAgentFileState {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return LaunchAgentFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return LaunchAgentFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) {
        return LaunchAgentFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = LaunchAgentConfig::parse(&content);

    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => LaunchAgentFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (Some(expected), actual) => LaunchAgentFileState::Stale {
            path: path.to_path_buf(),
            expected: Some(expected.clone()),
            actual,
        },
        (None, Some(actual)) => LaunchAgentFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (None, None) => LaunchAgentFileState::Stale {
            path: path.to_path_buf(),
            expected: None,
            actual: None,
        },
    }
}

pub fn launch_agent_path(home: &Utf8Path) -> Utf8PathBuf {
    home.join("Library/LaunchAgents")
        .join(LAUNCH_AGENT_FILE_NAME)
}

pub fn write_launch_agent_file(
    path: &Utf8Path,
    config: &LaunchAgentConfig,
) -> Result<(), PlatformError> {
    state::fs::write_sensitive_file(path, &config.render()?)
        .map_err(|error| PlatformError::LaunchAgent(error.to_string()))
}

pub fn remove_launch_agent_file(path: &Utf8Path) -> Result<(), PlatformError> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(PlatformError::LaunchAgent(error.to_string())),
    }
}

pub fn bootstrap_launch_agent(plist_path: &Utf8Path) -> Result<(), PlatformError> {
    let target = launchctl_gui_target();
    let plist_path = plist_path.to_string();

    run_launchctl(&["bootstrap", &target, &plist_path])
}

pub fn bootout_launch_agent() -> Result<(), PlatformError> {
    let service = launchctl_service_target();

    run_launchctl(&["bootout", &service])
}

pub fn kickstart_launch_agent() -> Result<(), PlatformError> {
    let service = launchctl_service_target();

    run_launchctl(&["kickstart", "-k", &service])
}

fn launchctl_gui_target() -> String {
    format!("gui/{}", rustix::process::getuid().as_raw())
}

fn launchctl_service_target() -> String {
    format!("{}/{}", launchctl_gui_target(), LAUNCH_AGENT_LABEL)
}

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

#[expect(
    clippy::disallowed_types,
    reason = "platform LaunchAgent helper owns launchctl process execution"
)]
type StdCommand = std::process::Command;

fn launchctl_output(args: &[&str]) -> io::Result<Output> {
    StdCommand::new("/bin/launchctl").args(args).output()
}

fn insert_pv_marker(content: &str) -> String {
    if let Some((declaration, body)) = content.split_once('\n') {
        format!("{declaration}\n{PV_MARKER}\n{body}")
    } else {
        format!("{PV_MARKER}\n{content}")
    }
}
