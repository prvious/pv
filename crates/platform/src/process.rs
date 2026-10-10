use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(target_os = "macos")]
#[path = "process/macos.rs"]
mod implementation;

pub use implementation::ProcessExitWatch;
#[cfg(not(target_os = "macos"))]
#[path = "process/unsupported.rs"]
mod implementation;

#[expect(
    clippy::disallowed_types,
    reason = "platform process helper owns shim process replacement"
)]
type StdCommand = std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub executable: Utf8PathBuf,
    pub argument_zero: String,
    pub arguments: Vec<String>,
    pub start_identity: ProcessStartIdentity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessStartIdentity {
    pub seconds: u64,
    pub microseconds: u64,
}

/// What a watched process did since [`ProcessExitWatch::try_events`] last collected its events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessEvents {
    /// The process replaced its program with `exec`. Its pid and birth identity are unchanged.
    pub exec: bool,
    /// The exact status of an exit the watch observed without reaping the process.
    pub exit: Option<std::process::ExitStatus>,
}

/// Identifies one kernel boot, independently of wall-clock changes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct BootSessionId(String);

impl TryFrom<String> for BootSessionId {
    type Error = io::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 36
            || !value.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
            || value.bytes().all(|byte| matches!(byte, b'0' | b'-'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid boot session UUID",
            ));
        }
        Ok(Self(value.to_ascii_uppercase()))
    }
}

impl From<BootSessionId> for String {
    fn from(value: BootSessionId) -> Self {
        value.0
    }
}

pub fn current_boot_session_id() -> Result<BootSessionId, crate::PlatformError> {
    implementation::current_boot_session_id()
}

pub fn inspect_process_identity(pid: u32) -> Result<Option<ProcessIdentity>, crate::PlatformError> {
    implementation::inspect_process_identity(pid)
}

pub fn inspect_process_start_identity(
    pid: u32,
) -> Result<Option<ProcessStartIdentity>, crate::PlatformError> {
    implementation::inspect_process_start_identity(pid)
}

/// Whether `pid` has exited and is waiting for its parent to reap it. `false` for a running
/// process, one still exiting, or none at all.
pub fn process_is_zombie(pid: u32) -> Result<bool, crate::PlatformError> {
    implementation::process_is_zombie(pid)
}

/// Whether a process group contains a member that can still run. Inspection failures are
/// errors, not evidence that the group has stopped. Zombies cannot run or retain listeners.
pub fn process_group_has_live_members(process_group: u32) -> Result<bool, crate::PlatformError> {
    implementation::process_group_has_live_members(process_group)
}

#[cfg(unix)]
pub fn exec_replace(program: &Path, args: &[String]) -> io::Result<ExitCode> {
    exec_replace_with_env(program, args, &[])
}

#[cfg(not(unix))]
pub fn exec_replace(program: &Path, args: &[String]) -> io::Result<ExitCode> {
    exec_replace_with_env(program, args, &[])
}

#[cfg(unix)]
pub fn exec_replace_with_env(
    program: &Path,
    args: &[String],
    env: &[(OsString, OsString)],
) -> io::Result<ExitCode> {
    let mut command = StdCommand::new(program);
    command.args(args).envs(env.iter().cloned());

    Err(command.exec())
}

#[cfg(not(unix))]
pub fn exec_replace_with_env(
    program: &Path,
    args: &[String],
    env: &[(OsString, OsString)],
) -> io::Result<ExitCode> {
    let status = StdCommand::new(program)
        .args(args)
        .envs(env.iter().cloned())
        .status()?;

    match status.code().and_then(|code| u8::try_from(code).ok()) {
        Some(code) => Ok(ExitCode::from(code)),
        None if status.success() => Ok(ExitCode::SUCCESS),
        None => Ok(ExitCode::FAILURE),
    }
}
