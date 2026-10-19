use std::io;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{
    PfFileState, PfRedirectConfig, PlatformError, inspect_socket_proxy_unit,
    render_socket_proxy_units, socket_proxy_unit_names,
};
use state::{Database, GatewayPort, PortOwner, PvPaths, fs};

use crate::commands::write_left_in_place;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Output, Streams};

use super::{
    delete_optional_file, pf_config_from_assignments, pv_paths,
    refresh_gateway_observation_after_pf_repair, release_new_gateway_ports, write_delete_result,
};

pub(super) fn install(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    if !check_unit_files(&system_directory(environment)?, &mut streams.out)? {
        return Ok(ExitCode::FAILURE);
    }
    let inspection = environment.inspect_low_ports()?;
    if inspection.ports.iter().any(|port| !port.available) {
        let repairable = environment.repairable_socket_proxy_ports()?;
        let conflicts = inspection
            .ports
            .iter()
            .filter(|port| !port.available && !repairable.contains(&port.port))
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            streams.out.failure("Socket proxy preparation failed")?;
            for port in conflicts {
                streams.out.detail(port.conflict_message())?;
                streams.out.hint(
                    "find it",
                    &format!("sudo ss -ltnp 'sport = :{}'", port.port),
                )?;
            }
            streams
                .out
                .hint("find unit", "systemctl list-sockets --all")?;
            streams
                .out
                .hint("stop unit", "sudo systemctl disable --now <unit>")?;
            streams.out.detail(
                "Stop the conflicting service or socket, then run `pv ports:install` again.",
            )?;
            return Ok(ExitCode::FAILURE);
        }
    }

    let mut database = Database::open(&paths)?;
    let existing_assignments = database.assigned_ports()?;
    let had_http_assignment = existing_assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(GatewayPort::Http));
    let had_https_assignment = existing_assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(GatewayPort::Https));
    let assignments =
        database.assign_gateway_ports(|port| environment.loopback_tcp_port_available(port))?;
    let config = pf_config_from_assignments(&assignments);
    if let Err(error) = prepare_units(&paths, &config, &mut streams.out) {
        return Err(release_after_failure(
            &mut database,
            had_http_assignment,
            had_https_assignment,
            error,
        ));
    }
    apply(
        environment,
        &config,
        &mut database,
        had_http_assignment,
        had_https_assignment,
    )?;
    refresh_gateway_observation_after_pf_repair(environment, &paths, &config, &mut database)?;
    streams
        .out
        .success("Installed system socket proxy config")?;
    Ok(ExitCode::SUCCESS)
}

fn prepare_units(
    paths: &PvPaths,
    config: &PfRedirectConfig,
    output: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    let units = render_socket_proxy_units(config);
    for (name, content) in &units {
        fs::write_sensitive_file(&paths.ports_config().join(name), content)?;
    }
    output.success("Prepared PV socket proxy config")?;
    for (name, _) in units {
        output.detail(Line::field("unit path: ", paths.ports_config().join(name)))?;
    }
    output.detail(Line::field(
        "HTTP proxy: 127.0.0.1:80 -> ",
        format!("127.0.0.1:{}", config.http_port),
    ))?;
    output.detail(Line::field(
        "HTTPS proxy: 127.0.0.1:443 -> ",
        format!("127.0.0.1:{}", config.https_port),
    ))?;
    Ok(())
}

fn apply(
    environment: &impl Environment,
    config: &PfRedirectConfig,
    database: &mut Database,
    had_http_assignment: bool,
    had_https_assignment: bool,
) -> Result<(), ExecuteError> {
    environment
        .install_socket_proxy_units(config)
        .map_err(|error| {
            if matches!(
                &error,
                PlatformError::PortProxyMutation { .. } | PlatformError::SystemIntegration(_)
            ) {
                error.into()
            } else {
                release_after_failure(
                    database,
                    had_http_assignment,
                    had_https_assignment,
                    error.into(),
                )
            }
        })
}

fn release_after_failure(
    database: &mut Database,
    had_http_assignment: bool,
    had_https_assignment: bool,
    original: ExecuteError,
) -> ExecuteError {
    match release_new_gateway_ports(database, had_http_assignment, had_https_assignment) {
        Ok(()) => original,
        Err(cleanup) => ExecuteError::PortPreparationCleanup {
            original: Box::new(original),
            cleanup: Box::new(cleanup),
        },
    }
}

fn system_directory(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.socket_proxy_directory())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn check_unit_files(directory: &Utf8Path, output: &mut Output<'_>) -> io::Result<bool> {
    for name in socket_proxy_unit_names() {
        match inspect_socket_proxy_unit(&directory.join(name), None) {
            PfFileState::Conflict { path } => {
                write_left_in_place(
                    output,
                    "System socket proxy unit is not PV-owned: ",
                    &path,
                    None,
                )?;
                return Ok(false);
            }
            PfFileState::Unreadable { path, message } => {
                write_left_in_place(
                    output,
                    "System socket proxy unit could not be inspected: ",
                    &path,
                    Some(&message),
                )?;
                return Ok(false);
            }
            _ => {}
        }
    }
    Ok(true)
}

