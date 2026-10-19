use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use camino::Utf8Path;
use nix::unistd::{Uid, User};
#[cfg(not(target_os = "macos"))]
use rustix::process::test_kill_process_group;
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};

use super::{HookCommandContext, HookEnvironment, HookError, HookOutput, composer_env_overlay};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const START_MARKER: &[u8] = b"__PV_HOOK_ENV_START__\0";
const END_MARKER: &[u8] = b"__PV_HOOK_ENV_END__\0";
const CAPTURE_SCRIPT: &str = "cd \"$PV_HOOK_PROJECT\" && printf '__PV_HOOK_ENV_START__\\0' && /usr/bin/env -0 && printf '__PV_HOOK_ENV_END__\\0'";
// The pipe also stops the group if PV is killed without running Rust destructors.
const GROUP_ANCHOR: &str = "while IFS= read -r _line; do :; done\nkill -KILL 0\n";

#[expect(
    clippy::disallowed_types,
    reason = "Project hook runner owns child process execution"
)]
type HookCommand = tokio::process::Command;

pub fn login_shell() -> Result<PathBuf, HookError> {
    User::from_uid(Uid::current())
        .map_err(|error| environment_error(error.to_string(), &[]))?
        .map(|user| user.shell)
        .ok_or_else(|| environment_error("account record not found", &[]))
}

/// Captures once per run, independently of the shell used to execute commands.
pub async fn capture_environment(
    shell: &Path,
    home: &Utf8Path,
    project_path: &Utf8Path,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<HookEnvironment, HookError> {
    if *shutdown.borrow() {
        return Err(HookError::Interrupted);
    }
    let mut command = HookCommand::new(shell);
    command
        .args(["-i", "-l", "-c", CAPTURE_SCRIPT])
        .current_dir(home)
        .env("HOME", home)
        .env("PV_HOOK_PROJECT", project_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = HookProcess::spawn(&mut command)
        .map_err(|error| environment_error(error.to_string(), &[]))?;
    let child = process.child_mut()?;
    let mut stdout = child.stdout.take().ok_or_else(missing_output)?;
    let mut stderr = child.stderr.take().ok_or_else(missing_output)?;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let result = {
        let capture = async {
            let (status, _, _) = tokio::try_join!(
                process.wait(),
                stdout.read_to_end(&mut stdout_bytes),
                stderr.read_to_end(&mut stderr_bytes),
            )?;
            Ok::<_, io::Error>(status)
        };
        tokio::select! {
            result = timeout(CAPTURE_TIMEOUT, capture) => match result {
                Ok(Ok(status)) => Ok(status),
                Ok(Err(error)) => Err(HookError::Io(error)),
                Err(_) => Err(environment_error("timed out after 10 seconds", &[])),
            },
            () = cancelled(shutdown) => Err(HookError::Interrupted),
        }
    };
    let cleanup = process.stop().await;
    let result = match result {
        Ok(status) if status.success() => parse_environment(&stdout_bytes, &stderr_bytes),
        Ok(status) => Err(environment_error(status.to_string(), &stderr_bytes)),
        Err(HookError::Interrupted) => Err(HookError::Interrupted),
        Err(HookError::Environment { reason, .. }) => Err(environment_error(reason, &stderr_bytes)),
        Err(error) => Err(environment_error(error.to_string(), &stderr_bytes)),
    };
    finish_cleanup(result, cleanup)
}

fn missing_output() -> io::Error {
    io::Error::other("hook environment capture output was not piped")
}

fn parse_environment(stdout: &[u8], stderr: &[u8]) -> Result<HookEnvironment, HookError> {
    let Some(start) = stdout
        .windows(START_MARKER.len())
        .position(|bytes| bytes == START_MARKER)
    else {
        return Err(environment_error("missing start marker", stderr));
    };
    let remaining = &stdout[start + START_MARKER.len()..];
    let Some(end) = remaining
        .windows(END_MARKER.len())
        .position(|bytes| bytes == END_MARKER)
    else {
        return Err(environment_error("missing end marker", stderr));
    };
    let mut values = BTreeMap::new();
    for entry in remaining[..end]
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let Some(separator) = entry.iter().position(|byte| *byte == b'=') else {
            return Err(environment_error("invalid environment entry", stderr));
        };
        let key = &entry[..separator];
        if matches!(
            key,
            b"PWD" | b"OLDPWD" | b"SHLVL" | b"_" | b"PV_HOOK_PROJECT"
        ) {
            continue;
        }
        values.insert(
            OsString::from_vec(key.to_vec()),
            OsString::from_vec(entry[separator + 1..].to_vec()),
        );
    }
    Ok(HookEnvironment { values })
}

fn environment_error(reason: impl Into<String>, stderr: &[u8]) -> HookError {
    let tail = &stderr[stderr.len().saturating_sub(2048)..];
    let stderr = String::from_utf8_lossy(tail);
    let stderr = stderr.trim();
    HookError::Environment {
        reason: reason.into(),
        stderr: if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        },
    }
}

