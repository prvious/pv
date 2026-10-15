#![cfg(target_os = "linux")]

#[path = "support/daemon_health.rs"]
mod daemon_health;

use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use daemon_health::DaemonFixture;
use insta::{Settings, assert_debug_snapshot};
use platform::{
    ActivePfRedirectInspection, DaemonRegistrationStatus, KeychainCertificate, LowPortInspection,
    PlatformError, PrivilegedHelperStatus, ResolverConfig, ResolverFileState,
    SYSTEM_RESOLVER_TEST_PATH,
};
use serde_json::{Value, json};
use state::{Database, HelperLifecycleLock, PortOwner, PvPaths, StateError, fs};

#[derive(Debug, Eq, PartialEq)]
enum Operation {
    Apply(u16),
    Remove,
}

struct TestEnvironment {
    home: Utf8PathBuf,
    resolver: RefCell<ResolverFileState>,
    apply_error: RefCell<Option<PlatformError>>,
    remove_error: RefCell<Option<PlatformError>>,
    operations: RefCell<Vec<Operation>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.to_owned(),
            resolver: RefCell::new(ResolverFileState::Missing {
                path: SYSTEM_RESOLVER_TEST_PATH.into(),
            }),
            apply_error: RefCell::new(None),
            remove_error: RefCell::new(None),
            operations: RefCell::new(Vec::new()),
        }
    }

    fn paths(&self) -> PvPaths {
        PvPaths::for_home(&self.home)
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
        Ok(self.paths().active_pv_binary().into_std_path_buf())
    }
    fn stdin_is_terminal(&self) -> bool {
        false
    }
    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn inspect_resolver_file(
        &self,
        path: &Utf8Path,
        _expected: Option<&ResolverConfig>,
    ) -> ResolverFileState {
        assert_eq!(path, Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH));
        self.resolver.borrow().clone()
    }

    fn install_resolver_config(
        &self,
        prepared_path: &Utf8Path,
        system_path: &Utf8Path,
    ) -> Result<(), PlatformError> {
        assert!(matches!(
            HelperLifecycleLock::acquire(&self.paths()),
            Err(StateError::HelperLifecycleLockHeld { .. })
        ));
        assert_eq!(system_path, Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH));
        let content = fs::read_to_string(prepared_path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        let config = ResolverConfig::parse(&content).ok_or_else(|| {
            PlatformError::SystemIntegration("fixture received invalid prepared config".to_owned())
        })?;
        self.operations
            .borrow_mut()
            .push(Operation::Apply(config.port));
        if let Some(error) = self.apply_error.borrow_mut().take() {
            return Err(error);
        }
        *self.resolver.borrow_mut() = ResolverFileState::Current {
            path: system_path.into(),
            port: config.port,
        };
        Ok(())
    }

    fn remove_resolver_config(&self, system_path: &Utf8Path) -> Result<(), PlatformError> {
        assert!(matches!(
            HelperLifecycleLock::acquire(&self.paths()),
            Err(StateError::HelperLifecycleLockHeld { .. })
        ));
        assert_eq!(system_path, Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH));
        self.operations.borrow_mut().push(Operation::Remove);
        if let Some(error) = self.remove_error.borrow_mut().take() {
            return Err(error);
        }
        *self.resolver.borrow_mut() = ResolverFileState::Missing {
            path: system_path.into(),
        };
        Ok(())
    }

    fn pf_anchor_path(&self) -> PathBuf {
        self.home.join("etc/pf.anchor").into_std_path_buf()
    }
    fn pf_conf_path(&self) -> PathBuf {
        self.home.join("etc/pf.conf").into_std_path_buf()
    }
    fn inspect_low_ports(&self) -> Result<LowPortInspection, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }
    fn inspect_active_pf_redirects_unprivileged(
        &self,
    ) -> Result<ActivePfRedirectInspection, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }
    fn privileged_helper_status(&self) -> Result<PrivilegedHelperStatus, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }
    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, PlatformError> {
        Ok(Vec::new())
    }
    fn inspect_daemon_registration(
        &self,
    ) -> Result<Option<DaemonRegistrationStatus>, PlatformError> {
        Ok(None)
    }
}