pub(super) fn uninstall(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let directory = system_directory(environment)?;
    if !check_unit_files(&directory, &mut streams.out)? {
        return Ok(ExitCode::FAILURE);
    }

    let files_missing = socket_proxy_unit_names().into_iter().all(|name| {
        matches!(
            inspect_socket_proxy_unit(&directory.join(name), None),
            PfFileState::Missing { .. }
        )
    });
    if !files_missing || !environment.socket_proxy_units_absent()? {
        environment.remove_socket_proxy_units()?;
    }
    for name in socket_proxy_unit_names() {
        let prepared = paths.ports_config().join(name);
        let deleted = delete_optional_file(&prepared)?;
        write_delete_result(
            &mut streams.out,
            "prepared socket proxy unit",
            &prepared,
            deleted,
        )?;
    }
    streams
        .out
        .success("Removed PV-owned system socket proxy units")?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::io::{self, Write};
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::slice;

    use anyhow::{Context, Result};
    use camino::{Utf8Path, Utf8PathBuf};
    use camino_tempfile::tempdir;
    use insta::{Settings, assert_debug_snapshot, assert_snapshot};
    use platform::{
        ActivePfRedirectInspection, LowPortInspection, LowPortState, PfRedirectConfig,
        PlatformError, PlatformTarget, PortOwner as ListenerOwner, render_socket_proxy_units,
        socket_proxy_unit_names,
    };
    use serde_json::Value;
    use state::{
        Database, GatewayPort, HelperLifecycleLock, PortOwner, PortRequest, PvPaths,
        RuntimeObservedStatus, RuntimeSubject, StateError, fs,
    };

    use crate::args::PortsStatusArgs;
    use crate::environment::Environment;
    use crate::error::ExecuteError;
    use crate::output::{Presentation, Streams};
    use crate::run_with_environment;

    struct TestEnvironment {
        home: Utf8PathBuf,
        system: Utf8PathBuf,
        error: RefCell<Option<PlatformError>>,
        apply_error: RefCell<Option<PlatformError>>,
        applications: RefCell<Vec<PfRedirectConfig>>,
        low_ports: RefCell<LowPortInspection>,
        repairable_ports: BTreeSet<u16>,
        unit_files_current: Cell<bool>,
        gateway_probe_succeeds: Cell<bool>,
        gateway_probes: Cell<u32>,
        active_inspection: RefCell<Option<ActivePfRedirectInspection>>,
        reconciliation_requests: Cell<u32>,
        removals: Cell<u32>,
        native_absence: RefCell<Option<Result<bool, PlatformError>>>,
    }

    impl TestEnvironment {
        fn new(directory: &Utf8Path) -> Self {
            Self {
                home: directory.join("home"),
                system: directory.join("etc/systemd/system"),
                error: RefCell::new(None),
                apply_error: RefCell::new(None),
                applications: RefCell::new(Vec::new()),
                low_ports: RefCell::new(LowPortInspection {
                    ports: [80, 443]
                        .into_iter()
                        .map(|port| LowPortState {
                            port,
                            available: true,
                            owners: Vec::new(),
                        })
                        .collect(),
                }),
                repairable_ports: BTreeSet::new(),
                unit_files_current: Cell::new(false),
                gateway_probe_succeeds: Cell::new(false),
                gateway_probes: Cell::new(0),
                active_inspection: RefCell::new(None),
                reconciliation_requests: Cell::new(0),
                removals: Cell::new(0),
                native_absence: RefCell::new(None),
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
            Some(self.home.clone().into_std_path_buf())
        }
        fn current_dir(&self) -> io::Result<PathBuf> {
            Ok(self.home.clone().into_std_path_buf())
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
        fn platform_target(&self) -> Result<PlatformTarget, PlatformError> {
            Ok(PlatformTarget::Linux)
        }
        fn socket_proxy_directory(&self) -> PathBuf {
            self.system.clone().into_std_path_buf()
        }
        fn socket_proxy_files_current(
            &self,
            _config: &PfRedirectConfig,
            _prepared_directory: &Utf8Path,
            _installed_directory: &Utf8Path,
        ) -> bool {
            self.unit_files_current.get()
        }
        fn inspect_low_ports(&self) -> Result<LowPortInspection, PlatformError> {
            Ok(self.low_ports.borrow().clone())
        }
        fn repairable_socket_proxy_ports(&self) -> Result<BTreeSet<u16>, PlatformError> {
            Ok(self.repairable_ports.clone())
        }
        fn loopback_tcp_port_available(&self, _port: u16) -> bool {
            false
        }
        fn install_socket_proxy_units(
            &self,
            config: &PfRedirectConfig,
        ) -> Result<(), PlatformError> {
            self.applications.borrow_mut().push(config.clone());
            self.apply_error.borrow_mut().take().map_or(Ok(()), Err)
        }
        fn probe_gateway_redirects(
            &self,
            _expected: &PfRedirectConfig,
            _ca: &Utf8Path,
        ) -> Result<(), String> {
            self.gateway_probes.set(self.gateway_probes.get() + 1);
            if self.gateway_probe_succeeds.get() {
                Ok(())
            } else {
                Err("fixture Gateway is stopped".to_owned())
            }
        }
        fn inspect_active_pf_redirects_unprivileged(
            &self,
        ) -> Result<ActivePfRedirectInspection, PlatformError> {
            self.active_inspection.borrow().clone().ok_or_else(|| {
                PlatformError::SystemIntegration("fixture system bus unavailable".to_owned())
            })
        }
        fn request_system_reconciliation(&self, _paths: &PvPaths) -> Result<(), String> {
            self.reconciliation_requests
                .set(self.reconciliation_requests.get() + 1);
            Ok(())
        }
        fn remove_socket_proxy_units(&self) -> Result<(), PlatformError> {
            self.removals.set(self.removals.get() + 1);
            self.error.borrow_mut().take().map_or(Ok(()), Err)
        }

        fn socket_proxy_units_absent(&self) -> Result<bool, PlatformError> {
            self.native_absence.borrow_mut().take().unwrap_or(Ok(false))
        }
    }

    fn seed_gateway_ports(environment: &TestEnvironment) -> Result<PfRedirectConfig> {
        let paths = environment.paths();
        let mut database = Database::open(&paths)?;
        let http = pv_fake::port()?;
        let https = pv_fake::port()?;
        for (kind, port) in [(GatewayPort::Http, http), (GatewayPort::Https, https)] {
            database.assign_port(PortRequest::gateway(kind, port, port, port), |_| true)?;
        }
        Ok(PfRedirectConfig::new(http, https))
    }

    fn prepare(environment: &TestEnvironment) -> Result<[(String, String); 4]> {
        let config = seed_gateway_ports(environment)?;
        let paths = environment.paths();
        let units = render_socket_proxy_units(&config);
        for (name, content) in &units {
            fs::write_sensitive_file(&paths.ports_config().join(name), content)?;
        }
        Ok(units)
    }

    fn run_uninstall(
        environment: &TestEnvironment,
    ) -> Result<(Result<ExitCode, ExecuteError>, String)> {
        run_mutation(environment, false)
    }

    fn run_mutation(
        environment: &TestEnvironment,
        install: bool,
    ) -> Result<(Result<ExitCode, ExecuteError>, String)> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let outcome = {
            let mut streams = Streams::new(&mut stdout, &mut stderr, Presentation::plain());
            if install {
                super::super::install(environment, &mut streams)
            } else {
                super::super::uninstall(environment, &mut streams)
            }
        };
        assert!(stderr.is_empty());
        Ok((outcome, String::from_utf8(stdout)?))
    }

    fn assert_prepared(paths: &PvPaths, config: &PfRedirectConfig) -> Result<()> {
        for (name, content) in render_socket_proxy_units(config) {
            assert_eq!(
                fs::read_to_string(&paths.ports_config().join(name))?,
                content
            );
        }
        Ok(())
    }

    fn status_json(
        environment: &TestEnvironment,
        config: &PfRedirectConfig,
    ) -> Result<(ExitCode, Value)> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = super::super::status(
            PortsStatusArgs { json: true },
            environment,
            &mut Streams::new(&mut stdout, &mut stderr, Presentation::plain()),
        )?;
        assert!(stderr.is_empty());
        let mut value: Value = serde_json::from_slice(&stdout)?;
        assert!(value["observed_at"].is_string());
        value["observed_at"] = "<time>".into();
        for (field, port, replacement) in [
            ("expected_http_port", config.http_port, "<http>"),
            ("expected_https_port", config.https_port, "<https>"),
            ("active_http_port", config.http_port, "<http>"),
            ("active_https_port", config.https_port, "<https>"),
        ] {
            if !value[field].is_null() {
                assert_eq!(value[field], port);
                value[field] = replacement.into();
            }
        }
        assert!(environment.applications.borrow().is_empty());
        assert_eq!(environment.removals.get(), 0);
        assert_eq!(environment.reconciliation_requests.get(), 0);
        Ok((exit, value))
    }

    struct OverlappingMutations<'a> {
        environment: &'a TestEnvironment,
        preparation_complete: bool,
        attempts: Vec<(&'static str, ExitCode, String)>,
    }

    impl Write for OverlappingMutations<'_> {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if self.attempts.is_empty() {
                let paths = self.environment.paths();
                let Some(database) = Database::open_read_only(&paths).map_err(io::Error::other)?
                else {
                    return Ok(buffer.len());
                };
                self.preparation_complete =
                    database.assigned_ports().map_err(io::Error::other)?.len() == 2;
                for name in socket_proxy_unit_names() {
                    self.preparation_complete &=
                        fs::path_entry_exists(&paths.ports_config().join(name))
                            .map_err(io::Error::other)?;
                }
                if !self.preparation_complete {
                    return Ok(buffer.len());
                }
                assert!(matches!(
                    HelperLifecycleLock::acquire(&paths),
                    Err(StateError::HelperLifecycleLockHeld { .. })
                ));
                for command in ["ports:uninstall", "ports:install"] {
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    let outcome = run_with_environment(
                        ["pv", command],
                        self.environment,
                        &mut stdout,
                        &mut stderr,
                    )
                    .map_err(io::Error::other)?;
                    self.attempts.push((
                        command,
                        outcome,
                        String::from_utf8(stderr).map_err(io::Error::other)?,
                    ));
                }
            }
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn overlapping_port_mutations_are_rejected_until_preparation_failure_releases_cli_lock()
    -> Result<()> {
        let directory = tempdir()?;
        let first = TestEnvironment::new(directory.path());
        let second = TestEnvironment::new(directory.path());
        let config = seed_gateway_ports(&first)?;
        let paths = first.paths();
        let assignments = Database::open_read_only(&paths)?
            .context("missing seeded database")?
            .assigned_ports()?;
        let mut probe = OverlappingMutations {
            environment: &second,
            preparation_complete: false,
            attempts: Vec::new(),
        };
        let mut stderr = Vec::new();
        let failed =
            run_with_environment(["pv", "ports:install"], &first, &mut probe, &mut stderr)?;
        assert_eq!(failed, ExitCode::FAILURE);
        assert!(
            probe.preparation_complete,
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(probe.attempts.len(), 2);
        for (command, outcome, stderr) in &probe.attempts {
            assert_eq!(*outcome, ExitCode::FAILURE, "{command}: {stderr}");
        }
        assert!(first.applications.borrow().is_empty());
        assert!(second.applications.borrow().is_empty());
        assert_eq!(second.removals.get(), 0);
        assert_eq!(Database::open(&paths)?.assigned_ports()?, assignments);
        assert_prepared(&paths, &config)?;
        let mut settings = Settings::clone_current();
        settings.add_filter(directory.path().as_str(), "<tempdir>");
        settings.bind(|| assert_debug_snapshot!(probe.attempts));

        let mut stdout = Vec::new();
        stderr.clear();
        let retry =
            run_with_environment(["pv", "ports:install"], &second, &mut stdout, &mut stderr)?;
        assert_eq!(retry, ExitCode::SUCCESS);
        assert!(stderr.is_empty());
        assert_eq!(
            second.applications.borrow().as_slice(),
            slice::from_ref(&config)
        );
        let assignments = Database::open(&paths)?.assigned_ports()?;
        assert_eq!(assignments.len(), 2);
        for (kind, port) in [
            (GatewayPort::Http, config.http_port),
            (GatewayPort::Https, config.https_port),
        ] {
            assert_eq!(
                assignments
                    .iter()
                    .find(|assignment| assignment.owner == PortOwner::Gateway(kind))
                    .map(|assignment| assignment.port),
                Some(port)
            );
        }
        assert_prepared(&paths, &config)?;
        Ok(())
    }

    #[test]
    fn routing_requires_gateway_identity_with_current_files_and_systemd_properties() -> Result<()> {
        let directory = tempdir()?;
        let environment = TestEnvironment::new(directory.path());
        let config = seed_gateway_ports(&environment)?;
        environment.unit_files_current.set(true);
        *environment.active_inspection.borrow_mut() = Some(ActivePfRedirectInspection {
            pf_enabled: true,
            pv_config: Some(config.clone()),
            pv_anchor_has_unparsed_rules: false,
            resolved_target_ports: BTreeSet::from([config.http_port, config.https_port]),
            has_unresolved_redirect_targets: false,
        });

        let (exit, unknown) = status_json(&environment, &config)?;
        assert_eq!(exit, ExitCode::FAILURE);
        environment.gateway_probe_succeeds.set(true);
        let (exit, active) = status_json(&environment, &config)?;
        assert_eq!(exit, ExitCode::SUCCESS);
        assert_eq!(environment.gateway_probes.get(), 2);
        assert_snapshot!(serde_json::to_string_pretty(&[unknown, active])?, @r#"
        [
          {
            "active_http_port": "<http>",
            "active_https_port": "<https>",
            "evidence": "systemd",
            "expected_http_port": "<http>",
            "expected_https_port": "<https>",
            "observed_at": "<time>",
            "state": "unknown"
          },
          {
            "active_http_port": "<http>",
            "active_https_port": "<https>",
            "evidence": "probe",
            "expected_http_port": "<http>",
            "expected_https_port": "<https>",
            "observed_at": "<time>",
            "state": "active"
          }
        ]
        "#);
        Ok(())
    }

    #[test]
    fn routing_preserves_native_absence_drift_and_unavailable_probe_states() -> Result<()> {
        let directory = tempdir()?;
        let environment = TestEnvironment::new(directory.path());
        let config = seed_gateway_ports(&environment)?;
        let exact = ActivePfRedirectInspection {
            pf_enabled: true,
            pv_config: Some(config.clone()),
            pv_anchor_has_unparsed_rules: false,
            resolved_target_ports: BTreeSet::from([config.http_port, config.https_port]),
            has_unresolved_redirect_targets: false,
        };
        let absent = ActivePfRedirectInspection {
            pf_enabled: false,
            pv_config: None,
            pv_anchor_has_unparsed_rules: false,
            resolved_target_ports: BTreeSet::new(),
            has_unresolved_redirect_targets: false,
        };
        let partial = ActivePfRedirectInspection {
            pv_anchor_has_unparsed_rules: true,
            resolved_target_ports: BTreeSet::from([config.http_port]),
            ..absent.clone()
        };
        let unresolved = ActivePfRedirectInspection {
            has_unresolved_redirect_targets: true,
            ..absent.clone()
        };
        let mut results = Vec::new();
        for (case, native, current, probe) in [
            ("absent with stale files", Some(absent), false, true),
            ("exact with stale files", Some(exact), false, true),
            ("partial", Some(partial), true, true),
            ("unresolved", Some(unresolved), true, true),
            ("unavailable and verified", None, true, true),
            ("unavailable and inconclusive", None, true, false),
            ("unavailable and stale", None, false, true),
        ] {
            environment.unit_files_current.set(current);
            *environment.active_inspection.borrow_mut() = native;
            environment.gateway_probe_succeeds.set(probe);
            let (exit, value) = status_json(&environment, &config)?;
            assert_eq!(environment.gateway_probes.get(), 0);
            assert_eq!(exit == ExitCode::SUCCESS, value["state"] == "active");
            results.push(format!(
                "{case}: {} ({})",
                value["state"], value["evidence"]
            ));
        }
        assert_snapshot!(results.join("\n"), @r#"
        absent with stale files: "inactive" ("systemd")
        exact with stale files: "drifted" ("systemd")
        partial: "drifted" ("systemd")
        unresolved: "drifted" ("systemd")
        unavailable and verified: "unknown" ("unavailable")
        unavailable and inconclusive: "unknown" ("unavailable")
        unavailable and stale: "drifted" ("unavailable")
        "#);
        Ok(())
    }

    #[test]
    fn routing_keeps_retained_native_targets_drifted_without_gateway_assignments() -> Result<()> {
        let directory = tempdir()?;
        let environment = TestEnvironment::new(directory.path());
        let config = PfRedirectConfig::new(pv_fake::port()?, pv_fake::port()?);
        let database = Database::open(&environment.paths())?;
        assert!(database.assigned_ports()?.is_empty());
        let mut results = Vec::new();
        for (resolved_target_ports, has_unresolved_redirect_targets) in [
            (BTreeSet::new(), false),
            (BTreeSet::new(), true),
            (BTreeSet::from([config.http_port]), false),
        ] {
            *environment.active_inspection.borrow_mut() = Some(ActivePfRedirectInspection {
                pf_enabled: false,
                pv_config: None,
                pv_anchor_has_unparsed_rules: false,
                resolved_target_ports,
                has_unresolved_redirect_targets,
            });
            let (exit, value) = status_json(&environment, &config)?;
            assert_eq!(exit, ExitCode::FAILURE);
            assert_eq!(environment.gateway_probes.get(), 0);
            results.push(value);
        }
        assert_snapshot!(serde_json::to_string_pretty(&results)?, @r#"
        [
          {
            "active_http_port": null,
            "active_https_port": null,
            "evidence": "systemd",
            "expected_http_port": null,
            "expected_https_port": null,
            "observed_at": "<time>",
            "state": "inactive"
          },
          {
            "active_http_port": null,
            "active_https_port": null,
            "evidence": "systemd",
            "expected_http_port": null,
            "expected_https_port": null,
            "observed_at": "<time>",
            "state": "drifted"
          },
          {
            "active_http_port": null,
            "active_https_port": null,
            "evidence": "systemd",
            "expected_http_port": null,
            "expected_https_port": null,
            "observed_at": "<time>",
            "state": "drifted"
          }
        ]
        "#);
        Ok(())
    }

    #[test]
    fn uninstall_skips_the_helper_only_when_native_units_are_absent() -> Result<()> {
        let directory = tempdir()?;
        for (case, absence) in [
            ("absent", Ok(true)),
            ("retained", Ok(false)),
            (
                "unreadable",
                Err(PlatformError::SystemIntegration(
                    "fixture bus unavailable".to_owned(),
                )),
            ),
        ] {
            let environment = TestEnvironment::new(&directory.path().join(case));
            prepare(&environment)?;
            let paths = environment.paths();
            let before = Database::open(&paths)?.assigned_ports()?;
            *environment.native_absence.borrow_mut() = Some(absence);
            *environment.error.borrow_mut() = Some(PlatformError::PrivilegedHelperUnavailable);

            let (outcome, stdout) = run_uninstall(&environment)?;
            match case {
                "absent" => assert_eq!(outcome?, ExitCode::SUCCESS),
                "retained" => assert!(matches!(
                    outcome,
                    Err(ExecuteError::Platform(
                        PlatformError::PrivilegedHelperUnavailable
                    ))
                )),
                _ => assert!(matches!(
                    outcome,
                    Err(ExecuteError::Platform(PlatformError::SystemIntegration(_)))
                )),
            }
            assert_eq!(environment.removals.get(), u32::from(case == "retained"));
            assert_eq!(Database::open(&paths)?.assigned_ports()?, before);
            for name in socket_proxy_unit_names() {
                assert_eq!(paths.ports_config().join(name).exists(), case != "absent");
            }
            if case == "absent" {
                assert_snapshot!(stdout.replace(directory.path().as_str(), "<tempdir>"), @r"
                Deleted prepared socket proxy unit: <tempdir>/absent/home/.pv/config/ports/pv-http.socket
                Deleted prepared socket proxy unit: <tempdir>/absent/home/.pv/config/ports/pv-http.service
                Deleted prepared socket proxy unit: <tempdir>/absent/home/.pv/config/ports/pv-https.socket
                Deleted prepared socket proxy unit: <tempdir>/absent/home/.pv/config/ports/pv-https.service
                Removed PV-owned system socket proxy units
                ");
            } else {
                assert!(stdout.is_empty());
            }
        }
        Ok(())
    }

    #[test]
    fn uninstall_waits_for_helper_removal_and_preserves_gateway_ports() -> Result<()> {
        let directory = tempdir()?;
        let environment = TestEnvironment::new(directory.path());
        let units = prepare(&environment)?;
        let paths = environment.paths();
        let before = Database::open(&paths)?.assigned_ports()?;
        *environment.error.borrow_mut() = Some(PlatformError::PortProxyMutation {
            operation: "remove",
            source: Box::new(PlatformError::PrivilegedHelperIo(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "fixture response lost after request",
            ))),
        });

        let (failed, stdout) = run_uninstall(&environment)?;
        assert!(matches!(
            failed,
            Err(ExecuteError::Platform(PlatformError::PortProxyMutation {
                operation: "remove",
                ..
            }))
        ));
        assert!(stdout.is_empty());
        assert_eq!(environment.removals.get(), 1);
        assert_eq!(Database::open(&paths)?.assigned_ports()?, before);
        for (name, content) in units {
            assert_eq!(
                fs::read_to_string(&paths.ports_config().join(name))?,
                content
            );
        }

        let (removed, stdout) = run_uninstall(&environment)?;
        assert_eq!(removed?, ExitCode::SUCCESS);
        assert_eq!(environment.removals.get(), 2);
        assert_eq!(Database::open(&paths)?.assigned_ports()?, before);
        for name in platform::socket_proxy_unit_names() {
            assert!(!paths.ports_config().join(name).exists());
        }
        assert_snapshot!(stdout.replace(directory.path().as_str(), "<tempdir>"), @r"
        Deleted prepared socket proxy unit: <tempdir>/home/.pv/config/ports/pv-http.socket
        Deleted prepared socket proxy unit: <tempdir>/home/.pv/config/ports/pv-http.service
        Deleted prepared socket proxy unit: <tempdir>/home/.pv/config/ports/pv-https.socket
        Deleted prepared socket proxy unit: <tempdir>/home/.pv/config/ports/pv-https.service
        Removed PV-owned system socket proxy units
        ");
        Ok(())
    }

    #[test]
    fn mutations_preserve_foreign_or_unreadable_units_without_calling_the_helper() -> Result<()> {
        let directory = tempdir()?;
        for unreadable in [false, true] {
            let environment = TestEnvironment::new(&directory.path().join(unreadable.to_string()));
            let units = prepare(&environment)?;
            let paths = environment.paths();
            let before = Database::open(&paths)?.assigned_ports()?;
            let foreign = environment.system.join("pv-https.service");
            if unreadable {
                fs::ensure_user_dir(&foreign)?;
            } else {
                fs::write_sensitive_file(&foreign, "administrator unit")?;
            }
            for install in [true, false] {
                let (outcome, _) = run_mutation(&environment, install)?;
                assert_eq!(outcome?, ExitCode::FAILURE);
            }
            assert_eq!(environment.removals.get(), 0);
            assert!(environment.applications.borrow().is_empty());
            assert_eq!(Database::open(&paths)?.assigned_ports()?, before);
            for (name, content) in units {
                assert_eq!(
                    fs::read_to_string(&paths.ports_config().join(name))?,
                    content
                );
            }
            if unreadable {
                assert!(foreign.is_dir());
            } else {
                assert_eq!(fs::read_to_string(&foreign)?, "administrator unit");
            }
        }
        Ok(())
    }

    #[test]
    fn install_repairs_owned_busy_sockets_using_the_stored_gateway_ports() -> Result<()> {
        let directory = tempdir()?;
        let mut environment = TestEnvironment::new(directory.path());
        let config = seed_gateway_ports(&environment)?;
        let paths = environment.paths();
        let before = Database::open(&paths)?.assigned_ports()?;
        for port in &mut environment.low_ports.borrow_mut().ports {
            port.available = false;
        }
        environment.repairable_ports = BTreeSet::from([80, 443]);
        environment.gateway_probe_succeeds.set(true);
        let (outcome, stdout) = run_mutation(&environment, true)?;
        assert_eq!(outcome?, ExitCode::SUCCESS);
        assert_eq!(
            environment.applications.borrow().as_slice(),
            slice::from_ref(&config)
        );
        assert_eq!(Database::open(&paths)?.assigned_ports()?, before);
        assert!(
            environment
                .low_ports
                .borrow()
                .ports
                .iter()
                .all(|port| !port.available)
        );
        assert_prepared(&paths, &config)?;
        assert!(!paths.pf_anchor_config().exists());
        let gateway = Database::open(&paths)?
            .runtime_observed_states()?
            .into_iter()
            .find(|state| state.subject == RuntimeSubject::Gateway)
            .context("Gateway observation missing")?;
        assert_eq!(gateway.status, RuntimeObservedStatus::Running);
        assert_eq!(
            gateway.message.as_deref(),
            Some("Gateway identity verified through ports 80 and 443 after socket proxy repair")
        );
        assert_eq!(environment.reconciliation_requests.get(), 0);
        let stdout = stdout
            .replace(directory.path().as_str(), "<tempdir>")
            .replace(&config.http_port.to_string(), "<http>")
            .replace(&config.https_port.to_string(), "<https>");
        assert_snapshot!(stdout, @r"
        Prepared PV socket proxy config
          unit path: <tempdir>/home/.pv/config/ports/pv-http.socket
          unit path: <tempdir>/home/.pv/config/ports/pv-http.service
          unit path: <tempdir>/home/.pv/config/ports/pv-https.socket
          unit path: <tempdir>/home/.pv/config/ports/pv-https.service
          HTTP proxy: 127.0.0.1:80 -> 127.0.0.1:<http>
          HTTPS proxy: 127.0.0.1:443 -> 127.0.0.1:<https>
        Installed system socket proxy config
        ");
        Ok(())
    }

    #[test]
    fn install_retains_ports_after_unknown_or_partial_apply_and_retries() -> Result<()> {
        let directory = tempdir()?;
        for (case, error, message, expected_status) in [
            (
                "unknown",
                unknown_apply(),
                "Low-port routing is unknown; run `pv ports:install`",
                RuntimeObservedStatus::Pending,
            ),
            (
                "partial",
                PlatformError::SystemIntegration(
                    "fixture apply failed after unit replacement".to_owned(),
                ),
                "Gateway config validation failed",
                RuntimeObservedStatus::Degraded,
            ),
        ] {
            let environment = TestEnvironment::new(&directory.path().join(case));
            let config = seed_gateway_ports(&environment)?;
            let paths = environment.paths();
            let mut database = Database::open(&paths)?;
            let before = database.assigned_ports()?;
            database.record_runtime_observed_snapshot(
                RuntimeSubject::Gateway,
                RuntimeObservedStatus::Degraded,
                Some(message),
            )?;
            *environment.apply_error.borrow_mut() = Some(error);
            let (failed, _) = run_mutation(&environment, true)?;
            assert!(matches!(
                (case, failed),
                (
                    "unknown",
                    Err(ExecuteError::Platform(PlatformError::PortProxyMutation {
                        operation: "apply",
                        ..
                    }))
                ) | (
                    "partial",
                    Err(ExecuteError::Platform(PlatformError::SystemIntegration(_)))
                )
            ));
            assert_eq!(database.assigned_ports()?, before);
            assert_prepared(&paths, &config)?;
            assert_eq!(environment.reconciliation_requests.get(), 0);
            let (retry, _) = run_mutation(&environment, true)?;
            assert_eq!(retry?, ExitCode::SUCCESS);
            assert_eq!(
                *environment.applications.borrow(),
                [config.clone(), config.clone()]
            );
            assert_eq!(database.assigned_ports()?, before);
            assert_prepared(&paths, &config)?;
            let gateway = database
                .runtime_observed_states()?
                .into_iter()
                .find(|state| state.subject == RuntimeSubject::Gateway)
                .context("Gateway observation missing")?;
            assert_eq!(gateway.status, expected_status);
            assert_eq!(
                gateway.message.as_deref(),
                Some(if case == "unknown" {
                    "Low-port routing repaired; Gateway readiness is pending reconciliation"
                } else {
                    message
                })
            );
            assert_eq!(environment.reconciliation_requests.get(), 1);
        }
        Ok(())
    }

    #[test]
    fn install_reports_foreign_busy_ports_before_writing_prepared_files() -> Result<()> {
        let directory = tempdir()?;
        let mut environment = TestEnvironment::new(directory.path());
        environment.low_ports.borrow_mut().ports[0] = LowPortState {
            port: 80,
            available: false,
            owners: vec![ListenerOwner {
                pid: 42,
                command: "nginx".to_owned(),
            }],
        };
        environment.repairable_ports.insert(443);
        let (outcome, stdout) = run_mutation(&environment, true)?;
        assert_eq!(outcome?, ExitCode::FAILURE);
        assert!(environment.applications.borrow().is_empty());
        assert!(!environment.paths().db().exists());
        assert!(!environment.paths().ports_config().exists());
        assert_snapshot!(stdout, @r"
        Socket proxy preparation failed
          Loopback TCP port 80 is unavailable. TCP listeners reported on this port: nginx (pid 42).
          find it: sudo ss -ltnp 'sport = :80'
          find unit: systemctl list-sockets --all
          stop unit: sudo systemctl disable --now <unit>
          Stop the conflicting service or socket, then run `pv ports:install` again.
        ");
        Ok(())
    }

    fn unknown_apply() -> PlatformError {
        PlatformError::PortProxyMutation {
            operation: "apply",
            source: Box::new(PlatformError::PrivilegedHelperIo(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "fixture response lost after apply",
            ))),
        }
    }

    #[test]
    fn failed_apply_releases_only_new_assignments_after_known_pre_mutation_errors() -> Result<()> {
        let directory = tempdir()?;
        for (had_http_assignment, had_https_assignment) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            for (case, error, may_have_changed_state) in [
                (
                    "unavailable",
                    PlatformError::PrivilegedHelperUnavailable,
                    false,
                ),
                (
                    "rejected",
                    PlatformError::PrivilegedHelperRejected {
                        message: "fixture rejected before mutation".to_owned(),
                    },
                    false,
                ),
                ("unknown", unknown_apply(), true),
                (
                    "partial",
                    PlatformError::SystemIntegration(
                        "fixture apply partially completed".to_owned(),
                    ),
                    true,
                ),
            ] {
                let environment = TestEnvironment::new(&directory.path().join(format!(
                    "{case}-{had_http_assignment}-{had_https_assignment}"
                )));
                let config = seed_gateway_ports(&environment)?;
                let mut database = Database::open(&environment.paths())?;
                let before = database.assigned_ports()?;
                *environment.apply_error.borrow_mut() = Some(error);
                assert!(
                    super::apply(
                        &environment,
                        &config,
                        &mut database,
                        had_http_assignment,
                        had_https_assignment
                    )
                    .is_err()
                );
                let expected = before
                    .into_iter()
                    .filter(|assignment| {
                        may_have_changed_state
                            || match assignment.owner {
                                PortOwner::Gateway(GatewayPort::Http) => had_http_assignment,
                                PortOwner::Gateway(GatewayPort::Https) => had_https_assignment,
                                _ => true,
                            }
                    })
                    .collect::<Vec<_>>();
                assert_eq!(database.assigned_ports()?, expected);
            }
        }
        Ok(())
    }

    #[test]
    fn reservation_cleanup_preserves_the_original_failure_and_its_own_cause() -> Result<()> {
        let directory = tempdir()?;
        let environment = TestEnvironment::new(directory.path());
        let config = seed_gateway_ports(&environment)?;
        let mut database =
            Database::open_read_only(&environment.paths())?.context("fixture database missing")?;
        let before = database.assigned_ports()?;
        *environment.apply_error.borrow_mut() = Some(PlatformError::PrivilegedHelperUnavailable);
        let error = super::apply(&environment, &config, &mut database, false, false)
            .err()
            .context("expected reservation cleanup failure")?;
        assert_snapshot!(error.to_string(), @"PV privileged helper is unavailable; run `pv setup`; could not release newly selected Gateway ports: SQLite error: attempt to write a readonly database");
        assert!(
            matches!(error, ExecuteError::PortPreparationCleanup { original, cleanup }
                if matches!(*original, ExecuteError::Platform(PlatformError::PrivilegedHelperUnavailable))
                    && matches!(*cleanup, ExecuteError::State(StateError::Sqlite(_)))
            )
        );
        assert_eq!(database.assigned_ports()?, before);
        Ok(())
    }
}
