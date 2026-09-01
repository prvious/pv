use std::io;
#[cfg(target_os = "macos")]
use std::process::ExitStatus;
use std::process::Output;

use crate::PlatformError;

#[cfg(target_os = "macos")]
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
            status: command_failure_status(&output),
        })
    }
}

fn command_failure_status(output: &Output) -> String {
    let status = output.status.to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        status
    } else {
        format!("{status}: {stderr}")
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "platform system integration helper owns privileged process execution"
)]
type StdCommand = std::process::Command;

#[cfg(target_os = "macos")]
fn command_status(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    StdCommand::new(program).args(args).status()
}

fn command_output(program: &str, args: &[&str]) -> io::Result<Output> {
    StdCommand::new(program).args(args).output()
}

#[cfg(all(test, unix))]
mod tests {
    use crate::PlatformError;

    use super::run_system_command_output;

    #[test]
    fn command_failure_includes_non_empty_stderr() -> anyhow::Result<()> {
        let result = run_system_command_output(
            "/bin/sh",
            &["-c", "printf 'pf inspection failed\\n' >&2; exit 7"],
        );

        let Err(PlatformError::SystemIntegrationCommandStatus { status, .. }) = result else {
            anyhow::bail!("expected a system integration command status error");
        };
        assert_eq!(status, "exit status: 7: pf inspection failed");

        Ok(())
    }

    #[test]
    fn command_failure_omits_empty_stderr() -> anyhow::Result<()> {
        let result = run_system_command_output("/bin/sh", &["-c", "exit 9"]);

        let Err(PlatformError::SystemIntegrationCommandStatus { status, .. }) = result else {
            anyhow::bail!("expected a system integration command status error");
        };
        assert_eq!(status, "exit status: 9");

        Ok(())
    }
}
