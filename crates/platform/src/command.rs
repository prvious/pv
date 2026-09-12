use std::io::{self, Read};
#[cfg(target_os = "macos")]
use std::process::ExitStatus;
use std::process::{Child, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wait_timeout::ChildExt;

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

    system_command_output(command, output)
}

pub(crate) fn run_system_command_output_with_timeout(
    program: &str,
    args: &[&str],
    wait: Duration,
) -> Result<String, PlatformError> {
    let command = format!("{program} {}", args.join(" "));
    let output = command_output_with_timeout(program, args, wait).map_err(|source| {
        PlatformError::SystemIntegrationCommand {
            command: command.clone(),
            source,
        }
    })?;

    system_command_output(command, output)
}

fn system_command_output(command: String, output: Output) -> Result<String, PlatformError> {
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

fn command_output_with_timeout(program: &str, args: &[&str], wait: Duration) -> io::Result<Output> {
    let mut child = StdCommand::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap(&mut child)?;
        return Err(io::Error::other("command stdout was not piped"));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_and_reap(&mut child)?;
        return Err(io::Error::other("command stderr was not piped"));
    };
    let stdout_reader = match spawn_output_reader(stdout) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_and_reap(&mut child)?;
            return Err(error);
        }
    };
    let stderr_reader = match spawn_output_reader(stderr) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_and_reap(&mut child)?;
            let _ = finish_output_reader(stdout_reader);
            return Err(error);
        }
    };

    let status = match child.wait_timeout(wait) {
        Ok(Some(status)) => status,
        Ok(None) => {
            terminate_and_reap(&mut child)?;
            let _ = finish_output_readers(stdout_reader, stderr_reader);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command exceeded {} ms", wait.as_millis()),
            ));
        }
        Err(error) => {
            if terminate_and_reap(&mut child).is_ok() {
                let _ = finish_output_readers(stdout_reader, stderr_reader);
            }
            return Err(error);
        }
    };
    let (stdout, stderr) = finish_output_readers(stdout_reader, stderr_reader)?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_output_reader(
    mut output: impl Read + Send + 'static,
) -> io::Result<JoinHandle<io::Result<Vec<u8>>>> {
    thread::Builder::new()
        .name("platform-command-output".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();
            output.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
}

fn finish_output_readers(
    stdout_reader: JoinHandle<io::Result<Vec<u8>>>,
    stderr_reader: JoinHandle<io::Result<Vec<u8>>>,
) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let stdout = finish_output_reader(stdout_reader);
    let stderr = finish_output_reader(stderr_reader);
    Ok((stdout?, stderr?))
}

fn finish_output_reader(reader: JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("command output reader panicked"))?
}

fn terminate_and_reap(child: &mut Child) -> io::Result<()> {
    if let Err(error) = child.kill()
        && child.try_wait()?.is_none()
    {
        return Err(error);
    }
    child.wait()?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::io;
    use std::time::Duration;

    use camino_tempfile::tempdir;
    use state::fs;

    use crate::PlatformError;

    use super::{StdCommand, run_system_command_output, run_system_command_output_with_timeout};

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

    #[test]
    fn bounded_command_preserves_successful_output() -> anyhow::Result<()> {
        let output = run_system_command_output_with_timeout(
            "/bin/sh",
            &["-c", "printf bounded"],
            Duration::from_secs(1),
        )?;

        assert_eq!(output, "bounded");

        Ok(())
    }

    #[test]
    fn bounded_command_drains_large_output() -> anyhow::Result<()> {
        let output = run_system_command_output_with_timeout(
            "/bin/sh",
            &["-c", "yes e | head -c 262144 >&2; yes o | head -c 262144"],
            Duration::from_secs(5),
        )?;

        assert_eq!(output.len(), 262_144);

        Ok(())
    }

    #[test]
    fn command_timeout_kills_and_reaps_child() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let pid_path = tempdir.path().join("pid");
        let result = run_system_command_output_with_timeout(
            "/bin/sh",
            &[
                "-c",
                "printf '%s' $$ > \"$1\"; exec /bin/sleep 30",
                "sh",
                pid_path.as_str(),
            ],
            Duration::from_millis(100),
        );

        let Err(PlatformError::SystemIntegrationCommand { source, .. }) = result else {
            anyhow::bail!("expected a command timeout, got {result:?}");
        };
        assert_eq!(source.kind(), io::ErrorKind::TimedOut);
        let pid = fs::read_to_string(&pid_path)?;
        let status = StdCommand::new("/bin/kill")
            .args(["-0", pid.as_str()])
            .status()?;
        assert!(!status.success());

        Ok(())
    }
}
