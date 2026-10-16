#![cfg(target_os = "linux")]

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot, assert_snapshot};
use platform::{
    ActivePfRedirectInspection, DaemonRegistrationConfig, DaemonRegistrationFileState,
    DaemonRegistrationStatus, HELPER_PROTOCOL_VERSION, LocalCaMetadata, PRIVILEGED_HELPER_VERSION,
    PlatformError, PlatformTarget, TrustDomainState, generate_local_ca,
    inspect_daemon_registration_file,
};
use serde_json::{Value, json};
use state::{PvPaths, fs};

struct TestEnvironment {
    home: Utf8PathBuf,
    browser_ca: Vec<(Utf8PathBuf, bool)>,
    operations: RefCell<Vec<&'static str>>,
    fail_next_enable: Cell<bool>,
    registration_status: Option<DaemonRegistrationStatus>,
    registration_conflict: Option<Utf8PathBuf>,
    registration_unloaded: bool,
    platform_target: PlatformTarget,
    helper_authentication_error: bool,
    terminal_width: Option<usize>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.to_owned(),
            browser_ca: Vec::new(),
            operations: RefCell::new(Vec::new()),
            fail_next_enable: Cell::new(false),
            registration_status: None,
            registration_conflict: None,
            registration_unloaded: false,
            platform_target: PlatformTarget::Linux,
            helper_authentication_error: false,
            terminal_width: None,
        }
    }

    fn unit_path(&self) -> Utf8PathBuf {
        self.home.join(".config/systemd/user/pv-daemon.service")
    }

    fn write_unit(&self) -> anyhow::Result<()> {
        let config = DaemonRegistrationConfig::for_paths(&PvPaths::for_home(&self.home));
        fs::write_sensitive_file(&self.unit_path(), &config.render()?)?;
        Ok(())
    }
}

impl Environment for TestEnvironment {
    fn browser_ca_databases(&self, _home: &Utf8Path) -> Result<Vec<Utf8PathBuf>, PlatformError> {
        Ok(self
            .browser_ca
            .iter()
            .map(|(path, _)| path.clone())
            .collect())
    }

    fn inspect_browser_ca_trust(
        &self,
        database: &Utf8Path,
        metadata: Option<&LocalCaMetadata>,
    ) -> TrustDomainState {
        let Some(metadata) = metadata else {
            return TrustDomainState::Unknown {
                reason: "local CA is unavailable".to_owned(),
            };
        };
        if self
            .browser_ca
            .iter()
            .any(|(path, trusted)| path == database && *trusted)
        {
            TrustDomainState::Current {
                fingerprint: metadata.fingerprint.clone(),
            }
        } else {
            TrustDomainState::NotTrusted {
                fingerprint: metadata.fingerprint.clone(),
            }
        }
    }

    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.as_std_path().to_owned())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.home.as_std_path().to_owned())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PvPaths::for_home(&self.home)
            .active_pv_binary()
            .into_std_path_buf())
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn stdout_is_terminal(&self) -> bool {
        self.terminal_width.is_some()
    }

    fn terminal_width(&self) -> Option<usize> {
        self.terminal_width
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn enable_daemon_registration(&self, _path: &Utf8Path) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("enable");
        if self.fail_next_enable.replace(false) {
            return Err(PlatformError::DaemonRegistrationState {
                operation: "enable",
                message: "fixture rejected enable".to_owned(),
            });
        }
        Ok(())
    }

    fn restart_daemon_registration(&self) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("restart");
        Err(PlatformError::DaemonRegistrationState {
            operation: "restart",
            message: "fixture stopped before health".to_owned(),
        })
    }

    fn stop_daemon_registration(&self) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("stop");
        Ok(())
    }

    fn remove_daemon_registration(&self, path: &Utf8Path) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("remove");
        fs::delete_file(path).map_err(PlatformError::DaemonRegistrationFile)
    }

    fn daemon_process_for_stop(
        &self,
        _paths: &PvPaths,
    ) -> Result<Option<daemon::DaemonProcess>, daemon::DaemonError> {
        Ok(None)
    }

    fn inspect_daemon_registration(
        &self,
    ) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
        if let Some(path) = &self.registration_conflict {
            return Err(PlatformError::DaemonRegistrationConflict { path: path.clone() });
        }
        if self.registration_unloaded {
            return Ok(None);
        }
        self.registration_status
            .clone()
            .map(Some)
            .ok_or(PlatformError::UserManagerSessionEnvironment)
    }

    fn resolver_test_path(&self) -> PathBuf {
        self.home.join("etc/resolver/test").into_std_path_buf()
    }

    fn os_release_path(&self) -> PathBuf {
        self.home.join("etc/os-release").into_std_path_buf()
    }

    fn pf_anchor_path(&self) -> PathBuf {
        self.home
            .join("etc/pf.anchors/com.prvious.pv")
            .into_std_path_buf()
    }

    fn pf_conf_path(&self) -> PathBuf {
        self.home.join("etc/pf.conf").into_std_path_buf()
    }

    fn socket_proxy_directory(&self) -> PathBuf {
        self.home.join("etc/systemd/system").into_std_path_buf()
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }

    fn inspect_active_pf_redirects_unprivileged(
        &self,
    ) -> Result<ActivePfRedirectInspection, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }

    fn privileged_helper_status(&self) -> Result<platform::PrivilegedHelperStatus, PlatformError> {
        if self.helper_authentication_error {
            return Err(PlatformError::PrivilegedHelperAuthentication(
                "fixture helper belongs to another account".to_owned(),
            ));
        }
        Err(PlatformError::PrivilegedHelperUnavailable)
    }

    fn platform_target(&self) -> Result<PlatformTarget, PlatformError> {
        Ok(self.platform_target)
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<platform::KeychainCertificate>, PlatformError> {
        Ok(Vec::new())
    }
}

