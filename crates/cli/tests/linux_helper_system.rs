#![cfg(target_os = "linux")]

#[path = "support/dns_system.rs"]
mod dns_system;

use std::io::{self, Write as _};
use std::process::Output;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use cli::{Environment, ProcessEnvironment};
use insta::{Settings, allow_duplicates, assert_snapshot};
use platform::{
    HELPER_EXECUTABLE_PATH, HELPER_METADATA_PATH, HELPER_PROTOCOL_VERSION, HELPER_SOCKET_PATH,
    PRIVILEGED_HELPER_VERSION, PrivilegedHelperClient, ProcessStartIdentity,
};
use serde_json::{Deserializer, Value, json};
use state::fs;

#[expect(
    clippy::disallowed_types,
    reason = "opt-in system fixture owns native service diagnostics and its independent cleanup timer"
)]
type SystemTestCommand = std::process::Command;

const TEST_SERVICE: &str = "pv-helper-system-test.service";
const CLEANUP_UNIT: &str = "pv-helper-system-cleanup";
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
const UNRELATED_CANDIDATES: [(&str, &str); 3] = [
    ("/var/lib/pv/helper.json.candidate", "0600"),
    ("/etc/systemd/system/pv-helper.socket.candidate", "0644"),
    ("/etc/systemd/system/pv-helper.service.candidate", "0644"),
];
const INSTALLATION_PATHS: [&str; 6] = [
    EXECUTABLE_DIRECTORY,
    WORK_DIRECTORY,
    SOCKET_UNIT,
    SERVICE_UNIT,
    HELPER_SOCKET_PATH,
    DROP_IN_DIRECTORY,
];

// This runs outside the test's cgroup after its hard lifetime limit. Stop that entire
// cgroup first, so a surviving sudo lifecycle child cannot race filesystem cleanup.
const CLEANUP_SCRIPT: &str = r#"set -eu
/usr/bin/systemctl --no-block stop pv-helper-system-test.service pv-helper.socket pv-helper.service || :
/usr/bin/systemctl kill --kill-who=all --signal=KILL pv-helper-system-test.service pv-helper.service || :
stopped=false
for _attempt in 1 2 3 4 5 6 7 8 9 10; do
    remaining=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain \
        --state=active,activating,deactivating,reloading \
        pv-helper-system-test.service pv-helper.socket pv-helper.service) || exit 1
    if [ -z "$remaining" ]; then stopped=true; break; fi
    sleep 0.5
done
if [ "$stopped" != true ]; then
    printf '%s\n' 'fixture cgroups did not stop; retaining recovery files' >&2
    exit 1
fi
for name in pv-helper.socket pv-helper.service; do
    failed=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain --state=failed "$name")
    if [ -n "$failed" ]; then /usr/bin/systemctl reset-failed "$name"; fi
done
# Empty-path and native-unit preflight proved that these belong only to this fixture.
rm -rf -- /usr/local/libexec/pv /var/lib/pv /etc/systemd/system/pv-helper.service.d
rm -f -- /etc/systemd/system/pv-helper.socket /etc/systemd/system/pv-helper.service \
    /etc/systemd/system/pv-helper.socket.rollback /etc/systemd/system/pv-helper.service.rollback \
    /etc/systemd/system/pv-helper.socket.candidate /etc/systemd/system/pv-helper.service.candidate \
    /run/pv-helper.sock /var/log/pv-helper.err.log \
    /etc/systemd/system/sockets.target.wants/pv-helper.socket \
    /run/systemd/system/sockets.target.wants/pv-helper.socket
/usr/bin/systemctl daemon-reload
remaining=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain pv-helper.socket pv-helper.service)
if [ -n "$remaining" ]; then
    printf '%s\n' 'fixture helper units remain loaded after cleanup' >&2
    exit 1
fi
# Keep the root-owned control directory and journal for post-mortem inspection.
"#;

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, PV_LINUX_SYSTEM_HELPER and the dedicated disposable systemd test service"]
fn systemd_helper_lifecycle_authenticates_replaces_rolls_back_and_removes() -> Result<()> {
    with_helper_fixture(HelperFixture::exercise)
}

