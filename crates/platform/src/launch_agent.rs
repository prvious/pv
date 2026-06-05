use std::io;
use std::process::ExitStatus;

use camino::{Utf8Path, Utf8PathBuf};

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

    pub fn render(&self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
{PV_MARKER}
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_AGENT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{program_path}</string>
    <string>{DAEMON_COMMAND}</string>
  </array>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout_path}</string>
  <key>StandardErrorPath</key>
  <string>{stderr_path}</string>
</dict>
</plist>
"#,
            program_path = escape_xml(self.program_path.as_str()),
            stdout_path = escape_xml(self.stdout_path.as_str()),
            stderr_path = escape_xml(self.stderr_path.as_str()),
        )
    }

    pub fn parse(content: &str) -> Option<Self> {
        if !content.lines().any(|line| line.trim() == PV_MARKER) {
            return None;
        }

        let label = string_after_key(content, "Label")?;
        if label != LAUNCH_AGENT_LABEL {
            return None;
        }

        let program_arguments = program_arguments(content)?;
        let [program_path, daemon_command] = program_arguments.as_slice() else {
            return None;
        };
        if daemon_command != DAEMON_COMMAND {
            return None;
        }

        if !boolean_after_key(content, "KeepAlive")? || !boolean_after_key(content, "RunAtLoad")? {
            return None;
        }

        Some(Self::new(
            program_path.as_str(),
            string_after_key(content, "StandardOutPath")?.as_str(),
            string_after_key(content, "StandardErrorPath")?.as_str(),
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
    state::fs::write_sensitive_file(path, &config.render())
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
    let status = launchctl_status(args).map_err(|source| PlatformError::LaunchAgentCommand {
        command: command.clone(),
        source,
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::LaunchAgentCommandStatus {
            command,
            status: status.to_string(),
        })
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "platform LaunchAgent helper owns launchctl process execution"
)]
type StdCommand = std::process::Command;

#[expect(
    clippy::disallowed_methods,
    reason = "platform LaunchAgent helper owns launchctl process execution"
)]
fn launchctl_status(args: &[&str]) -> io::Result<ExitStatus> {
    StdCommand::new("/bin/launchctl").args(args).status()
}

fn string_after_key(content: &str, key: &str) -> Option<String> {
    let string_start_marker = "<string>";
    let string_end_marker = "</string>";
    let key_marker = format!("<key>{}</key>", escape_xml(key));
    let after_key = content.split_once(&key_marker)?.1;
    let string_start = after_key.find(string_start_marker)? + string_start_marker.len();
    let string_end = after_key[string_start..].find(string_end_marker)? + string_start;

    unescape_xml(&after_key[string_start..string_end])
}

fn boolean_after_key(content: &str, key: &str) -> Option<bool> {
    let key_marker = format!("<key>{}</key>", escape_xml(key));
    let after_key = content.split_once(&key_marker)?.1.trim_start();

    if after_key.starts_with("<true/>") {
        Some(true)
    } else if after_key.starts_with("<false/>") {
        Some(false)
    } else {
        None
    }
}

fn program_arguments(content: &str) -> Option<Vec<String>> {
    let key_marker = "<key>ProgramArguments</key>";
    let after_key = content.split_once(key_marker)?.1;
    let array_start_marker = "<array>";
    let array_end_marker = "</array>";
    let array_start = after_key.find(array_start_marker)? + array_start_marker.len();
    let array_end = after_key[array_start..].find(array_end_marker)? + array_start;
    let array_content = &after_key[array_start..array_end];
    let mut arguments = Vec::new();
    let mut remaining = array_content;

    while let Some((_, after_start)) = remaining.split_once("<string>") {
        let (argument, after_end) = after_start.split_once("</string>")?;
        arguments.push(unescape_xml(argument)?);
        remaining = after_end;
    }

    Some(arguments)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unescape_xml(value: &str) -> Option<String> {
    let mut unescaped = value.to_string();

    for (escaped, plain) in [
        ("&apos;", "'"),
        ("&quot;", "\""),
        ("&gt;", ">"),
        ("&lt;", "<"),
        ("&amp;", "&"),
    ] {
        unescaped = unescaped.replace(escaped, plain);
    }

    if unescaped.contains('&') {
        None
    } else {
        Some(unescaped)
    }
}
