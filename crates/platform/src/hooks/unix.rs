use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use camino::Utf8Path;
use nix::libc;
use nix::unistd::{Uid, User};
use rustix::fs::{Mode, OFlags, open};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader, copy, sink};
use tokio::process::Child;
use tokio::sync::watch;

use super::{HookCommandContext, HookEnvironment, HookError, HookOutput, composer_env_overlay};
use crate::clock::{self, Deadline, Limit};

/// How long a login shell gets to print its environment before PV stops it.
pub const CAPTURE_TIMEOUT: Limit = Limit::new("hook environment capture", Duration::from_secs(10));
const CLEANUP_TIMEOUT: Limit = Limit::new("hook process-group cleanup", Duration::from_secs(5));
const STDERR_TAIL_LENGTH: usize = 2048;
const START_MARKER: &[u8] = b"__PV_HOOK_ENV_START__\0";
const END_MARKER: &[u8] = b"__PV_HOOK_ENV_END__\0";
const CAPTURE_SCRIPT: &str = "cd \"$PV_HOOK_PROJECT\" && printf '__PV_HOOK_ENV_START__\\0' && /usr/bin/env -0 && printf '__PV_HOOK_ENV_END__\\0'";
// The pipe also stops the group if PV is killed without running Rust destructors.
const GROUP_ANCHOR: &str = "while IFS= read -r _line; do :; done\nkill -KILL 0\n";

nix::ioctl_none_bad!(detach_terminal, libc::TIOCNOTTY);

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
    let mut stdout = BufReader::new(child.stdout.take().ok_or_else(missing_output)?);
    let mut stderr = child.stderr.take().ok_or_else(missing_output)?;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let result = {
        let capture = async {
            let capture = async {
                read_environment_output(&mut stdout, &mut stdout_bytes).await?;
                // Discard exit-time output so the shell never blocks on a full pipe, but don't
                // wait for end-of-file: a background job may keep stdout open.
                let mut discard = sink();
                let mut drained = false;
                loop {
                    tokio::select! {
                        status = process.wait() => break status,
                        result = copy(&mut stdout, &mut discard), if !drained => {
                            result?;
                            drained = true;
                        }
                    }
                }
            };
            tokio::pin!(capture);
            tokio::select! {
                biased;
                result = read_stderr_tail(&mut stderr, &mut stderr_bytes) => {
                    result?;
                    capture.await
                }
                result = &mut capture => result,
            }
        };
        tokio::select! {
            result = clock::timeout(&CAPTURE_TIMEOUT, capture) => match result {
                Ok(Ok(status)) => Ok(status),
                Ok(Err(error)) => Err(HookError::Io(error)),
                Err(_) => Err(environment_error(
                    format!(
                        "timed out after {} seconds",
                        CAPTURE_TIMEOUT.duration().as_secs()
                    ),
                    &[],
                )),
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
    finish_cleanup(result, cleanup, *shutdown.borrow())
}

fn detach_controlling_terminal(command: &mut HookCommand) {
    // SAFETY: This child-only callback uses only open/ioctl/close and errno access, without
    // allocation or locks. The child is not its session leader, so TIOCNOTTY detaches only it
    // from the inherited terminal; the descriptor remains owned until the ioctl finishes.
    unsafe {
        command.pre_exec(|| {
            let terminal = match open(
                c"/dev/tty",
                OFlags::RDONLY | OFlags::NOCTTY | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(terminal) => terminal,
                Err(rustix::io::Errno::NXIO) => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            detach_terminal(terminal.as_raw_fd()).map_err(io::Error::from)?;
            Ok(())
        });
    }
}

fn missing_output() -> io::Error {
    io::Error::other("hook environment capture output was not piped")
}

async fn read_environment_output(
    output: &mut (impl AsyncBufRead + Unpin),
    bytes: &mut Vec<u8>,
) -> io::Result<()> {
    let mut started = false;
    loop {
        let start = bytes.len();
        if output.read_until(0, bytes).await? == 0 {
            return Ok(());
        }
        started |= bytes.ends_with(START_MARKER);
        if started && &bytes[start..] == END_MARKER {
            return Ok(());
        }
    }
}

async fn read_stderr_tail(
    output: &mut (impl AsyncRead + Unpin),
    bytes: &mut Vec<u8>,
) -> io::Result<()> {
    let mut buffer = [0; 8192];
    loop {
        let length = output.read(&mut buffer).await?;
        if length == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..length]);
        bytes.drain(..bytes.len().saturating_sub(STDERR_TAIL_LENGTH));
    }
}

fn environment_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(END_MARKER.len())
        .enumerate()
        .find_map(|(index, window)| {
            (window == END_MARKER && (index == 0 || bytes[index - 1] == 0)).then_some(index)
        })
}