fn with_helper_fixture(exercise: fn(&mut HelperFixture, &Utf8Path) -> Result<()>) -> Result<()> {
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
    require_dedicated_service(owner_uid)?;
    for path in INSTALLATION_PATHS
        .into_iter()
        .chain(ENABLED_LINKS)
        .chain(UNIT_ROLLBACK_PATHS)
        .chain(UNRELATED_CANDIDATES.map(|(path, _mode)| path))
        .chain([LOG, CONTROL_DIRECTORY])
    {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "refusing existing system path {path}"
        );
    }
    require_units_absent(&[
        "pv-helper.socket",
        "pv-helper.service",
        "pv-helper-system-cleanup.timer",
        "pv-helper-system-cleanup.service",
    ])?;
    root(&["/usr/bin/true"])?;
    let mut fixture = HelperFixture {
        scratch: tempdir()?,
        owner_uid,
        control_created: false,
        timer_attempted: false,
        cleaned: false,
        processes: Vec::new(),
    };
    let result = exercise(&mut fixture, &helper);
    let cleanup = fixture.cleanup();
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn require_dedicated_service(owner_uid: u32) -> Result<()> {
    for (property, expected) in [
        ("User", owner_uid.to_string()),
        ("KillMode", "control-group".to_owned()),
        ("RuntimeMaxUSec", "2min".to_owned()),
        ("TimeoutStopUSec", "5s".to_owned()),
    ] {
        ensure!(
            service_property(TEST_SERVICE, property)? == expected,
            "dedicated test service must have {property}={expected}"
        );
    }
    let control_group = service_property(TEST_SERVICE, "ControlGroup")?;
    ensure!(
        control_group.ends_with("/pv-helper-system-test.service"),
        "unexpected test control group {control_group}"
    );
    let groups = fs::read_to_string(Utf8Path::new("/proc/self/cgroup"))?;
    ensure!(
        groups
            .lines()
            .any(|line| line.splitn(3, ':').nth(2) == Some(control_group.as_str())),
        "test is outside its bounded systemd service"
    );
    let root_groups = root_text(&["/usr/bin/cat", "/proc/self/cgroup"])?;
    ensure!(
        root_groups
            .lines()
            .any(|line| line.splitn(3, ':').nth(2) == Some(control_group.as_str())),
        "sudo children escape the test service; refusing this test environment"
    );
    Ok(())
}

struct HelperFixture {
    scratch: Utf8TempDir,
    owner_uid: u32,
    control_created: bool,
    timer_attempted: bool,
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
        self.arm_cleanup()?;

        root(&["/usr/bin/install", "-d", "-m", "0755", DROP_IN_DIRECTORY])?;
        self.root_file(
            "first-install-failure.sh",
            "printf 'started\\n' > /run/pv-helper-system-test/first-install-started\nexec /usr/bin/sleep 150\n",
            &format!("{CONTROL_DIRECTORY}/first-install-failure.sh"),
            "0700",
        )?;
        self.root_file(
            "first-install-failure.conf",
            "[Service]\nExecStartPre=/bin/sh /run/pv-helper-system-test/first-install-failure.sh\n",
            &format!("{DROP_IN_DIRECTORY}/10-pv-system-test.conf"),
            "0644",
        )?;
        let failure = self
            .install(&original, &original_sha256)
            .err()
            .context("initial activation must fail")?;
        assert_eq!(
            root_text(&[
                "/usr/bin/cat",
                &format!("{CONTROL_DIRECTORY}/first-install-started")
            ])?,
            "started\n"
        );
        self.remove_condition()?;
        self.require_removed()
            .with_context(|| format!("initial installation failed: {failure:#}"))?;
        platform::remove_privileged_helper()?;

