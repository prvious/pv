#![cfg(unix)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Stdio};
use std::time::Duration;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
#[cfg(target_os = "macos")]
use platform::ProcessEvents;
use platform::clock;
use platform::{
    ProcessExitWatch, inspect_process_identity, inspect_process_start_identity,
    process_group_has_live_members, process_is_zombie,
};
use state::fs;
#[cfg(target_os = "linux")]
use tokio::io::{Interest, unix::AsyncFd};

#[expect(
    clippy::disallowed_types,
    reason = "platform integration tests spawn controlled processes for native inspection"
)]
type TestCommand = std::process::Command;

#[test]
fn native_process_identity_reports_direct_executable_and_ordered_arguments() -> Result<()> {
    let mut child = ChildGuard(TestCommand::new("/bin/sleep").arg("30").spawn()?);
    let identity = inspect_child(&mut child)?;

    assert_eq!(
        identity.executable,
        fs::canonicalize(Utf8Path::new("/bin/sleep"))?
    );
    assert_eq!(identity.argument_zero, "/bin/sleep");
    assert_eq!(identity.arguments, ["30"]);
    assert!(identity.start_identity.seconds > 0);
    assert!(identity.start_identity.microseconds < 1_000_000);
    assert_eq!(
        inspect_process_start_identity(child.0.id())?,
        Some(identity.start_identity)
    );

    Ok(())
}

#[test]
fn native_process_identity_preserves_shell_command_as_one_argument() -> Result<()> {
    let command = "kill -STOP $$";
    let mut child = ChildGuard(TestCommand::new("/bin/sh").args(["-c", command]).spawn()?);
    clock::poll_blocking(Duration::from_millis(50));
    let identity = inspect_child(&mut child)?;

    assert!(identity.executable.is_absolute());
    assert_eq!(identity.argument_zero, "/bin/sh");
    assert_eq!(identity.arguments, ["-c", command]);

    Ok(())
}

#[test]
fn native_process_identity_reports_shebang_script_and_ordered_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let script = tempdir.path().join("owned-runtime");
    state::fs::write_sensitive_file(&script, "#!/bin/sh\nkill -STOP $$\n")?;
    set_executable(&script)?;
    let mut child = ChildGuard(TestCommand::new(&script).args(["one", "two"]).spawn()?);
    clock::poll_blocking(Duration::from_millis(50));
    let identity = inspect_child(&mut child)?;

    assert!(identity.executable.is_absolute());
    assert_eq!(identity.argument_zero, "/bin/sh");
    assert_eq!(
        identity.arguments,
        [script.to_string(), "one".to_string(), "two".to_string()]
    );

    Ok(())
}

#[test]
#[cfg(target_os = "macos")]
fn process_watch_reports_exec_then_exit_without_reaping() -> Result<()> {
    // `/bin/sh` would re-exec itself as bash, so start bash directly: its only exec is the one the
    // test releases by writing a line.
    let mut child = ChildGuard(
        TestCommand::new("/bin/bash")
            .args(["-c", "read line; exec /bin/sleep 30"])
            .stdin(Stdio::piped())
            .spawn()?,
    );
    let pid = child.0.id();
    let birth = inspect_process_start_identity(pid)?;
    let mut watch = ProcessExitWatch::with_exec(pid)?;

    assert_eq!(watch.try_events()?, ProcessEvents::default());
    child
        .0
        .stdin
        .take()
        .ok_or_else(|| anyhow!("bash stdin was not piped"))?
        .write_all(b"go\n")?;
    let exec = wait_for_events(&mut watch, |events| events.exec)?;
    let identity = inspect_child(&mut child)?;
    child.0.kill()?;
    let exit = wait_for_events(&mut watch, |events| events.exit.is_some())?;

    assert!(exec.exec);
    assert_eq!(identity.executable, Utf8Path::new("/bin/sleep"));
    assert_eq!(Some(identity.start_identity), birth);
    assert_eq!(exit.exit.and_then(|status| status.signal()), Some(9));
    assert!(process_is_zombie(pid)?);
    assert_eq!(child.0.wait()?.signal(), Some(9));

    Ok(())
}