fn parse_environment(stdout: &[u8], stderr: &[u8]) -> Result<HookEnvironment, HookError> {
    let Some(start) = stdout
        .windows(START_MARKER.len())
        .position(|bytes| bytes == START_MARKER)
    else {
        return Err(environment_error("missing start marker", stderr));
    };
    let remaining = &stdout[start + START_MARKER.len()..];
    let Some(end) = environment_end(remaining) else {
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
    let tail = &stderr[stderr.len().saturating_sub(STDERR_TAIL_LENGTH)..];
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
        finish_cleanup(result, cleanup, *shutdown.borrow())?;
    }
    Ok(())
}

fn finish_cleanup<T>(
    result: Result<T, HookError>,
    cleanup: io::Result<()>,
    interrupted: bool,
) -> Result<T, HookError> {
    let result = result.and_then(|value| {
        if interrupted {
            Err(HookError::Interrupted)
        } else {
            Ok(value)
        }
    });
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(HookError::CleanupFailed(error)),
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
        detach_controlling_terminal(command);
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
            child.start_kill()?;
            child.wait().await?;
        }
        self.anchor.wait().await?;
        let deadline = Deadline::after(&CLEANUP_TIMEOUT);
        while group_is_running(self.group)? {
            if deadline.is_expired() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "hook process group {} did not exit",
                        self.group.as_raw_nonzero()
                    ),
                ));
            }
            clock::poll(Duration::from_millis(10)).await;
        }
        Ok(())
    }
}

fn group_is_running(group: Pid) -> io::Result<bool> {
    crate::process_group_has_live_members(group.as_raw_nonzero().get().unsigned_abs()).map_err(
        |error| {
            io::Error::other(format!(
                "hook process group {}: {error}",
                group.as_raw_nonzero()
            ))
        },
    )
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

    #[cfg(target_os = "macos")]
    use std::process::Stdio;
    #[cfg(target_os = "macos")]
    use std::time::Duration;

    #[cfg(target_os = "macos")]
    use anyhow::Context;
    #[cfg(target_os = "macos")]
    use camino_tempfile::tempdir;
    #[cfg(target_os = "macos")]
    use tokio::io::{AsyncBufReadExt, BufReader};
    #[cfg(target_os = "macos")]
    use tokio::time::{sleep, timeout};

    #[cfg(target_os = "macos")]
    use super::{HookCommand, HookProcess, group_is_running};
    use super::{HookError, finish_cleanup, parse_environment};

    #[test]
    fn cleanup_failure_after_success_remains_distinguishable() {
        let result = finish_cleanup(Ok(()), Err(io::Error::other("could not stop group")), false);
        assert!(matches!(result, Err(HookError::CleanupFailed(_))));
    }

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
        let result: Result<(), HookError> = finish_cleanup(
            Err(failure),
            Err(io::Error::other("could not stop group")),
            false,
        );
        let Err(error @ HookError::Cleanup { .. }) = result else {
            anyhow::bail!("expected original and cleanup failures");
        };
        insta::assert_snapshot!(error.to_string());
        Ok(())
    }

    #[test]
    fn interruption_during_final_cleanup_cannot_report_success() {
        assert!(matches!(
            finish_cleanup(Ok(()), Ok(()), true),
            Err(HookError::Interrupted)
        ));
        assert!(matches!(
            finish_cleanup(Ok(()), Err(io::Error::other("could not stop group")), true),
            Err(HookError::Cleanup { original, .. }) if matches!(*original, HookError::Interrupted)
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn hook_cleanup_kills_a_direct_child_that_left_the_group() -> anyhow::Result<()> {
        let mut command = HookCommand::new("python3");
        command
            .args([
                "-c",
                "import os,time; os.setsid(); print('ready', flush=True); time.sleep(150)",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut process = HookProcess::spawn(&mut command)?;
        let stdout = process.child_mut()?.stdout.take().context("child stdout")?;
        let mut ready = String::new();
        timeout(
            Duration::from_secs(5),
            BufReader::new(stdout).read_line(&mut ready),
        )
        .await??;
        assert_eq!(ready, "ready\n");
        timeout(Duration::from_secs(5), process.stop()).await??;
        Ok(())
    }

    #[cfg(target_os = "macos")]
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
        let mut process = HookProcess::spawn(&mut command).context("spawn anchored command")?;
        timeout(Duration::from_secs(5), async {
            while !state::fs::path_entry_exists(&directory.path().join("pid"))? {
                sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, state::StateError>(())
        })
        .await??;
        process.anchor.stdin.take();
        let status = timeout(Duration::from_secs(5), process.wait())
            .await
            .context("wait for the command after owner-pipe closure")?
            .context("read the command exit after owner-pipe closure")?;
        // Verify the fallback itself stopped every member before parent cleanup can help.
        // The unreaped anchor keeps the group identifier owned throughout this check.
        timeout(Duration::from_secs(5), async {
            while group_is_running(process.group)? {
                sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, io::Error>(())
        })
        .await
        .context("wait for the anchor to stop its group")?
        .context("inspect the anchor's group")?;
        process
            .stop()
            .await
            .context("reap the stopped anchor and command")?;
        assert!(!status.success());
        Ok(())
    }
}