        root(&[
            "/usr/bin/install",
            "-D",
            "-o",
            "root",
            "-g",
            "root",
            "-m",
            "0755",
            original.as_str(),
            HELPER_EXECUTABLE_PATH,
        ])?;
        root(&["/usr/bin/install", "-d", "-m", "0700", WORK_DIRECTORY])?;
        let partial_metadata = serde_json::to_string_pretty(&json!({
            "owner_uid": self.owner_uid,
            "helper_version": PRIVILEGED_HELPER_VERSION,
            "protocol_version": HELPER_PROTOCOL_VERSION,
        }))?;
        self.root_file(
            "partial-metadata.json",
            &partial_metadata,
            HELPER_METADATA_PATH,
            "0600",
        )?;
        root(&["/usr/bin/install", "-d", "-m", "0755", DROP_IN_DIRECTORY])?;
        self.root_file(
            "partial-install-failure.sh",
            "printf 'partial\\n' > /run/pv-helper-system-test/first-install-started\nexit 1\n",
            &format!("{CONTROL_DIRECTORY}/first-install-failure.sh"),
            "0700",
        )?;
        self.root_file(
            "partial-install-failure.conf",
            "[Service]\nExecStartPre=/bin/sh /run/pv-helper-system-test/first-install-failure.sh\n",
            &format!("{DROP_IN_DIRECTORY}/10-pv-system-test.conf"),
            "0644",
        )?;
        let partial_failure = self
            .install(&replacement, &replacement_sha256)
            .err()
            .context("partial installation activation must fail")?;
        assert_eq!(
            root_text(&[
                "/usr/bin/cat",
                &format!("{CONTROL_DIRECTORY}/first-install-started")
            ])?,
            "partial\n"
        );
        self.remove_condition()?;
        assert_eq!(
            checksum(Utf8Path::new(HELPER_EXECUTABLE_PATH))?,
            original_sha256
        );
        assert_eq!(
            root_text(&["/usr/bin/cat", HELPER_METADATA_PATH])?,
            partial_metadata
        );
        for path in [SOCKET_UNIT, SERVICE_UNIT] {
            ensure!(
                !fs::path_entry_exists(Utf8Path::new(path))?,
                "introduced unit remains after partial rollback: {path}"
            );
        }
        self.install(&original, &original_sha256).with_context(|| {
            format!("retry after partial installation failed: {partial_failure:#}")
        })?;
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

        let unrelated_candidate_content = "unrelated root-owned candidate\n";
        for (path, mode) in UNRELATED_CANDIDATES {
            self.root_file(
                "unrelated-candidate",
                unrelated_candidate_content,
                path,
                mode,
            )?;
        }
        self.install(&replacement, &replacement_sha256)?;
        let replaced = self.capture_process()?;
        assert_ne!(replaced, first);
        require_exited(first)?;
        for (path, _mode) in UNRELATED_CANDIDATES {
            assert_eq!(
                root_text(&["/usr/bin/cat", path])
                    .with_context(|| format!("could not read unrelated candidate {path}"))?,
                unrelated_candidate_content
            );
            root(&["/usr/bin/rm", "--", path])?;
        }
        assert_eq!(
            checksum(Utf8Path::new(HELPER_EXECUTABLE_PATH))?,
            replacement_sha256
        );
        assert_eq!(
            root_text(&["/usr/bin/stat", "-c", "%d:%i", LOCK])?,
            lock_inode
        );