#[test]
fn linux_enable_retries_persistent_registration_after_writing_the_unit() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.fail_next_enable.set(true);

    let first = run_pv(&["daemon:enable"], &environment)?;
    assert_eq!(first.exit_code, ExitCode::FAILURE);
    let unit = fs::read_to_string(&environment.unit_path())?;
    assert!(DaemonRegistrationConfig::parse(&unit).is_some());
    let retry = run_pv(&["daemon:enable"], &environment)?;
    assert_eq!(retry.exit_code, ExitCode::FAILURE);
    assert_eq!(fs::read_to_string(&environment.unit_path())?, unit);
    assert_debug_snapshot!((first, retry, environment.operations.borrow().clone()));
    Ok(())
}

#[test]
fn linux_disable_stops_then_removes_the_registration_through_the_environment() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.write_unit()?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!environment.unit_path().exists());
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!((output, environment.operations.borrow().clone()));
    });
    Ok(())
}

#[test]
fn linux_disable_uses_the_injected_registration_name() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut outputs = Vec::new();
    for target in [PlatformTarget::Macos, PlatformTarget::Linux] {
        let home = tempdir.path().join(format!("{target:?}"));
        let mut environment = TestEnvironment::new(&home);
        environment.platform_target = target;
        fs::ensure_layout(&PvPaths::for_home(&home))?;
        let output = run_pv(&["daemon:disable"], &environment)?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_eq!(*environment.operations.borrow(), ["stop"]);
        assert!(!environment.unit_path().exists());
        outputs.push((target, output));
    }
    assert_debug_snapshot!(outputs);
    Ok(())
}

