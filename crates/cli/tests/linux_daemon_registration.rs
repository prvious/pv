#![cfg(target_os = "linux")]

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot, assert_snapshot};
use platform::{DaemonRegistrationConfig, DaemonRegistrationStatus, PlatformError};
use serde_json::{Value, json};
use state::{PvPaths, fs};

struct TestEnvironment {
    home: Utf8PathBuf,
    operations: RefCell<Vec<&'static str>>,
    fail_next_enable: Cell<bool>,
    registration_status: Option<DaemonRegistrationStatus>,
    registration_conflict: Option<Utf8PathBuf>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.to_owned(),
            operations: RefCell::new(Vec::new()),
            fail_next_enable: Cell::new(false),
            registration_status: None,
            registration_conflict: None,
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
        self.registration_status
            .clone()
            .map(Some)
            .ok_or(PlatformError::UserManagerSessionEnvironment)
    }

    fn resolver_test_path(&self) -> PathBuf {
        self.home.join("etc/resolver/test").into_std_path_buf()
    }

    fn pf_anchor_path(&self) -> PathBuf {
        self.home
            .join("etc/pf.anchors/com.prvious.pv")
            .into_std_path_buf()
    }

    fn pf_conf_path(&self) -> PathBuf {
        self.home.join("etc/pf.conf").into_std_path_buf()
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }

    fn privileged_helper_status(&self) -> Result<platform::PrivilegedHelperStatus, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
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
        "manager": manager_check(&doctor)?,
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
            "manager": manager_check(&doctor)?,
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
    for (load_state, active_state, sub_state, main_pid) in [
        ("loaded", "inactive", "dead", 0),
        ("loaded", "failed", "failed", 0),
        ("loaded", "deactivating", "stop-sigterm", 42),
        ("loaded", "active", "running", 0),
        ("error", "active", "running", 42),
    ] {
        environment.registration_status = Some(DaemonRegistrationStatus {
            load_state: load_state.to_owned(),
            active_state: active_state.to_owned(),
            sub_state: sub_state.to_owned(),
            main_pid,
            fragment_path: environment.unit_path(),
        });
        let output = run_pv(&["doctor", "--json"], &environment)?;
        let report: Value = serde_json::from_str(&output.stdout)?;
        let check = manager_check(&report)?;
        assert_eq!(check["status"], "fail");
        observed.push(check.clone());
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
    let check = manager_check(&report)?;
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(check);
    });
    Ok(())
}

fn manager_check(report: &Value) -> anyhow::Result<&Value> {
    report["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["checks"].as_array().into_iter().flatten())
        .find(|check| check["name"] == "Systemd user manager")
        .ok_or_else(|| anyhow::anyhow!("missing systemd manager diagnostic"))
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