pub async fn run_commands(
    context: &HookCommandContext<'_>,
    phase: &str,
    commands: &[String],
    shutdown: &mut watch::Receiver<bool>,
    mut announce: impl FnMut(&str) -> io::Result<()>,
) -> Result<(), HookError> {
    let mut log = match context.output {
        HookOutput::Terminal => None,
        HookOutput::Log(path) => {
            let file = state::fs::open_private_log_file(path)?;
            file.set_len(0)?;
            Some(file)
        }
    };
    let overlay = composer_env_overlay(
        context.paths,
        context
            .environment
            .values
            .get(&OsString::from("PATH"))
            .map(OsString::as_os_str),
    )?;
    for script in commands {
        if *shutdown.borrow() {
            return Err(HookError::Interrupted);
        }
        let announcement = format!("Running {phase}: {script}");
        let mut command = HookCommand::new("/bin/sh");
        command
            .args(["-c", script])
            .current_dir(context.project_path)
            .env_clear()
            .envs(&context.environment.values)
            .envs(overlay.iter().cloned())
            .stdin(Stdio::null());
        if let Some(source) = context.source_path {
            command.env("PV_SOURCE_PATH", source);
        } else {
            command.env_remove("PV_SOURCE_PATH");
        }
        if let Some(log) = log.as_mut() {
            writeln!(log, "{announcement}")?;
            command.stdout(log.try_clone()?).stderr(log.try_clone()?);
        } else {
            announce(&announcement)?;
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }
        let mut process = HookProcess::spawn(&mut command)?;
        let result = tokio::select! {
            result = process.wait() => result.map_err(HookError::from),
            () = cancelled(shutdown) => Err(HookError::Interrupted),
        };
        let cleanup = process.stop().await;
        let result = result.and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(HookError::CommandFailed {
                    command: script.clone(),
                    status: status.to_string(),
                })
            }
        });
        finish_cleanup(result, cleanup)?;
    }
    Ok(())
}

fn finish_cleanup<T>(
    result: Result<T, HookError>,
    cleanup: io::Result<()>,
) -> Result<T, HookError> {
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(original), Err(cleanup)) => Err(HookError::Cleanup {
            original: Box::new(original),
            cleanup,
        }),
    }
}

async fn cancelled(shutdown: &mut watch::Receiver<bool>) {
    if shutdown.wait_for(|requested| *requested).await.is_err() {
        std::future::pending::<()>().await;
    }
}

struct HookProcess {
    anchor: Child,
    child: Option<Child>,
    group: Pid,
    // Once the anchor has been reaped, this group identifier could be reused.
    signalled: bool,
}