#[test]
fn linux_doctor_registration_uses_the_injected_platform() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut checks = Vec::new();
    for target in [PlatformTarget::Macos, PlatformTarget::Linux] {
        for case in ["missing", "current", "stale", "conflict", "unreadable"] {
            let home = tempdir.path().join(format!("{target:?}-{case}"));
            let mut environment = TestEnvironment::new(&home);
            environment.platform_target = target;
            fs::ensure_layout(&PvPaths::for_home(&home))?;
            let path = environment.unit_path();
            match case {
                "current" => environment.write_unit()?,
                "stale" => {
                    environment.write_unit()?;
                    let content =
                        fs::read_to_string(&path)?.replace("ExecStart=", "InvalidExecStart=");
                    fs::write_sensitive_file(&path, &content)?;
                }
                "conflict" => fs::write_sensitive_file(&path, "foreign unit\n")?,
                "unreadable" => fs::ensure_user_dir(&path)?,
                _ => {}
            }
            let actual_case = match inspect_daemon_registration_file(&path, None) {
                DaemonRegistrationFileState::Missing { .. } => "missing",
                DaemonRegistrationFileState::Current { .. } => "current",
                DaemonRegistrationFileState::Stale { .. } => "stale",
                DaemonRegistrationFileState::Conflict { .. } => "conflict",
                DaemonRegistrationFileState::Unreadable { .. } => "unreadable",
            };
            assert_eq!(actual_case, case);
            let output = run_pv(&["doctor", "--json"], &environment)?;
            assert_eq!(output.exit_code, ExitCode::FAILURE);
            assert!(output.stderr.is_empty());
            assert!(environment.operations.borrow().is_empty());
            let report: Value = serde_json::from_str(&output.stdout)?;
            let check = report["sections"]
                .as_array()
                .and_then(|sections| {
                    sections
                        .iter()
                        .find(|section| section["title"] == "Daemon & jobs")
                })
                .and_then(|section| section["checks"].as_array())
                .and_then(|checks| checks.first())
                .ok_or_else(|| anyhow::anyhow!("missing daemon registration diagnostic"))?;
            checks.push((target, case, check.clone()));
        }
    }
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(checks);
    });
    Ok(())
}

