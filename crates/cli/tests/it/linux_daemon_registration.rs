#![cfg(target_os = "linux")]

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::Environment;
use insta::{Settings, assert_debug_snapshot};
use platform::{ActivePfRedirectInspection, DaemonRegistrationConfig, PlatformError};
use serde_json::Value;
use state::{PvPaths, fs};

use crate::support::run_pv;

struct TestEnvironment {
    home: Utf8PathBuf,
    operations: RefCell<Vec<&'static str>>,
    fail_next_enable: Cell<bool>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.to_owned(),
            operations: RefCell::new(Vec::new()),
            fail_next_enable: Cell::new(false),
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
            return Err(PlatformError::SystemIntegration(
                "fixture rejected enable".to_owned(),
            ));
        }
        Ok(())
    }

    fn restart_daemon_registration(&self) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("restart");
        Err(PlatformError::SystemIntegration(
            "fixture stopped before health".to_owned(),
        ))
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
        Err(PlatformError::PrivilegedHelperUnavailable)
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<platform::KeychainCertificate>, PlatformError> {
        Ok(Vec::new())
    }
}

#[test]
fn linux_enable_and_restart_retry_persistent_enablement_after_it_fails() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut observed = Vec::new();
    for command in ["daemon:enable", "daemon:restart"] {
        let environment = TestEnvironment::new(&tempdir.path().join(command));
        if command == "daemon:restart" {
            environment.write_unit()?;
        }
        environment.fail_next_enable.set(true);

        let first = run_pv(&[command], &environment)?;
        let unit = fs::read_to_string(&environment.unit_path())?;
        let retry = run_pv(&[command], &environment)?;

        assert!(DaemonRegistrationConfig::parse(&unit).is_some());
        assert_eq!(fs::read_to_string(&environment.unit_path())?, unit);
        observed.push((
            command,
            first.exit_code,
            retry.exit_code,
            environment.operations.borrow().clone(),
        ));
    }
    assert_debug_snapshot!(observed);
    Ok(())
}

#[test]
fn linux_doctor_reports_a_unit_that_is_not_enabled() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let mut observed = Vec::new();
    for (case, enabled) in [("enabled", true), ("not enabled", false)] {
        let environment = TestEnvironment::new(&tempdir.path().join(case));
        environment.write_unit()?;
        if enabled {
            let unit = environment.unit_path();
            let wants = unit.with_file_name("default.target.wants");
            fs::ensure_user_dir(&wants)?;
            fs::symlink_file(&unit, &wants.join("pv-daemon.service"))?;
        }
        let output = run_pv(&["doctor", "--json"], &environment)?;
        let report: Value = serde_json::from_str(&output.stdout)?;
        observed.push((case, registration_check(&report)?.clone()));
    }
    snapshot_settings(tempdir.path()).bind(|| {
        assert_debug_snapshot!(observed);
    });
    Ok(())
}

fn registration_check(report: &Value) -> anyhow::Result<&Value> {
    report["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["checks"].as_array().into_iter().flatten())
        .find(|check| check["name"] == "Daemon service")
        .ok_or_else(|| anyhow::anyhow!("missing daemon service diagnostic"))
}

fn snapshot_settings(tempdir: &Utf8Path) -> Settings {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings
}
