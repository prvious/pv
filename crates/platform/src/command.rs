use std::io;
use std::process::{ExitStatus, Output};

use crate::PlatformError;

pub(crate) fn run_system_command(program: &str, args: &[&str]) -> Result<(), PlatformError> {
    let command = format!("{program} {}", args.join(" "));
    let status = command_status(program, args).map_err(|source| {
        PlatformError::SystemIntegrationCommand {
            command: command.clone(),
            source,
        }
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::SystemIntegrationCommandStatus {
            command,
            status: status.to_string(),
        })
    }
}

pub(crate) fn run_system_command_output(
    program: &str,
    args: &[&str],
) -> Result<String, PlatformError> {
    let command = format!("{program} {}", args.join(" "));
    let output = command_output(program, args).map_err(|source| {
        PlatformError::SystemIntegrationCommand {
            command: command.clone(),
            source,
        }
    })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(PlatformError::SystemIntegrationCommandStatus {
            command,
            status: output.status.to_string(),
        })
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "platform system integration helper owns privileged process execution"
)]
type StdCommand = std::process::Command;

fn command_status(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    StdCommand::new(program).args(args).status()
}

fn command_output(program: &str, args: &[&str]) -> io::Result<Output> {
    StdCommand::new(program).args(args).output()
}