#[test]
fn linux_distribution_is_informational_in_doctor_and_status() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut environment = TestEnvironment::new(&tempdir.path().join("home"));
    fs::ensure_layout(&PvPaths::for_home(&environment.home))?;
    let path = environment.home.join("etc/os-release");
    let baseline = run_pv(&["doctor", "--json"], &environment)?;
    let baseline_report: Value = serde_json::from_str(&baseline.stdout)?;
    let baseline_doctor_state = diagnostics_without_distribution(&baseline_report)?;
    let baseline_status = run_pv(&["status", "--json"], &environment)?;
    let baseline_status_report: Value = serde_json::from_str(&baseline_status.stdout)?;
    let baseline_status_state = diagnostics_without_distribution(&baseline_status_report)?;
    let mut observed = Vec::new();
    for (case, content) in [
        (
            "double quoted",
            Some(r#"PRETTY_NAME="Étoile \"LTS\" $HOME $(printf expanded) `printf expanded`""#),
        ),
        ("single quoted", Some("PRETTY_NAME='Debian \"Linux\" 13'")),
        (
            "last assignment",
            Some("PRETTY_NAME=Earlier\nPRETTY_NAME=Last"),
        ),
        (
            "malformed last",
            Some("PRETTY_NAME=Earlier\nPRETTY_NAME=\"broken"),
        ),
        ("empty last", Some("PRETTY_NAME=Earlier\nPRETTY_NAME=\"\"")),
        (
            "unset",
            Some(
                "NAME=Linux\n# PRETTY_NAME=Ignored\nOTHER_PRETTY_NAME=Ignored\nPRETTY_NAME_SUFFIX=Ignored",
            ),
        ),
        ("multiple words", Some("PRETTY_NAME=Two Words")),
        ("control character", Some("PRETTY_NAME='Unsafe\u{1b}[31m'")),
        ("missing", None),
        ("directory", None),
        ("invalid UTF-8", None),
        ("symlink", Some("PRETTY_NAME='Ubuntu 22.04.5 LTS'")),
    ] {
        if case == "directory" {
            fs::ensure_user_dir(&path)?;
        } else if case == "invalid UTF-8" {
            fs::create_new_file(&path)?.write_all(b"PRETTY_NAME=\xff\n")?;
        } else if let Some(content) = content {
            if case == "symlink" {
                let target = environment.home.join("os-release-target");
                fs::write_sensitive_file(&target, content)?;
                fs::symlink_file(&target, &path)?;
            } else {
                fs::write_sensitive_file(&path, content)?;
            }
        }
        let output = run_pv(&["doctor", "--json"], &environment)?;
        let report: Value = serde_json::from_str(&output.stdout)?;
        let status = run_pv(&["status", "--json"], &environment)?;
        let status_report: Value = serde_json::from_str(&status.stdout)?;
        assert_eq!(output.exit_code, baseline.exit_code, "{case}");
        assert_eq!(status.exit_code, baseline_status.exit_code, "{case}");
        assert!(output.stderr.is_empty(), "{case}");
        assert!(status.stderr.is_empty(), "{case}");
        assert!(environment.operations.borrow().is_empty(), "{case}");
        assert_eq!(
            diagnostics_without_distribution(&report)?,
            baseline_doctor_state,
            "{case}"
        );
        assert_eq!(
            diagnostics_without_distribution(&status_report)?,
            baseline_status_state,
            "{case}"
        );
        assert_eq!(
            status_report["linux_distribution"], report["linux_distribution"],
            "{case}"
        );
        observed.push(json!({"case": case, "distribution": report["linux_distribution"]}));
        if case == "directory" {
            fs::delete_dir_all(&path)?;
        } else if case != "missing" {
            fs::delete_file(&path)?;
        }
    }
    assert_snapshot!(serde_json::to_string_pretty(&observed)?);

    environment.platform_target = PlatformTarget::Macos;
    fs::write_sensitive_file(&path, "PRETTY_NAME=LinuxFixture")?;
    for (command, exit_code) in [
        ("doctor", baseline.exit_code),
        ("status", baseline_status.exit_code),
    ] {
        let output = run_pv(&[command, "--json"], &environment)?;
        let report: Value = serde_json::from_str(&output.stdout)?;
        assert!(report.get("linux_distribution").is_none(), "{command}");
        assert_eq!(output.exit_code, exit_code, "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert!(environment.operations.borrow().is_empty(), "{command}");
    }
    Ok(())
}

#[test]
fn linux_distribution_follows_the_doctor_and_status_headings() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut environment = TestEnvironment::new(&tempdir.path().join("home"));
    fs::ensure_layout(&PvPaths::for_home(&environment.home))?;
    fs::write_sensitive_file(
        &environment.home.join("etc/os-release"),
        "PRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\n",
    )?;
    let mut observed = Vec::new();
    for command in ["doctor", "status"] {
        let baseline = run_pv(&[command, "--json"], &environment)?;
        for (width, lines) in [(None, 2), (Some(72), 3)] {
            environment.terminal_width = width;
            let output = run_pv(&["--no-color", command], &environment)?;
            assert_eq!(output.exit_code, baseline.exit_code);
            assert!(output.stderr.is_empty());
            assert!(environment.operations.borrow().is_empty());
            observed.push(
                output
                    .stdout
                    .lines()
                    .take(lines)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    }
    assert_debug_snapshot!(observed);
    Ok(())
}

fn diagnostics_without_distribution(report: &Value) -> anyhow::Result<String> {
    let timestamp = if report.get("sections").is_some() {
        &doctor_check(report, "Port redirect config")?["routing"]["observed_at"]
    } else {
        &report["integrations"]["ports"]["observed_at"]
    }
    .as_str()
    .ok_or_else(|| anyhow::anyhow!("missing routing observation time"))?;
    let mut state = report.clone();
    state
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("diagnostic report is not an object"))?
        .remove("linux_distribution");
    Ok(serde_json::to_string(&state)?.replace(timestamp, "<timestamp>"))
}

#[test]
fn linux_restart_restores_persistent_enablement_before_starting_the_daemon() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.write_unit()?;
    environment.fail_next_enable.set(true);

    let first = run_pv(&["daemon:restart"], &environment)?;
    let retry = run_pv(&["daemon:restart"], &environment)?;
    assert_eq!(first.exit_code, ExitCode::FAILURE);
    assert_eq!(retry.exit_code, ExitCode::FAILURE);
    assert_debug_snapshot!(environment.operations.borrow().clone(), @r#"
    [
        "enable",
        "enable",
        "restart",
    ]
    "#);
    Ok(())
}

#[test]
fn linux_status_and_doctor_report_an_unavailable_manager_and_keep_other_checks()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.write_unit()?;

    let status = run_pv(&["status", "--json"], &environment)?;
    let doctor = run_pv(&["doctor", "--json"], &environment)?;
    assert_eq!(status.exit_code, ExitCode::FAILURE);
    assert_eq!(doctor.exit_code, ExitCode::FAILURE);
    assert!(status.stderr.is_empty());
    assert!(doctor.stderr.is_empty());
    let status: Value = serde_json::from_str(&status.stdout)?;
    let doctor: Value = serde_json::from_str(&doctor.stdout)?;
    let observed = json!({
        "daemon": status["daemon"],
        "manager": doctor_check(&doctor, "Systemd user manager")?,
        "sections": doctor["sections"].as_array().map(|sections| sections.iter().map(|section| section["title"].clone()).collect::<Vec<_>>()),
    });
    assert_snapshot!(serde_json::to_string_pretty(&observed)?);
    Ok(())
}

