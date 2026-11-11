#![cfg(target_os = "linux")]

use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::Environment;
use insta::{Settings, assert_debug_snapshot};
use platform::{
    KeychainCertificate, LowPortInspection, PlatformError, PlatformTarget, PrivilegedHelperStatus,
    ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH,
};
use serde_json::{Value, json};
use state::{Database, PortOwner, PvPaths};

use crate::support::run_pv;

#[derive(Debug, Eq, PartialEq)]
enum Operation {
    Apply(u16),
    Remove,
}

struct TestEnvironment {
    home: Utf8PathBuf,
    target: PlatformTarget,
    resolver: RefCell<ResolverFileState>,
    operations: RefCell<Vec<Operation>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.to_owned(),
            target: PlatformTarget::Linux,
            resolver: RefCell::new(ResolverFileState::Missing {
                path: SYSTEM_RESOLVER_TEST_PATH.into(),
            }),
            operations: RefCell::new(Vec::new()),
        }
    }

    fn paths(&self) -> PvPaths {
        PvPaths::for_home(&self.home)
    }
}

impl Environment for TestEnvironment {
    fn platform_target(&self) -> Result<PlatformTarget, PlatformError> {
        Ok(self.target)
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
        Ok(self.paths().active_pv_binary().into_std_path_buf())
    }
    fn stdin_is_terminal(&self) -> bool {
        false
    }
    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn inspect_system_resolver(&self, _expected: Option<&ResolverConfig>) -> ResolverFileState {
        self.resolver.borrow().clone()
    }

    fn apply_system_resolver(&self, port: u16) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push(Operation::Apply(port));
        *self.resolver.borrow_mut() = ResolverFileState::Current {
            path: SYSTEM_RESOLVER_TEST_PATH.into(),
            port,
        };
        Ok(())
    }

    fn remove_system_resolver(&self) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push(Operation::Remove);
        *self.resolver.borrow_mut() = ResolverFileState::Missing {
            path: SYSTEM_RESOLVER_TEST_PATH.into(),
        };
        Ok(())
    }

    fn inspect_low_ports(&self) -> Result<LowPortInspection, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }
    fn privileged_helper_status(&self) -> Result<PrivilegedHelperStatus, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }
    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, PlatformError> {
        Ok(Vec::new())
    }
}

#[test]
fn linux_dns_status_and_doctor_report_conflicts_and_unreadable_state_without_mutation() -> Result<()>
{
    let directory = tempdir()?;
    let environment = TestEnvironment::new(&directory.path().join("home"));
    let paths = environment.paths();
    let port = pv_fake::seed_dns_port(&paths)?;
    let path = Utf8PathBuf::from(SYSTEM_RESOLVER_TEST_PATH);
    let mut observed = Vec::new();
    for (case, state) in [
        (
            "conflict",
            ResolverFileState::Conflict { path: path.clone() },
        ),
        (
            "unreadable",
            ResolverFileState::Unreadable {
                path,
                message: "fixture resolved inspection failed".to_owned(),
            },
        ),
    ] {
        *environment.resolver.borrow_mut() = state;
        let dns = run_pv(&["dns:status"], &environment)?;
        let status = run_pv(&["status", "--json"], &environment)?;
        let doctor = run_pv(&["doctor", "--json"], &environment)?;
        assert!(dns.stderr.is_empty() && status.stderr.is_empty() && doctor.stderr.is_empty());
        assert_eq!(dns_port(&paths)?, Some(port));
        let status: Value = serde_json::from_str(&status.stdout)?;
        let doctor: Value = serde_json::from_str(&doctor.stdout)?;
        let check = doctor["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|section| section["checks"].as_array().into_iter().flatten())
            .find(|check| check["name"] == "DNS config")
            .ok_or_else(|| anyhow!("DNS doctor check missing"))?;
        observed.push(json!({ "case": case, "dns": dns.stdout, "status": status["integrations"]["dns"], "doctor": check }));
    }
    assert!(environment.operations.borrow().is_empty());
    snapshot_settings(directory.path(), &[port]).bind(|| assert_debug_snapshot!(observed));
    Ok(())
}

fn dns_port(paths: &PvPaths) -> Result<Option<u16>> {
    Ok(Database::open(paths)?
        .assigned_ports()?
        .into_iter()
        .find(|assignment| assignment.owner == PortOwner::Dns)
        .map(|assignment| assignment.port))
}

fn snapshot_settings(directory: &Utf8Path, ports: &[u16]) -> Settings {
    let mut settings = Settings::clone_current();
    settings.add_filter(directory.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    for (index, port) in ports.iter().enumerate() {
        let replacement = if index == 0 {
            "<dns-port>"
        } else {
            "<other-dns-port>"
        };
        settings.add_filter(&format!(r"\b{port}\b"), replacement);
    }
    settings
}
