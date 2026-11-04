#![cfg(target_os = "linux")]

use platform::clock::{self, real};
use std::io::{self, Write as _};
use std::process::Output;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use cli::{Environment, ProcessEnvironment};
use insta::{Settings, assert_snapshot};
use platform::{
    HELPER_EXECUTABLE_PATH, HELPER_METADATA_PATH, HELPER_PROTOCOL_VERSION, HELPER_SOCKET_PATH,
    PRIVILEGED_HELPER_VERSION, PrivilegedHelperClient, ProcessStartIdentity,
};
use serde_json::{Value, json};
use state::fs;

#[expect(
    clippy::disallowed_types,
    reason = "opt-in system fixture controls native helper lifecycle and diagnostics"
)]
type SystemTestCommand = std::process::Command;

const CONTROL_DIRECTORY: &str = "/run/pv-helper-system-test";
const EXECUTABLE_DIRECTORY: &str = "/usr/local/libexec/pv";
const WORK_DIRECTORY: &str = "/var/lib/pv";
const CANDIDATE: &str = "/usr/local/libexec/pv/pv-helper.candidate";
const LOCK: &str = "/usr/local/libexec/.pv-helper-lifecycle.lock";
const SOCKET_UNIT: &str = "/etc/systemd/system/pv-helper.socket";
const SERVICE_UNIT: &str = "/etc/systemd/system/pv-helper.service";
const DROP_IN_DIRECTORY: &str = "/etc/systemd/system/pv-helper.service.d";
const LOG: &str = "/var/log/pv-helper.err.log";
const ENABLED_LINKS: [&str; 2] = [
    "/etc/systemd/system/sockets.target.wants/pv-helper.socket",
    "/run/systemd/system/sockets.target.wants/pv-helper.socket",
];
const UNIT_ROLLBACK_PATHS: [&str; 2] = [
    "/etc/systemd/system/pv-helper.socket.rollback",
    "/etc/systemd/system/pv-helper.service.rollback",
];
const INSTALLATION_PATHS: [&str; 6] = [
    EXECUTABLE_DIRECTORY,
    WORK_DIRECTORY,
    SOCKET_UNIT,
    SERVICE_UNIT,
    HELPER_SOCKET_PATH,
    DROP_IN_DIRECTORY,
];

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, PV_LINUX_SYSTEM_HELPER and a disposable systemd machine"]
fn systemd_helper_lifecycle_authenticates_replaces_rolls_back_and_removes() -> Result<()> {
    let environment = ProcessEnvironment;
    if environment.var_os("PV_LINUX_SYSTEM_TESTS").as_deref() != Some("1".as_ref()) {
        return Ok(());
    }
    let helper = environment
        .var_os("PV_LINUX_SYSTEM_HELPER")
        .ok_or_else(|| anyhow!("PV_LINUX_SYSTEM_HELPER must name the built pv-helper"))?;
    let helper = Utf8PathBuf::from_path_buf(helper.into())
        .map_err(|path| anyhow!("non-UTF-8 helper executable: {path:?}"))?;
    ensure!(helper.is_absolute(), "helper executable must be absolute");
    let owner_uid: u32 = text(checked("/usr/bin/id", &["-u"])?)?.trim().parse()?;
    ensure!(owner_uid != 0, "run as the disposable non-root account");
    for path in INSTALLATION_PATHS
        .into_iter()
        .chain(ENABLED_LINKS)
        .chain(UNIT_ROLLBACK_PATHS)
        .chain([LOG, CONTROL_DIRECTORY])
    {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "refusing existing system path {path}"
        );
    }
    require_units_absent(&["pv-helper.socket", "pv-helper.service"])?;
    root(&["/usr/bin/true"])?;
    let mut fixture = HelperFixture {
        scratch: tempdir()?,
        owner_uid,
        cleaned: false,
        processes: Vec::new(),
    };
    let result = fixture.exercise(&helper);
    let cleanup = fixture.cleanup();
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

struct HelperFixture {
    scratch: Utf8TempDir,
    owner_uid: u32,
    cleaned: bool,
    processes: Vec<(u32, ProcessStartIdentity)>,
}

