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
#[cfg(target_os = "linux")]
use procfs::{ProcError, ProcResult, process::all_processes_with_root};
use rustix::fs::{Mode, OFlags, open};
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use rustix::process::test_kill_process_group;
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};

use super::{HookCommandContext, HookEnvironment, HookError, HookOutput, composer_env_overlay};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
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
                read_environment_output(&mut stdout, &mut stdout_bytes),
                read_stderr_tail(&mut stderr, &mut stderr_bytes),
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
    finish_cleanup(result, cleanup, *shutdown.borrow())
}

fn missing_output() -> io::Error {
    io::Error::other("hook environment capture output was not piped")
}

async fn read_environment_output(
    output: &mut (impl AsyncRead + Unpin),
    bytes: &mut Vec<u8>,
) -> io::Result<()> {
    let mut buffer = [0; 8192];
    let mut started = false;
    let mut complete = false;
    loop {
        let length = output.read(&mut buffer).await?;
        if length == 0 {
            return Ok(());
        }
        if complete {
            continue;
        }
        bytes.extend_from_slice(&buffer[..length]);
        if !started {
            if let Some(start) = bytes
                .windows(START_MARKER.len())
                .position(|window| window == START_MARKER)
            {
                bytes.drain(..start);
                started = true;
            } else {
                bytes.drain(..bytes.len().saturating_sub(START_MARKER.len() - 1));
                continue;
            }
        }
        if let Some(end) = environment_end(&bytes[START_MARKER.len()..]) {
            bytes.truncate(START_MARKER.len() + end + END_MARKER.len());
            complete = true;
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
    let directory = state::fs::open_read_file(context.project_path)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "hook Project directory is missing")
    })?;
    // Keep the directory itself across renames, and its descriptor away from stdio slots.
    let directory = rustix::io::fcntl_dupfd_cloexec(&directory, 3).map_err(io::Error::from)?;
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
            .env_clear()
            .envs(&context.environment.values)
            .envs(overlay.iter().cloned())
            .stdin(Stdio::null());
        let command_directory =
            rustix::io::fcntl_dupfd_cloexec(&directory, 3).map_err(io::Error::from)?;
        // SAFETY: The child-only callback performs just fchdir on its owned live descriptor,
        // with no allocation or locks. It changes no parent state, and exec closes the descriptor.
        unsafe {
            command.pre_exec(move || {
                rustix::process::fchdir(&command_directory).map_err(io::Error::from)
            });
        }
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

#[cfg(target_os = "linux")]
fn group_is_running(group: Pid) -> io::Result<bool> {
    linux_group_is_running(Path::new("/proc"), group)
}

#[cfg(target_os = "linux")]
fn linux_group_is_running(root: &Path, group: Pid) -> io::Result<bool> {
    let mut previous_zombies = None;
    for _attempt in 0..8 {
        let mut zombies = Vec::new();
        for process in all_processes_with_root(root).map_err(io::Error::other)? {
            let Some(process) = existing_process(process)? else {
                continue;
            };
            let Some(status) = existing_process(process.stat())? else {
                continue;
            };
            if status.pgrp != group.as_raw_nonzero().get() {
                continue;
            }
            if status.state != 'Z' {
                return Ok(true);
            }
            // A Linux thread-group leader can be a zombie while another thread still runs.
            // Inspect all tasks and require the zombie snapshot to stay stable.
            let Some(tasks) = existing_process(process.tasks())? else {
                if existing_process(process.stat())?.is_some() {
                    return Err(io::Error::other("hook process-group tasks unavailable"));
                }
                continue;
            };
            let mut inspected_task = false;
            for task in tasks {
                let Some(task) = existing_process(task)? else {
                    continue;
                };
                let Some(status) = existing_process(task.stat())? else {
                    continue;
                };
                inspected_task = true;
                if status.state != 'Z' {
                    return Ok(true);
                }
                zombies.push((status.pid, status.starttime));
            }
            if !inspected_task && existing_process(process.stat())?.is_some() {
                return Err(io::Error::other("hook process-group tasks unavailable"));
            }
        }
        zombies.sort_unstable();
        if zombies.is_empty() || previous_zombies.as_ref() == Some(&zombies) {
            return Ok(false);
        }
        previous_zombies = Some(zombies);
    }
    Err(io::Error::other(
        "hook process-group snapshot did not stabilize",
    ))
}