#[test]
fn linux_status_and_doctor_show_native_properties_and_reject_a_foreign_loaded_unit()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut observed = Vec::new();
    for owned in [true, false] {
        let mut environment =
            TestEnvironment::new(&tempdir.path().join(if owned { "owned" } else { "foreign" }));
        environment.write_unit()?;
        if !owned {
            fs::write_sensitive_file(
                &environment.unit_path(),
                "[Service]\nExecStart=/another/daemon\n",
            )?;
        }
        environment.registration_status = Some(DaemonRegistrationStatus {
            load_state: "loaded".to_owned(),
            unit_file_state: Some("enabled".to_owned()),
            active_state: "active".to_owned(),
            sub_state: "running".to_owned(),
            main_pid: 42,
            fragment_path: environment.unit_path(),
        });
        let status = run_pv(&["status", "--json"], &environment)?;
        let doctor = run_pv(&["doctor", "--json"], &environment)?;
        let plain_status = run_pv(&["status"], &environment)?;
        assert!(status.stderr.is_empty());
        assert!(doctor.stderr.is_empty());
        assert!(plain_status.stderr.is_empty());
        let status: Value = serde_json::from_str(&status.stdout)?;
        let doctor: Value = serde_json::from_str(&doctor.stdout)?;
        observed.push(json!({
            "daemon": status["daemon"],
            "manager": doctor_check(&doctor, "Systemd user manager")?,
            "plain_status": plain_status.stdout,
        }));
    }
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(observed);
    });
    Ok(())
}

#[test]
fn linux_doctor_rejects_an_owned_daemon_that_is_not_loaded_and_running() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.write_unit()?;
    let mut observed = Vec::new();
    for (load_state, unit_file_state, active_state, sub_state, main_pid) in [
        ("loaded", "enabled", "inactive", "dead", 0),
        ("loaded", "enabled", "failed", "failed", 0),
        ("loaded", "enabled", "deactivating", "stop-sigterm", 42),
        ("loaded", "enabled", "active", "running", 0),
        ("error", "enabled", "active", "running", 42),
        ("loaded", "disabled", "active", "running", 42),
        ("loaded", "enabled-runtime", "active", "running", 42),
    ] {
        environment.registration_status = Some(DaemonRegistrationStatus {
            load_state: load_state.to_owned(),
            unit_file_state: Some(unit_file_state.to_owned()),
            active_state: active_state.to_owned(),
            sub_state: sub_state.to_owned(),
            main_pid,
            fragment_path: environment.unit_path(),
        });
        let output = run_pv(&["doctor", "--json"], &environment)?;
        let report: Value = serde_json::from_str(&output.stdout)?;
        let check = doctor_check(&report, "Systemd user manager")?;
        assert_eq!(check["status"], "fail");
        observed.push(check.clone());
    }
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(observed);
    });
    Ok(())
}

#[test]
fn linux_doctor_rejects_a_configured_service_that_is_not_loaded() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut observed = Vec::new();
    for (state, expected_status) in [("missing", "pass"), ("current", "fail"), ("stale", "fail")] {
        let mut environment = TestEnvironment::new(&tempdir.path().join(state));
        environment.registration_unloaded = true;
        if state != "missing" {
            environment.write_unit()?;
        }
        if state == "stale" {
            let unit = fs::read_to_string(&environment.unit_path())?;
            fs::write_sensitive_file(
                &environment.unit_path(),
                &unit.replace("RestartSec=1", "RestartSec=2"),
            )?;
            assert!(matches!(
                inspect_daemon_registration_file(&environment.unit_path(), None),
                DaemonRegistrationFileState::Stale { .. }
            ));
        }
        let output = run_pv(&["doctor", "--json"], &environment)?;
        let report: Value = serde_json::from_str(&output.stdout)?;
        let check = doctor_check(&report, "Systemd user manager")?;
        assert_eq!(check["status"], expected_status);
        observed.push((state, check.clone()));
    }
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(observed);
    });
    Ok(())
}