        self.root_file(
            "expected.sha256",
            &format!("{replacement_sha256}  {HELPER_EXECUTABLE_PATH}\n"),
            &format!("{CONTROL_DIRECTORY}/expected.sha256"),
            "0600",
        )?;
        // Exercise helper recovery after repeated startup failures hit systemd's start limit.
        root(&["/usr/bin/install", "-d", "-m", "0755", DROP_IN_DIRECTORY])?;
        self.root_file(
            "condition.conf",
            "[Unit]\nStartLimitIntervalSec=60\nStartLimitBurst=10\n[Service]\nEnvironment=LC_ALL=C\nExecStartPre=/usr/bin/sha256sum --check /run/pv-helper-system-test/expected.sha256\nStandardOutput=append:/run/pv-helper-system-test/condition.log\n",
            &format!("{DROP_IN_DIRECTORY}/10-pv-system-test.conf"),
            "0644",
        )?;
        let expected_installation = root_text(&[
            "/usr/bin/sha256sum",
            HELPER_EXECUTABLE_PATH,
            HELPER_METADATA_PATH,
            SOCKET_UNIT,
            SERVICE_UNIT,
        ])?;
        root(&[
            "/usr/bin/systemctl",
            "stop",
            "pv-helper.socket",
            "pv-helper.service",
        ])?;
        require_exited(replaced)?;
        let limit_window = Instant::now();
        root(&[
            "/usr/bin/systemctl",
            "reset-failed",
            "pv-helper.socket",
            "pv-helper.service",
        ])?;
        self.root_file(
            "expected.sha256",
            &format!("{original_sha256}  {HELPER_EXECUTABLE_PATH}\n"),
            &format!("{CONTROL_DIRECTORY}/expected.sha256"),
            "0600",
        )?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        root(&["/usr/bin/systemctl", "start", "pv-helper.socket"])?;
        for _attempt in 0..11 {
            let failure = SystemTestCommand::new("sudo")
                .args(["-n", "/usr/bin/systemctl", "start", "pv-helper.service"])
                .output()?;
            ensure!(
                !failure.status.success() && failure.status.code().is_some(),
                "installed helper must fail its checksum condition: {failure:?}"
            );
        }
        assert_eq!(
            service_property("pv-helper.service", "ActiveState")?,
            "failed"
        );
        assert_eq!(
            service_property("pv-helper.socket", "ActiveState")?,
            "failed"
        );
        assert_eq!(
            service_property("pv-helper.socket", "Result")?,
            "service-start-limit-hit"
        );
        self.root_file(
            "expected.sha256",
            &format!("{replacement_sha256}  {HELPER_EXECUTABLE_PATH}\n"),
            &format!("{CONTROL_DIRECTORY}/expected.sha256"),
            "0600",
        )?;
        self.install(&replacement, &replacement_sha256)
            .context("first valid replacement must repair the retained start limit")?;
        ensure!(
            limit_window.elapsed() < Duration::from_secs(60),
            "repair must finish before the start-limit interval expires"
        );
        assert_ne!(self.capture_process()?, replaced);
        assert_eq!(
            root_text(&[
                "/usr/bin/sha256sum",
                HELPER_EXECUTABLE_PATH,
                HELPER_METADATA_PATH,
                SOCKET_UNIT,
                SERVICE_UNIT,
            ])?,
            expected_installation
        );
        for (socket_active, service_active) in [(true, true), (false, true), (true, false)] {
            let prior_process = self.capture_process()?;
            if !socket_active {
                root(&["/usr/bin/systemctl", "stop", "pv-helper.socket"])?;
                assert_eq!(
                    service_property("pv-helper.socket", "ActiveState")?,
                    "inactive"
                );
                ensure!(!fs::path_entry_exists(Utf8Path::new(HELPER_SOCKET_PATH))?);
                assert_eq!(self.capture_service_process()?, prior_process);
            }
            if !service_active {
                root(&["/usr/bin/systemctl", "stop", "pv-helper.service"])?;
                require_exited(prior_process)?;
                assert_eq!(
                    service_property("pv-helper.socket", "ActiveState")?,
                    "active"
                );
                assert_eq!(
                    service_property("pv-helper.service", "LoadState")?,
                    "loaded"
                );
                assert_eq!(
                    service_property("pv-helper.service", "ActiveState")?,
                    "inactive"
                );
                assert_eq!(service_property("pv-helper.service", "MainPID")?, "0");
            }
            self.root_file(
                "empty-condition.log",
                "",
                &format!("{CONTROL_DIRECTORY}/condition.log"),
                "0600",
            )?;
            root(&["/usr/bin/journalctl", "--sync"])?;
            let last_entry: Value = serde_json::from_str(&root_text(&[
                "/usr/bin/journalctl",
                "--boot",
                "--lines=1",
                "--output=json",
                "--quiet",
                "--no-pager",
            ])?)?;
            let cursor = last_entry["__CURSOR"]
                .as_str()
                .context("journal must expose a cursor before the rejected replacement")?;
            let failure = self
                .install(&original, &original_sha256)
                .err()
                .context("replacement condition must reject the changed helper")?;
            root(&["/usr/bin/journalctl", "--sync"])?;
            let journal = root_text(&[
                "/usr/bin/journalctl",
                "--boot",
                "--after-cursor",
                cursor,
                "--unit=pv-helper.socket",
                "--output=json",
                "--quiet",
                "--no-pager",
            ])?;
            let entries = Deserializer::from_str(&journal)
                .into_iter::<Value>()
                .collect::<Result<Vec<_>, _>>()?;
            ensure!(
                entries
                    .iter()
                    .any(|entry| entry["UNIT_RESULT"] == "service-start-limit-hit"),
                "replacement must hit the service start limit: {journal}"
            );
            let condition = root_text(&[
                "/usr/bin/cat",
                &format!("{CONTROL_DIRECTORY}/condition.log"),
            ])?;
            let restored = if service_active {
                let restored = self.capture_service_process().with_context(|| {
                    format!("helper did not recover after rejected replacement: {failure:#}")
                })?;
                assert_ne!(restored, prior_process);
                Some(restored)
            } else {
                assert_eq!(
                    service_property("pv-helper.service", "LoadState")?,
                    "loaded"
                );
                assert_eq!(
                    service_property("pv-helper.service", "ActiveState")?,
                    "inactive",
                    "rollback must restore the idle helper service"
                );
                assert_eq!(service_property("pv-helper.service", "MainPID")?, "0");
                None
            };
            require_exited(prior_process)?;
            assert_eq!(
                service_property("pv-helper.socket", "ActiveState")?,
                if socket_active { "active" } else { "inactive" }
            );
            assert_eq!(
                fs::path_entry_exists(Utf8Path::new(HELPER_SOCKET_PATH))?,
                socket_active
            );
            let mut condition_lines = condition.lines().collect::<Vec<_>>();
            condition_lines.dedup();
            allow_duplicates! {
                assert_snapshot!(condition_lines.join("\n"), @r"
                /usr/local/libexec/pv/pv-helper: FAILED
                /usr/local/libexec/pv/pv-helper: OK
                ");
            }
            assert_eq!(
                checksum(Utf8Path::new(HELPER_EXECUTABLE_PATH))?,
                replacement_sha256
            );
            assert_eq!(
                serde_json::from_str::<Value>(&root_text(&[
                    "/usr/bin/cat",
                    HELPER_METADATA_PATH
                ])?)?,
                metadata
            );
            assert_eq!(
                fs::read_link(Utf8Path::new(ENABLED_LINKS[0]))?,
                Utf8Path::new(SOCKET_UNIT)
            );
            for path in UNIT_ROLLBACK_PATHS {
                ensure!(
                    !fs::path_entry_exists(Utf8Path::new(path))?,
                    "helper unit rollback remains after recovery: {path}"
                );
            }
            if let Some(restored) = restored
                && !socket_active
            {
                root(&["/usr/bin/systemctl", "stop", "pv-helper.service"])?;
                require_exited(restored)?;
                root(&["/usr/bin/systemctl", "start", "pv-helper.socket"])?;
            }
            self.capture_process()?;
        }
        self.remove_condition()?;

        root(&["/usr/bin/install", "-d", "-m", "0755", DROP_IN_DIRECTORY])?;
        self.root_file(
            "cleanup-warning.conf",
            "[Service]\nExecStartPre=/usr/bin/chmod 0644 /usr/local/libexec/pv/pv-helper.rollback\n",
            &format!("{DROP_IN_DIRECTORY}/10-pv-system-test.conf"),
            "0644",
        )?;
        let outcome = platform::install_privileged_helper(
            &original,
            self.scratch.path(),
            &original_sha256,
            PRIVILEGED_HELPER_VERSION,
            HELPER_PROTOCOL_VERSION,
        )?;
        ensure!(outcome.cleanup_warning().is_some(), "{outcome:?}");
        self.capture_process()?;
        assert_eq!(
            checksum(Utf8Path::new(HELPER_EXECUTABLE_PATH))?,
            original_sha256
        );
        assert_eq!(
            checksum(Utf8Path::new(&format!("{HELPER_EXECUTABLE_PATH}.rollback")))?,
            replacement_sha256
        );
        self.remove_condition()?;
        for path in [
            HELPER_EXECUTABLE_PATH,
            HELPER_METADATA_PATH,
            SOCKET_UNIT,
            SERVICE_UNIT,
        ] {
            root(&["/usr/bin/rm", "--", &format!("{path}.rollback")])?;
        }

        for directory in [WORK_DIRECTORY, EXECUTABLE_DIRECTORY] {
            let live = self.capture_process()?;
            let unrelated = format!("{directory}/unrelated-system-test-file");
            let content = "unrelated root-owned content\n";
            self.root_file("unrelated", content, &unrelated, "0600")?;
            let failure = platform::remove_privileged_helper()
                .err()
                .context("removal must preserve unrelated directory contents")?;
            assert_eq!(
                root_text(&["/usr/bin/cat", &unrelated])?,
                content,
                "unrelated content changed after removal failed: {failure:#}"
            );
            for path in [
                HELPER_EXECUTABLE_PATH,
                HELPER_METADATA_PATH,
                SOCKET_UNIT,
                SERVICE_UNIT,
                HELPER_SOCKET_PATH,
                LOG,
            ]
            .into_iter()
            .chain(ENABLED_LINKS)
            .chain(UNIT_ROLLBACK_PATHS)
            {
                root(&["/usr/bin/test", "!", "-e", path])
                    .with_context(|| format!("helper cleanup did not reach {path}: {failure:#}"))?;
            }
            require_units_absent(&["pv-helper.socket", "pv-helper.service"])
                .with_context(|| format!("helper cleanup did not finish: {failure:#}"))?;
            require_exited(live)?;

            // Administrator recovery belongs to the fixture, not the removal retry contract.
            root(&["/usr/bin/rm", "--", &unrelated])?;
            for directory in [WORK_DIRECTORY, EXECUTABLE_DIRECTORY] {
                if fs::path_entry_exists(Utf8Path::new(directory))? {
                    root(&["/usr/bin/rmdir", "--", directory])?;
                }
            }
            self.require_removed()?;
            self.install(&original, &original_sha256)?;
        }

        root(&[
            "/usr/bin/install",
            "-o",
            "root",
            "-g",
            "root",
            "-m",
            "0755",
            HELPER_EXECUTABLE_PATH,
            CANDIDATE,
        ])?;
        let owner_uid = self.owner_uid.to_string();
        let removal_arguments = [
            CANDIDATE,
            "helper-lifecycle",
            "remove",
            "--owner-uid",
            &owner_uid,
            "--sha256",
            &original_sha256,
            "--helper-version",
            "1.0.0",
            "--protocol-version",
            "2",
        ];
        let mut mismatched_metadata = metadata.clone();
        mismatched_metadata["helper_version"] = json!("1.0.0");
        self.root_file(
            "mismatched-metadata.json",
            &serde_json::to_string_pretty(&mismatched_metadata)?,
            HELPER_METADATA_PATH,
            "0600",
        )?;
        let before = root_text(&[
            "/usr/bin/sha256sum",
            HELPER_EXECUTABLE_PATH,
            HELPER_METADATA_PATH,
            SOCKET_UNIT,
            SERVICE_UNIT,
        ])?;
        let live = self.capture_process()?;
        let refused = SystemTestCommand::new("sudo")
            .arg("-n")
            .args(removal_arguments)
            .output()?;
        ensure!(
            !refused.status.success(),
            "mismatched installed identity was removed"
        );
        assert_eq!(
            root_text(&[
                "/usr/bin/sha256sum",
                HELPER_EXECUTABLE_PATH,
                HELPER_METADATA_PATH,
                SOCKET_UNIT,
                SERVICE_UNIT
            ])?,
            before
        );
        assert_eq!(self.capture_process()?, live);
        self.root_file(
            "restored-metadata.json",
            &serde_json::to_string_pretty(&metadata)?,
            HELPER_METADATA_PATH,
            "0600",
        )?;
        ensure!(
            fs::path_entry_exists(Utf8Path::new(LOG))?,
            "installed helper service did not create its error log"
        );
        root(&[
            "/usr/bin/systemctl",
            "kill",
            "--kill-who=main",
            "--signal=KILL",
            "pv-helper.service",
        ])?;
        require_exited(live)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let active_state = service_property("pv-helper.service", "ActiveState")?;
            if active_state == "failed" {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "killed helper did not reach failed state: {active_state}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let response = root(&removal_arguments)?;
        assert_eq!(
            serde_json::from_slice::<Option<String>>(&response.stdout)?,
            None
        );
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(LOG))?,
            "helper error log remains after successful removal"
        );
        self.require_removed()?;
        platform::remove_privileged_helper()?;
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