#[cfg(target_os = "linux")]
fn existing_process<T>(result: ProcResult<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(ProcError::NotFound(_)) => Ok(None),
        Err(error) => Err(io::Error::other(error)),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

    #[cfg(target_os = "linux")]
    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    #[cfg(target_os = "linux")]
    use rustix::process::Pid;
    use tokio::time::{sleep, timeout};

    #[cfg(target_os = "linux")]
    use super::linux_group_is_running;
    use super::{
        END_MARKER, HookCommand, HookError, HookProcess, STDERR_TAIL_LENGTH, finish_cleanup,
        parse_environment, read_environment_output, read_stderr_tail,
    };

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_group_checks_threads_after_the_leader_exits() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let leader = directory.path().join("123");
        write_process_stat(&leader, 123, 'Z')?;
        write_process_stat(&leader.join("task/123"), 123, 'Z')?;
        write_process_stat(&leader.join("task/124"), 124, 'S')?;
        let group = Pid::from_raw(123).ok_or_else(|| anyhow::anyhow!("invalid fixture group"))?;
        assert!(linux_group_is_running(
            directory.path().as_std_path(),
            group
        )?);
        write_process_stat(&leader.join("task/124"), 124, 'Z')?;
        assert!(!linux_group_is_running(
            directory.path().as_std_path(),
            group
        )?);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_group_does_not_treat_missing_tasks_as_stopped() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let leader = directory.path().join("123");
        write_process_stat(&leader, 123, 'Z')?;
        let group = Pid::from_raw(123).ok_or_else(|| anyhow::anyhow!("invalid fixture group"))?;
        let mut errors = Vec::new();
        for empty_directory in [false, true] {
            if empty_directory {
                state::fs::ensure_user_dir(&leader.join("task"))?;
            }
            let error = linux_group_is_running(directory.path().as_std_path(), group)
                .err()
                .ok_or_else(|| anyhow::anyhow!("expected uncertain task inspection"))?;
            errors.push(error.to_string());
        }
        insta::assert_debug_snapshot!(errors, @r#"
        [
            "hook process-group tasks unavailable",
            "hook process-group tasks unavailable",
        ]
        "#);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn write_process_stat(directory: &Utf8Path, pid: i32, status: char) -> anyhow::Result<()> {
        state::fs::ensure_user_dir(directory)?;
        let mut fields = vec!["0".to_string(); 52];
        fields[0] = pid.to_string();
        fields[1] = "(fixture)".to_string();
        fields[2] = status.to_string();
        fields[4] = "123".to_string();
        fields[21] = "42".to_string();
        state::fs::write_sensitive_file(&directory.join("stat"), &fields.join(" "))?;
        Ok(())
    }

    #[tokio::test]
    async fn capture_discards_noise_and_retains_only_the_stderr_tail() -> anyhow::Result<()> {
        let frame = b"__PV_HOOK_ENV_START__\0FIRST=prefix__PV_HOOK_ENV_END__\0AFTER=retained\0__PV_HOOK_ENV_END__\0";
        // Split the start marker across reads, then keep draining after the end marker.
        for padding in [
            8192 * 3 - 7,
            8192 * 3 - (frame.len() - END_MARKER.len()) - 7,
        ] {
            let mut output = vec![b'x'; padding];
            output.extend_from_slice(frame);
            output.extend(vec![b'y'; 8192 * 3]);
            let mut captured = Vec::new();
            read_environment_output(&mut output.as_slice(), &mut captured).await?;
            assert_eq!(captured, frame);
        }

        let mut stderr = vec![b'x'; 8192 * 3];
        stderr.extend(vec![b't'; STDERR_TAIL_LENGTH]);
        let mut tail = Vec::new();
        read_stderr_tail(&mut stderr.as_slice(), &mut tail).await?;
        assert_eq!(tail, vec![b't'; STDERR_TAIL_LENGTH]);
        Ok(())
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