#[test]
fn linux_doctor_reports_configuration_repair_for_a_foreign_drop_in() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.write_unit()?;
    environment.registration_conflict = Some(
        environment
            .unit_path()
            .with_extension("service.d")
            .join("override.conf"),
    );
    let output = run_pv(&["doctor", "--json"], &environment)?;
    let report: Value = serde_json::from_str(&output.stdout)?;
    let check = doctor_check(&report, "Systemd user manager")?;
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(check);
    });
    Ok(())
}

#[test]
fn linux_doctor_uses_the_injected_platform_for_cross_account_helper_recovery() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let mut environment = TestEnvironment::new(&tempdir.path().join("home"));
    environment.helper_authentication_error = true;
    let paths = PvPaths::for_home(&environment.home);
    fs::write_sensitive_file(
        &paths
            .app_release_helper(env!("CARGO_PKG_VERSION"))
            .with_file_name("pv-helper.json"),
        &serde_json::to_string(&json!({
            "version": PRIVILEGED_HELPER_VERSION,
            "protocol_version": HELPER_PROTOCOL_VERSION,
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        }))?,
    )?;
    let mut observed = Vec::new();
    for target in [PlatformTarget::Macos, PlatformTarget::Linux] {
        environment.platform_target = target;
        let output = run_pv(&["doctor", "--json"], &environment)?;
        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stderr.is_empty());
        let report: Value = serde_json::from_str(&output.stdout)?;
        let check = doctor_check(&report, "Privileged helper")?;
        assert_eq!(check["status"], "fail");
        observed.push(json!({ "target": target.as_str(), "check": check }));
    }
    assert!(environment.operations.borrow().is_empty());
    assert_snapshot!(serde_json::to_string_pretty(&observed)?);
    Ok(())
}

fn doctor_check<'a>(report: &'a Value, name: &str) -> anyhow::Result<&'a Value> {
    report["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["checks"].as_array().into_iter().flatten())
        .find(|check| check["name"] == name)
        .ok_or_else(|| anyhow::anyhow!("missing {name} diagnostic"))
}

fn snapshot_settings(tempdir: &Utf8Path) -> Settings {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;
    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

#[test]
fn linux_browser_ca_diagnostics_report_each_database() -> anyhow::Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let paths = PvPaths::for_home(&home);
    let generated = generate_local_ca()?;
    fs::write_sensitive_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    fs::write_sensitive_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let mut environment = TestEnvironment::new(&home);
    environment.browser_ca = vec![
        (home.join(".pki/nssdb"), true),
        (home.join(".mozilla/firefox/default"), false),
    ];
    let doctor = run_pv(&["doctor", "--json"], &environment)?;
    let report: Value = serde_json::from_str(&doctor.stdout)?;
    let sections = report["sections"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing doctor sections"))?;
    let checks = sections
        .iter()
        .filter_map(|section| section["checks"].as_array())
        .flatten()
        .filter(|check| check["name"] == "Browser CA trust")
        .collect::<Vec<_>>();
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0]["status"], "pass");
    assert_eq!(checks[1]["status"], "fail");
    let status = run_pv(&["status", "--json"], &environment)?;
    let status_report: Value = serde_json::from_str(&status.stdout)?;
    let browsers = status_report["integrations"]["browser_ca"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing browser CA status"))?;
    assert_eq!(browsers.len(), 2);
    assert_eq!(
        browsers[0]["database"],
        environment.browser_ca[0].0.as_str()
    );
    assert_eq!(
        browsers[1]["database"],
        environment.browser_ca[1].0.as_str()
    );
    assert_eq!(
        browsers[0]["state"]["Current"]["fingerprint"],
        generated.metadata.fingerprint
    );
    assert_eq!(
        browsers[1]["state"]["NotTrusted"]["fingerprint"],
        generated.metadata.fingerprint
    );
    snapshot_settings(directory.path()).bind(|| {
        assert_debug_snapshot!(checks);
    });
    Ok(())
}
