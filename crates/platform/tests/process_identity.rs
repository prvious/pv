#![cfg(target_os = "macos")]

use std::process::Child;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use platform::inspect_process_identity;

#[expect(
    clippy::disallowed_types,
    reason = "platform integration tests spawn controlled processes for native inspection"
)]
type TestCommand = std::process::Command;

#[test]
fn native_process_identity_reports_direct_executable_and_ordered_arguments() -> Result<()> {
    let mut child = ChildGuard(TestCommand::new("/bin/sleep").arg("30").spawn()?);
    let identity = inspect_child(&mut child)?;

    assert_eq!(identity.executable, Utf8Path::new("/bin/sleep"));
    assert_eq!(identity.argument_zero, "/bin/sleep");
    assert_eq!(identity.arguments, ["30"]);
    assert!(identity.start_identity.seconds > 0);
    assert!(identity.start_identity.microseconds < 1_000_000);

    Ok(())
}

#[test]
fn native_process_identity_preserves_shell_command_as_one_argument() -> Result<()> {
    let command = "kill -STOP $$";
    let mut child = ChildGuard(TestCommand::new("/bin/sh").args(["-c", command]).spawn()?);
    thread::sleep(Duration::from_millis(50));
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
    thread::sleep(Duration::from_millis(50));
    let identity = inspect_child(&mut child)?;

    assert!(identity.executable.is_absolute());
    assert_eq!(identity.argument_zero, "/bin/sh");
    assert_eq!(
        identity.arguments,
        [script.to_string(), "one".to_string(), "two".to_string()]
    );

    Ok(())
}

fn inspect_child(child: &mut ChildGuard) -> Result<platform::ProcessIdentity> {
    let pid = child.0.id();

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
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}