#[test]
fn linux_dns_commands_refuse_helper_lifecycle_contention_before_mutation() -> Result<()> {
    let directory = tempdir()?;
    let environment = TestEnvironment::new(&directory.path().join("home"));
    let paths = environment.paths();
    let port = pv_fake::seed_dns_port(&paths)?;
    let prepared = format!(
        "{}# retained fixture preparation\n",
        ResolverConfig::new(port).render()
    );
    fs::write_sensitive_file(&paths.resolver_config(), &prepared)?;
    let guard = HelperLifecycleLock::acquire(&paths)?;
    let install = run_pv(&["dns:install"], &environment)?;
    let uninstall = run_pv(&["dns:uninstall"], &environment)?;
    assert_eq!(install.exit_code, ExitCode::FAILURE);
    assert_eq!(uninstall.exit_code, ExitCode::FAILURE);
    assert_eq!(dns_port(&paths)?, Some(port));
    assert_eq!(fs::read_to_string(&paths.resolver_config())?, prepared);
    assert!(environment.operations.borrow().is_empty());
    snapshot_settings(directory.path(), &[port])
        .bind(|| assert_debug_snapshot!((install, uninstall)));
    drop(guard);
    let _released = HelperLifecycleLock::acquire(&paths)?;
    Ok(())
}

#[test]
fn linux_dns_install_keeps_the_port_after_unknown_or_partial_apply_and_retries_it() -> Result<()> {
    let directory = tempdir()?;
    for (case, error) in [
        ("unknown", unknown_outcome("apply")),
        (
            "partial",
            PlatformError::SystemIntegration(
                "fixture native apply failed after writing the unit".to_owned(),
            ),
        ),
    ] {
        let environment = TestEnvironment::new(&directory.path().join(case));
        *environment.apply_error.borrow_mut() = Some(error);
        let paths = environment.paths();
        assert!(dns_port(&paths)?.is_none());
        let daemon = DaemonFixture::start(&paths)?;
        let first = run_pv(&["dns:install"], &environment)?;
        assert_eq!(daemon.finish()?.len(), 1);
        assert_eq!(first.exit_code, ExitCode::FAILURE);
        let port =
            dns_port(&paths)?.ok_or_else(|| anyhow!("uncertain DNS reservation was released"))?;
        assert_eq!(
            ResolverConfig::parse(&fs::read_to_string(&paths.resolver_config())?),
            Some(ResolverConfig::new(port))
        );
        assert!(matches!(
            *environment.resolver.borrow(),
            ResolverFileState::Missing { .. }
        ));
        let daemon = DaemonFixture::start(&paths)?;
        let retry = run_pv(&["dns:install"], &environment)?;
        assert_eq!(daemon.finish()?.len(), 1);
        assert_eq!(retry.exit_code, ExitCode::SUCCESS);
        assert_eq!(dns_port(&paths)?, Some(port));
        assert_eq!(
            *environment.operations.borrow(),
            vec![Operation::Apply(port), Operation::Apply(port)]
        );
        let mut settings = snapshot_settings(directory.path(), &[port]);
        settings.set_snapshot_suffix(case);
        settings.bind(|| assert_debug_snapshot!((first, retry)));
    }
    Ok(())
}

#[test]
fn linux_dns_install_releases_a_new_port_after_an_unavailable_or_rejecting_helper() -> Result<()> {
    let directory = tempdir()?;
    for error in [
        PlatformError::PrivilegedHelperUnavailable,
        PlatformError::PrivilegedHelperRejected {
            message: "fixture rejected before mutation".to_owned(),
        },
    ] {
        let environment = TestEnvironment::new(&directory.path().join("home"));
        *environment.apply_error.borrow_mut() = Some(error);
        let paths = environment.paths();
        let daemon = DaemonFixture::start(&paths)?;
        let output = run_pv(&["dns:install"], &environment)?;
        daemon.finish()?;
        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(dns_port(&paths)?, None);
        assert_eq!(environment.operations.borrow().len(), 1);
    }
    Ok(())
}

#[test]
fn linux_dns_uninstall_waits_for_removal_even_when_inspection_is_missing() -> Result<()> {
    let directory = tempdir()?;
    let environment = TestEnvironment::new(&directory.path().join("home"));
    let paths = environment.paths();
    let port = pv_fake::seed_dns_port(&paths)?;
    fs::write_sensitive_file(
        &paths.resolver_config(),
        &ResolverConfig::new(port).render(),
    )?;
    *environment.remove_error.borrow_mut() = Some(unknown_outcome("remove"));
    let first = run_pv(&["dns:uninstall"], &environment)?;
    assert_eq!(first.exit_code, ExitCode::FAILURE);
    assert_eq!(dns_port(&paths)?, Some(port));
    let retry = run_pv(&["dns:uninstall"], &environment)?;
    assert_eq!(retry.exit_code, ExitCode::SUCCESS);
    assert_eq!(dns_port(&paths)?, None);
    assert_eq!(
        *environment.operations.borrow(),
        vec![Operation::Remove, Operation::Remove]
    );
    assert!(!paths.resolver_config().exists());
    snapshot_settings(directory.path(), &[port]).bind(|| assert_debug_snapshot!((first, retry)));
    Ok(())
}