    fn arm_cleanup(&mut self) -> Result<()> {
        root(&["/usr/bin/install", "-d", "-m", "0700", CONTROL_DIRECTORY])?;
        self.control_created = true;
        self.root_file(
            "cleanup.sh",
            CLEANUP_SCRIPT,
            &format!("{CONTROL_DIRECTORY}/cleanup.sh"),
            "0700",
        )?;
        self.timer_attempted = true;
        root(&[
            "/usr/bin/systemd-run",
            "--collect",
            "--on-active=150s",
            "--unit",
            CLEANUP_UNIT,
            "--property=RuntimeMaxSec=30s",
            "/bin/sh",
            &format!("{CONTROL_DIRECTORY}/cleanup.sh"),
        ])?;
        Ok(())
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
            .chain(UNRELATED_CANDIDATES.map(|(path, _mode)| path))
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
        if self.cleaned || !self.control_created {
            return Ok(());
        }
        self.remove_condition()?;
        for (path, _mode) in UNRELATED_CANDIDATES {
            root(&["/usr/bin/rm", "-f", "--", path])?;
        }
        platform::remove_privileged_helper()?;
        self.require_removed()?;
        root(&["/usr/bin/rm", "-f", "--", LOG])?;
        if self.timer_attempted {
            root(&[
                "/usr/bin/systemctl",
                "stop",
                &format!("{CLEANUP_UNIT}.timer"),
            ])?;
        }
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
    let deadline = Instant::now() + Duration::from_secs(2);
    while platform::inspect_process_start_identity(pid)? == Some(birth) {
        ensure!(
            Instant::now() < deadline,
            "helper process {pid} remained present"
        );
        thread::sleep(Duration::from_millis(10));
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