impl HelperFixture {
    fn exercise(&mut self, helper: &Utf8Path) -> Result<()> {
        let original = self.scratch.path().join("original/pv-helper");
        let replacement = self.scratch.path().join("replacement/pv-helper");
        fs::copy_file_atomically(helper, &original)?;
        fs::copy_file_atomically(helper, &replacement)?;
        // ELF ignores a trailer. Change the bytes without changing the helper's identity.
        fs::open_append_file(&replacement)?.write_all(b"\nPV lifecycle replacement fixture\n")?;
        let original_sha256 = checksum(&original)?;
        let replacement_sha256 = checksum(&replacement)?;
        ensure!(original_sha256 != replacement_sha256);
        root(&["/usr/bin/install", "-d", "-m", "0700", CONTROL_DIRECTORY])?;

        self.install(&original, &original_sha256)?;
        let first = self.capture_process()?;
        assert_eq!(
            PrivilegedHelperClient.status()?,
            PrivilegedHelperClient.status()?
        );
        assert_eq!(self.capture_process()?, first);
        assert_eq!(
            checksum(Utf8Path::new(HELPER_EXECUTABLE_PATH))?,
            original_sha256
        );
        for (path, expected) in [
            (HELPER_EXECUTABLE_PATH, "0:0:755"),
            (HELPER_METADATA_PATH, "0:0:600"),
            (WORK_DIRECTORY, "0:0:700"),
            (SOCKET_UNIT, "0:0:644"),
            (SERVICE_UNIT, "0:0:644"),
            (LOCK, "0:0:600"),
        ] {
            assert_eq!(
                root_text(&["/usr/bin/stat", "-c", "%u:%g:%a", path])?.trim(),
                expected
            );
        }
        assert_eq!(
            root_text(&["/usr/bin/stat", "-c", "%u:%a", HELPER_SOCKET_PATH])?.trim(),
            format!("{}:600", self.owner_uid)
        );
        let lock_inode = root_text(&["/usr/bin/stat", "-c", "%d:%i", LOCK])?;
        assert_eq!(
            fs::read_link(Utf8Path::new(ENABLED_LINKS[0]))?,
            Utf8Path::new(SOCKET_UNIT)
        );
        let metadata: Value =
            serde_json::from_str(&root_text(&["/usr/bin/cat", HELPER_METADATA_PATH])?)?;
        assert_eq!(
            metadata,
            json!({
                "owner_uid": self.owner_uid,
                "helper_version": "2.0.0",
                "protocol_version": 2,
            })
        );

        // Root can bypass socket permissions, so this specifically exercises SO_PEERCRED.
        // Send no request bytes: authentication must precede reading a frame.
        let response = root(&[
            "/usr/bin/python3",
            "-c",
            "import socket,sys\nwith socket.socket(socket.AF_UNIX) as client:\n client.settimeout(5)\n client.connect('/run/pv-helper.sock')\n sys.stdout.buffer.write(client.makefile('rb').readline(16385))\n",
        ])?;
        let response: Value = serde_json::from_slice(&response.stdout)?;
        assert_eq!(response["protocol_version"], 2);
        assert_eq!(response["outcome"]["status"], "error");
        assert_eq!(response["outcome"]["code"], "authentication_failed");
        self.require_other_owner_refused(&original, &original_sha256)?;
        assert_eq!(self.capture_process()?, first);

        self.install(&replacement, &replacement_sha256)?;
        let replaced = self.capture_process()?;
        assert_ne!(replaced, first);
        require_exited(first)?;
        assert_eq!(
            checksum(Utf8Path::new(HELPER_EXECUTABLE_PATH))?,
            replacement_sha256
        );

        self.root_file(
            "expected.sha256",
            &format!("{replacement_sha256}  {HELPER_EXECUTABLE_PATH}\n"),
            &format!("{CONTROL_DIRECTORY}/expected.sha256"),
            "0600",
        )?;
        root(&["/usr/bin/install", "-d", "-m", "0755", DROP_IN_DIRECTORY])?;
        self.root_file(
            "condition.conf",
            "[Service]\nEnvironment=LC_ALL=C\nExecStartPre=/usr/bin/sha256sum --check /run/pv-helper-system-test/expected.sha256\nStandardOutput=append:/run/pv-helper-system-test/condition.log\n",
            &format!("{DROP_IN_DIRECTORY}/10-pv-system-test.conf"),
            "0644",
        )?;
        let before = root_text(&[
            "/usr/bin/sha256sum",
            HELPER_EXECUTABLE_PATH,
            HELPER_METADATA_PATH,
            SOCKET_UNIT,
            SERVICE_UNIT,
        ])?;
        let failure = self
            .install(&original, &original_sha256)
            .err()
            .context("replacement must fail its activation checksum")?;
        let restored = self.capture_process().with_context(|| {
            format!("helper did not recover after failed replacement: {failure:#}")
        })?;
        assert_ne!(restored, replaced);
        require_exited(replaced)?;
        assert_eq!(
            root_text(&[
                "/usr/bin/sha256sum",
                HELPER_EXECUTABLE_PATH,
                HELPER_METADATA_PATH,
                SOCKET_UNIT,
                SERVICE_UNIT,
            ])?,
            before
        );
        let condition = root_text(&[
            "/usr/bin/cat",
            &format!("{CONTROL_DIRECTORY}/condition.log"),
        ])?;
        let mut lines = condition.lines().collect::<Vec<_>>();
        lines.dedup();
        assert_snapshot!(lines.join("\n"), @r"
        /usr/local/libexec/pv/pv-helper: FAILED
        /usr/local/libexec/pv/pv-helper: OK
        ");
        self.remove_condition()?;

        ensure!(
            fs::path_entry_exists(Utf8Path::new(LOG))?,
            "helper log was not created"
        );
        platform::remove_privileged_helper()?;
        require_exited(restored)?;
        self.require_removed()?;
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(LOG))?,
            "helper log remains after removal"
        );
        root(&["/usr/bin/install", "-d", "-m", "0755", EXECUTABLE_DIRECTORY])?;
        root(&["/usr/bin/install", "-d", "-m", "0700", WORK_DIRECTORY])?;
        self.root_file(
            "leftover-candidate",
            "interrupted install",
            CANDIDATE,
            "0755",
        )?;
        platform::remove_privileged_helper()?;
        self.require_removed()?;
        assert_eq!(
            root_text(&["/usr/bin/stat", "-c", "%d:%i", LOCK])?,
            lock_inode
        );
        Ok(())
    }

    fn install(&self, candidate: &Utf8Path, checksum: &str) -> Result<()> {
        let outcome = platform::install_privileged_helper(
            candidate,
            self.scratch.path(),
            checksum,
            PRIVILEGED_HELPER_VERSION,
            HELPER_PROTOCOL_VERSION,
        )?;
        assert_eq!(outcome.status().owner_uid, self.owner_uid);
        assert_eq!(outcome.status().version, "2.0.0");
        assert_eq!(outcome.status().protocol_version, 2);
        ensure!(outcome.cleanup_warning().is_none(), "{outcome:?}");
        Ok(())
    }

    fn require_other_owner_refused(&self, original: &Utf8Path, checksum: &str) -> Result<()> {
        let installed = root_text(&[
            "/usr/bin/sha256sum",
            HELPER_EXECUTABLE_PATH,
            HELPER_METADATA_PATH,
            SOCKET_UNIT,
            SERVICE_UNIT,
        ])?;
        root(&[
            "/usr/bin/install",
            "-o",
            "root",
            "-g",
            "root",
            "-m",
            "0755",
            original.as_str(),
            CANDIDATE,
        ])?;
        let other_uid = self.owner_uid.checked_add(1).context("test UID range")?;
        let output = SystemTestCommand::new("sudo")
            .args([
                "-n",
                CANDIDATE,
                "helper-lifecycle",
                "replace",
                "--owner-uid",
                &other_uid.to_string(),
                "--sha256",
                checksum,
                "--helper-version",
                "2.0.0",
                "--protocol-version",
                "2",
            ])
            .output()?;
        ensure!(
            !output.status.success(),
            "another owner replaced the helper"
        );
        let mut settings = Settings::clone_current();
        settings.add_filter(
            &format!("installing UID {}", self.owner_uid),
            "installing UID [owner]",
        );
        settings.add_filter(
            &format!("setting up UID {other_uid}"),
            "setting up UID [other]",
        );
        let _guard = settings.bind_to_scope();
        assert_snapshot!(String::from_utf8(output.stderr)?, @"Error: PV privileged helper installation failed: helper belongs to installing UID [owner]; uninstall it from that account before setting up UID [other]");
        assert_eq!(
            root_text(&[
                "/usr/bin/sha256sum",
                HELPER_EXECUTABLE_PATH,
                HELPER_METADATA_PATH,
                SOCKET_UNIT,
                SERVICE_UNIT,
            ])?,
            installed
        );
        Ok(())
    }

    fn capture_process(&mut self) -> Result<(u32, ProcessStartIdentity)> {
        let status = PrivilegedHelperClient.status()?;
        assert_eq!(status.owner_uid, self.owner_uid);
        assert_eq!(status.version, "2.0.0");
        assert_eq!(status.protocol_version, 2);
        assert_eq!(
            service_property("pv-helper.socket", "ActiveState")?,
            "active"
        );
        self.capture_service_process()
    }

    fn capture_service_process(&mut self) -> Result<(u32, ProcessStartIdentity)> {
        assert_eq!(
            service_property("pv-helper.service", "ActiveState")?,
            "active"
        );
        let pid: u32 = service_property("pv-helper.service", "MainPID")?.parse()?;
        let birth =
            platform::inspect_process_start_identity(pid)?.context("helper process missing")?;
        let process = (pid, birth);
        if !self.processes.contains(&process) {
            self.processes.push(process);
        }
        Ok(process)
    }

    fn root_file(&self, name: &str, content: &str, destination: &str, mode: &str) -> Result<()> {
        let source = self.scratch.path().join(name);
        fs::write_sensitive_file(&source, content)?;
        root(&[
            "/usr/bin/install",
            "-o",
            "root",
            "-g",
            "root",
            "-m",
            mode,
            source.as_str(),
            destination,
        ])?;
        Ok(())
    }

    fn remove_condition(&self) -> Result<()> {
        if fs::path_entry_exists(Utf8Path::new(DROP_IN_DIRECTORY))? {
            root(&["/usr/bin/rm", "-rf", "--", DROP_IN_DIRECTORY])?;
            root(&["/usr/bin/systemctl", "daemon-reload"])?;
        }
        Ok(())
    }

    fn require_removed(&self) -> Result<()> {
        for path in INSTALLATION_PATHS
            .into_iter()
            .chain(ENABLED_LINKS)
            .chain(UNIT_ROLLBACK_PATHS)
        {
            ensure!(
                !fs::path_entry_exists(Utf8Path::new(path))?,
                "helper path remains: {path}"
            );
        }
        for process in &self.processes {
            require_exited(*process)?;
        }
        require_units_absent(&["pv-helper.socket", "pv-helper.service"])
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        self.remove_condition()?;
        platform::remove_privileged_helper()?;
        self.require_removed()?;
        root(&["/usr/bin/rm", "-f", "--", LOG])?;
        root(&["/usr/bin/rm", "-rf", "--", CONTROL_DIRECTORY])?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for HelperFixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _reported = writeln!(
                io::stderr().lock(),
                "helper fixture cleanup failed: {error:#}"
            );
        }
    }
}