impl HookProcess {
    fn spawn(command: &mut HookCommand) -> io::Result<Self> {
        let anchor = HookCommand::new("/bin/sh")
            .args(["-c", GROUP_ANCHOR])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let group = anchor
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw)
            .ok_or_else(|| io::Error::other("hook process-group anchor has no process ID"))?;
        let mut process = Self {
            anchor,
            child: None,
            group,
            signalled: false,
        };
        process.child = Some(
            command
                .process_group(group.as_raw_nonzero().get())
                .kill_on_drop(true)
                .spawn()?,
        );
        Ok(process)
    }

    fn child_mut(&mut self) -> io::Result<&mut Child> {
        self.child
            .as_mut()
            .ok_or_else(|| io::Error::other("hook command has no child process"))
    }

    async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child_mut()?.wait().await
    }

    fn signal(&mut self) -> io::Result<()> {
        if !self.signalled {
            match kill_process_group(self.group, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => self.signalled = true,
                Err(rustix::io::Errno::PERM) if !group_is_running(self.group)? => {
                    self.signalled = true;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    async fn stop(&mut self) -> io::Result<()> {
        self.signal()?;
        if let Some(child) = self.child.as_mut() {
            child.wait().await?;
        }
        self.anchor.wait().await?;
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        while group_is_running(self.group)? {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "hook process group did not exit",
                ));
            }
            sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn group_is_running(group: Pid) -> io::Result<bool> {
    crate::process_group_has_live_members(group.as_raw_nonzero().get().unsigned_abs())
        .map_err(io::Error::other)
}

#[cfg(not(target_os = "macos"))]
fn group_is_running(group: Pid) -> io::Result<bool> {
    match test_kill_process_group(group) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::SRCH) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

impl Drop for HookProcess {
    fn drop(&mut self) {
        // Explicit stop reports errors. On cancellation, the shell anchor's owner pipe is the
        // fallback if signalling fails; the anchor must remain alive until that pipe closes.
        let _signal_result = self.signal();
        self.anchor.stdin.take();
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use std::process::Stdio;
    use std::time::Duration;

    use camino_tempfile::tempdir;
    use tokio::time::{sleep, timeout};

    use super::{HookCommand, HookError, HookProcess, finish_cleanup, parse_environment};

    #[test]
    fn capture_drops_shell_bookkeeping_without_changing_values() -> anyhow::Result<()> {
        let environment = parse_environment(
            b"__PV_HOOK_ENV_START__\0PWD=stale\0OLDPWD=stale\0SHLVL=99\0_=shell\0PV_HOOK_PROJECT=temporary\0MULTILINE=first\nsecond=value\0__PV_HOOK_ENV_END__\0",
            b"",
        )?;
        let values = environment.values.into_iter().collect::<Vec<_>>();
        insta::assert_debug_snapshot!(values);
        Ok(())
    }

    #[test]
    fn hook_cleanup_preserves_the_original_command_failure() -> anyhow::Result<()> {
        let failure = HookError::CommandFailed {
            command: "exit 7".to_string(),
            status: "exit status: 7".to_string(),
        };
        let result: Result<(), HookError> =
            finish_cleanup(Err(failure), Err(io::Error::other("could not stop group")));
        let Err(error @ HookError::Cleanup { .. }) = result else {
            anyhow::bail!("expected original and cleanup failures");
        };
        insta::assert_snapshot!(error.to_string());
        Ok(())
    }

    #[tokio::test]
    async fn hook_anchor_stops_its_group_when_the_owner_pipe_closes() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let mut command = HookCommand::new("/bin/sh");
        command
            .args(["-c", "/bin/sleep 150 & descendant=$!; printf '%s' \"$descendant\" > pid.tmp; mv pid.tmp pid; wait \"$descendant\""])
            .current_dir(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = HookProcess::spawn(&mut command)?;
        timeout(Duration::from_secs(5), async {
            while !state::fs::path_entry_exists(&directory.path().join("pid"))? {
                sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, state::StateError>(())
        })
        .await??;
        process.anchor.stdin.take();
        let status = timeout(Duration::from_secs(5), process.wait()).await??;
        process.stop().await?;
        assert!(!status.success());
        Ok(())
    }
}