#[cfg(target_os = "macos")]
fn wait_for_events(
    watch: &mut ProcessExitWatch,
    done: impl Fn(&ProcessEvents) -> bool,
) -> Result<ProcessEvents> {
    for _attempt in 0..500 {
        let events = watch.try_events()?;
        if done(&events) {
            return Ok(events);
        }
        clock::poll_blocking(Duration::from_millis(10));
    }

    Err(anyhow!("process events never arrived"))
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pidfd_readiness_holds_exact_exit_status_until_the_parent_reaps() -> Result<()> {
    for killed in [false, true] {
        let mut child = ChildGuard(
            TestCommand::new("/bin/bash")
                .args(["-c", "read line; exit 37"])
                .stdin(Stdio::piped())
                .spawn()?,
        );
        let pid = child.0.id();
        let mut watch = AsyncFd::with_interest(ProcessExitWatch::new(pid)?, Interest::READABLE)?;
        assert!(watch.get_mut().try_exit_status()?.is_none());
        if killed {
            child.0.kill()?;
        } else {
            child
                .0
                .stdin
                .take()
                .ok_or_else(|| anyhow!("missing child stdin"))?
                .write_all(b"go\n")?;
        }
        {
            let _ready = clock::real::timeout(Duration::from_secs(5), watch.readable()).await??;
        }
        let status = watch
            .get_mut()
            .try_exit_status()?
            .ok_or_else(|| anyhow!("missing exit status"))?;
        assert_eq!(status.code(), if killed { None } else { Some(37) });
        assert_eq!(status.signal(), killed.then_some(9));
        assert_eq!(watch.get_mut().try_exit_status()?, Some(status));
        assert!(process_is_zombie(pid)?);
        assert_eq!(child.0.wait()?, status);
        assert!(inspect_process_start_identity(pid)?.is_none());
    }
    Ok(())
}

#[test]
fn group_inspection_keeps_live_members_after_the_leader_exits() -> Result<()> {
    let mut leader = ChildGuard(
        TestCommand::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .spawn()?,
    );
    let process_group = leader.0.id();
    let mut member = ChildGuard(
        TestCommand::new("/bin/sleep")
            .arg("30")
            .process_group(i32::try_from(process_group)?)
            .spawn()?,
    );
    let mut leader_watch = ProcessExitWatch::new(process_group)?;
    let mut member_watch = ProcessExitWatch::new(member.0.id())?;

    assert!(process_group_has_live_members(process_group)?);
    leader.0.kill()?;
    wait_for_exit(&mut leader_watch)?;
    assert!(process_is_zombie(process_group)?);
    assert!(process_group_has_live_members(process_group)?);
    member.0.kill()?;
    wait_for_exit(&mut member_watch)?;
    assert!(!process_group_has_live_members(process_group)?);
    leader.0.wait()?;
    member.0.wait()?;
    assert!(!process_group_has_live_members(process_group)?);
    assert!(inspect_process_identity(process_group)?.is_none());
    assert!(inspect_process_start_identity(process_group)?.is_none());
    assert!(!process_is_zombie(process_group)?);
    Ok(())
}

fn wait_for_exit(watch: &mut ProcessExitWatch) -> Result<std::process::ExitStatus> {
    for _attempt in 0..500 {
        if let Some(status) = watch.try_exit_status()? {
            return Ok(status);
        }
        clock::poll_blocking(Duration::from_millis(10));
    }
    Err(anyhow!("process exit never arrived"))
}

fn inspect_child(child: &mut ChildGuard) -> Result<platform::ProcessIdentity> {
    let pid = child.0.id();
    // On Linux, `spawn` returns once the child execs, a moment before the kernel publishes its
    // arguments.
    for _attempt in 0..500 {
        if let Ok(Some(identity)) = inspect_process_identity(pid) {
            return Ok(identity);
        }
        clock::poll_blocking(Duration::from_millis(10));
    }

    inspect_process_identity(pid)?.ok_or_else(|| anyhow!("process {pid} had no native identity"))
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _kill_result = self.0.kill();
        let _wait_result = self.0.wait();
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "platform integration test marks a controlled shebang fixture executable"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}