fn require_exited((pid, birth): (u32, ProcessStartIdentity)) -> Result<()> {
    let deadline = real::now() + Duration::from_secs(2);
    while platform::inspect_process_start_identity(pid)? == Some(birth) {
        ensure!(
            real::now() < deadline,
            "helper process {pid} remained present"
        );
        clock::poll_blocking(Duration::from_millis(10));
    }
    Ok(())
}

fn service_property(unit: &str, property: &str) -> Result<String> {
    Ok(text(checked(
        "/usr/bin/systemctl",
        &["show", "--value", "--property", property, unit],
    )?)?
    .trim()
    .to_owned())
}

fn checksum(path: &Utf8Path) -> Result<String> {
    let output = text(checked("/usr/bin/sha256sum", &[path.as_str()])?)?;
    output
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .context("checksum output")
}

fn root(arguments: &[&str]) -> Result<Output> {
    let mut command = SystemTestCommand::new("sudo");
    command.arg("-n").args(arguments);
    checked_output(&mut command)
}

fn root_text(arguments: &[&str]) -> Result<String> {
    text(root(arguments)?)
}

fn checked(program: &str, arguments: &[&str]) -> Result<Output> {
    checked_output(SystemTestCommand::new(program).args(arguments))
}

fn require_units_absent(units: &[&str]) -> Result<()> {
    for operation in ["list-unit-files", "list-units"] {
        let mut command = SystemTestCommand::new("/usr/bin/systemctl");
        command
            .args([operation, "--all", "--no-legend", "--no-pager", "--plain"])
            .args(units);
        let output = command.output()?;
        // Systemd 249 returns ENOENT for an empty unit-file listing.
        ensure!(
            (output.status.success()
                || (operation == "list-unit-files" && output.status.code() == Some(1)))
                && String::from_utf8_lossy(&output.stdout).trim().is_empty()
                && output.stderr.is_empty(),
            "cannot establish absent units with {command:?}: {}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

fn checked_output(command: &mut SystemTestCommand) -> Result<Output> {
    let output = command.output()?;
    ensure!(
        output.status.success(),
        "{command:?} failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn text(output: Output) -> Result<String> {
    Ok(String::from_utf8(output.stdout)?)
}
