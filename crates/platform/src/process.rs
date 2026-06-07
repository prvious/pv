use std::io;
use std::path::Path;
use std::process::ExitCode;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[expect(
    clippy::disallowed_types,
    reason = "platform process helper owns shim process replacement"
)]
type StdCommand = std::process::Command;

#[cfg(unix)]
pub fn exec_replace(program: &Path, args: &[String]) -> io::Result<ExitCode> {
    Err(StdCommand::new(program).args(args).exec())
}

#[cfg(not(unix))]
pub fn exec_replace(program: &Path, args: &[String]) -> io::Result<ExitCode> {
    let status = StdCommand::new(program).args(args).status()?;

    match status.code().and_then(|code| u8::try_from(code).ok()) {
        Some(code) => Ok(ExitCode::from(code)),
        None if status.success() => Ok(ExitCode::SUCCESS),
        None => Ok(ExitCode::FAILURE),
    }
}
