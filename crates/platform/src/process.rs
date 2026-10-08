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