#[test]
fn linux_dns_status_and_doctor_consume_native_states_without_mutation() -> Result<()> {
    let directory = tempdir()?;
    let environment = TestEnvironment::new(&directory.path().join("home"));
    let paths = environment.paths();
    let port = pv_fake::seed_dns_port(&paths)?;
    let other_port = pv_fake::port()?;
    let prepared = ResolverConfig::new(port).render();
    fs::write_sensitive_file(&paths.resolver_config(), &prepared)?;
    let path = Utf8PathBuf::from(SYSTEM_RESOLVER_TEST_PATH);
    let mut observed = Vec::new();
    for (case, state) in [
        (
            "current",
            ResolverFileState::Current {
                path: path.clone(),
                port,
            },
        ),
        ("missing", ResolverFileState::Missing { path: path.clone() }),
        (
            "stale",
            ResolverFileState::Stale {
                path: path.clone(),
                expected_port: Some(port),
                actual_port: Some(other_port),
            },
        ),
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
        assert_eq!(fs::read_to_string(&paths.resolver_config())?, prepared);
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
    snapshot_settings(directory.path(), &[port, other_port])
        .bind(|| assert_debug_snapshot!(observed));
    Ok(())
}

#[test]
fn linux_dns_conflicts_require_manual_repair_and_do_not_mutate_system_state() -> Result<()> {
    let directory = tempdir()?;
    let environment = TestEnvironment::new(&directory.path().join("home"));
    let paths = environment.paths();
    let port = pv_fake::seed_dns_port(&paths)?;
    *environment.resolver.borrow_mut() = ResolverFileState::Conflict {
        path: SYSTEM_RESOLVER_TEST_PATH.into(),
    };
    let install = run_pv(&["dns:install"], &environment)?;
    *environment.remove_error.borrow_mut() = Some(PlatformError::PrivilegedHelperRejected {
        message: "fixture refused foreign DNS ownership".to_owned(),
    });
    let uninstall = run_pv(&["dns:uninstall"], &environment)?;
    assert_eq!(install.exit_code, ExitCode::FAILURE);
    assert_eq!(uninstall.exit_code, ExitCode::FAILURE);
    assert_eq!(dns_port(&paths)?, Some(port));
    assert_eq!(*environment.operations.borrow(), [Operation::Remove]);
    snapshot_settings(directory.path(), &[port])
        .bind(|| assert_debug_snapshot!((install, uninstall)));
    Ok(())
}

#[test]
fn linux_dns_uninstall_delegates_uncertain_inspection_and_releases_only_after_success() -> Result<()>
{
    let directory = tempdir()?;
    for unreadable in [false, true] {
        for accepted in [false, true] {
            let case = format!(
                "{}-{}",
                if unreadable { "unreadable" } else { "conflict" },
                if accepted { "accepted" } else { "rejected" }
            );
            let environment = TestEnvironment::new(&directory.path().join(&case));
            let paths = environment.paths();
            let port = pv_fake::seed_dns_port(&paths)?;
            *environment.resolver.borrow_mut() = if unreadable {
                ResolverFileState::Unreadable {
                    path: SYSTEM_RESOLVER_TEST_PATH.into(),
                    message: "fixture resolved unavailable".to_owned(),
                }
            } else {
                ResolverFileState::Conflict {
                    path: SYSTEM_RESOLVER_TEST_PATH.into(),
                }
            };
            if !accepted {
                *environment.remove_error.borrow_mut() =
                    Some(PlatformError::PrivilegedHelperRejected {
                        message: "fixture refused unsafe DNS removal".to_owned(),
                    });
            }
            let output = run_pv(&["dns:uninstall"], &environment)?;
            assert_eq!(
                output.exit_code,
                if accepted {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            );
            assert_eq!(dns_port(&paths)?, (!accepted).then_some(port));
            assert_eq!(*environment.operations.borrow(), [Operation::Remove]);
            let mut settings = snapshot_settings(directory.path(), &[port]);
            settings.set_snapshot_suffix(case);
            settings.bind(|| assert_debug_snapshot!(output));
        }
    }
    Ok(())
}

fn unknown_outcome(operation: &'static str) -> PlatformError {
    PlatformError::ResolverMutation {
        operation,
        source: Box::new(PlatformError::PrivilegedHelperIo(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "fixture response lost after request",
        ))),
    }
}

fn dns_port(paths: &PvPaths) -> Result<Option<u16>> {
    Ok(Database::open(paths)?
        .assigned_ports()?
        .into_iter()
        .find(|assignment| assignment.owner == PortOwner::Dns)
        .map(|assignment| assignment.port))
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(arguments: &[&str], environment: &impl Environment) -> Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_with_environment(
        ["pv"].into_iter().chain(arguments.iter().copied()),
        environment,
        &mut stdout,
        &mut stderr,
    )?;
    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
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
