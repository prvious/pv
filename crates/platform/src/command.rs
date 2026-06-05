use std::io;
use std::process::ExitStatus;

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

#[expect(
    clippy::disallowed_types,
    reason = "platform system integration helper owns privileged process execution"
)]
type StdCommand = std::process::Command;

fn command_status(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    StdCommand::new(program).args(args).status()
}
