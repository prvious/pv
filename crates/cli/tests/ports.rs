use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use platform::{PfConfReference, PfRedirectConfig};
use state::{Database, PortOwner, PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    pf_anchor_path: PathBuf,
    pf_conf_path: PathBuf,
    listening_ports: BTreeSet<u16>,
    active_pf_config: RefCell<Option<PfRedirectConfig>>,
    active_pf_privilege_modes: RefCell<Vec<platform::PrivilegeMode>>,
    active_pf_read_fails_when_unloaded: bool,
    operations: RefCell<Vec<String>>,
}

impl TestEnvironment {
    fn new(
        home: &Utf8Path,
        current_dir: &Utf8Path,
        pf_anchor_path: &Utf8Path,
        pf_conf_path: &Utf8Path,
    ) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            pf_anchor_path: pf_anchor_path.as_std_path().to_path_buf(),
            pf_conf_path: pf_conf_path.as_std_path().to_path_buf(),
            listening_ports: BTreeSet::new(),
            active_pf_config: RefCell::new(None),
            active_pf_privilege_modes: RefCell::new(Vec::new()),
            active_pf_read_fails_when_unloaded: false,
            operations: RefCell::new(Vec::new()),
        }
    }

    fn with_listener(mut self, port: u16) -> Self {
        self.listening_ports.insert(port);
        self
    }

    fn with_active_pf_read_failing_when_unloaded(mut self) -> Self {
        self.active_pf_read_fails_when_unloaded = true;
        self
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.borrow().clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn pf_anchor_path(&self) -> PathBuf {
        self.pf_anchor_path.clone()
    }

    fn pf_conf_path(&self) -> PathBuf {
        self.pf_conf_path.clone()
    }

    fn loopback_tcp_listener_ports(&self) -> Result<BTreeSet<u16>, platform::PlatformError> {
        Ok(self.listening_ports.clone())
    }

    fn install_pf_redirects(
        &self,
        prepared_anchor_path: &Utf8Path,
        prepared_reference_path: &Utf8Path,
        system_anchor_path: &Utf8Path,
        system_pf_conf_path: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        let anchor = state::fs::read_to_string(prepared_anchor_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        let reference = state::fs::read_to_string(prepared_reference_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        write_file(system_anchor_path, &anchor)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        write_file(system_pf_conf_path, &reference)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        *self.active_pf_config.borrow_mut() = PfRedirectConfig::parse_anchor(&anchor);
        self.operations.borrow_mut().push(format!(
            "install pf {prepared_anchor_path} {prepared_reference_path} -> {system_anchor_path} {system_pf_conf_path}"
        ));

        Ok(())
    }

    fn active_pf_redirect_config(
        &self,
    ) -> Result<Option<PfRedirectConfig>, platform::PlatformError> {
        self.active_pf_redirect_config_with_privilege_mode(platform::PrivilegeMode::NonInteractive)
    }

    fn active_pf_redirect_config_with_privilege_mode(
        &self,
        privilege_mode: platform::PrivilegeMode,
    ) -> Result<Option<PfRedirectConfig>, platform::PlatformError> {
        self.active_pf_privilege_modes
            .borrow_mut()
            .push(privilege_mode);
        if self.active_pf_read_fails_when_unloaded && self.active_pf_config.borrow().is_none() {
            return Err(platform::PlatformError::SystemIntegrationCommandStatus {
                command: "/sbin/pfctl -s nat".to_string(),
                status: "exit status: 1".to_string(),
            });
        }

        Ok(self.active_pf_config.borrow().clone())
    }

    fn remove_pf_redirects(
        &self,
        system_anchor_path: &Utf8Path,
        system_pf_conf_path: &Utf8Path,
        candidate_dir: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        delete_optional_file(system_anchor_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        delete_optional_file(system_pf_conf_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        *self.active_pf_config.borrow_mut() = None;
        self.operations.borrow_mut().push(format!(
            "remove pf {system_anchor_path} {system_pf_conf_path} via {candidate_dir}"
        ));

        Ok(())
    }
}

#[test]
fn ports_install_writes_prepared_and_system_pf_artifacts() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    );

    let output = run_pv(&["ports:install"], &environment)?;
    let paths = pv_paths(&home);
    let prepared_anchor = read_required_file(&paths.pf_anchor_config())?;
    let prepared_reference = read_required_file(&paths.pf_conf_reference_config())?;
    let parsed_anchor = PfRedirectConfig::parse_anchor(&prepared_anchor)
        .ok_or_else(|| anyhow::anyhow!("prepared pf anchor did not parse"))?;
    let parsed_reference = PfConfReference::parse_block(&prepared_reference)
        .ok_or_else(|| anyhow::anyhow!("prepared pf.conf reference did not parse"))?;
    let system_anchor_after_install = read_required_file(&system_anchor_path)?;
    let system_pf_conf_after_install = read_required_file(&system_pf_conf_path)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert_eq!(parsed_anchor.http_port, 48080);
    assert_eq!(parsed_anchor.https_port, 48443);
    assert_eq!(parsed_reference, PfConfReference);
    assert_eq!(system_anchor_after_install, prepared_anchor);
    assert_eq!(system_pf_conf_after_install, prepared_reference);

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            paths.pf_anchor_config(),
            prepared_anchor,
            paths.pf_conf_reference_config(),
            prepared_reference,
            system_anchor_after_install,
            system_pf_conf_after_install,
            environment.operations.borrow().clone(),
        ));
    });

    Ok(())
}

#[test]
fn ports_install_refuses_non_pv_owned_system_anchor() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    );
    let conflict_anchor =
        "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\n";
    write_file(&system_anchor_path, conflict_anchor)?;

    let output = run_pv(&["ports:install"], &environment)?;
    let system_anchor_after_install = read_required_file(&system_anchor_path)?;
    let assignments = Database::open(&pv_paths(&home))?.assigned_ports()?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert_eq!(system_anchor_after_install, conflict_anchor);
    assert!(!assignments.iter().any(|assignment| {
        matches!(
            assignment.owner,
            PortOwner::Gateway(state::GatewayPort::Http)
        ) || matches!(
            assignment.owner,
            PortOwner::Gateway(state::GatewayPort::Https)
        )
    }));
    assert!(environment.operations.borrow().is_empty());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, system_anchor_after_install));
    });

    Ok(())
}

#[test]
fn ports_install_reloads_current_files_when_active_redirects_are_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    );
    let paths = pv_paths(&home);
    let anchor = PfRedirectConfig::new(48080, 48443).render_anchor();
    let reference = PfConfReference.render();

    write_file(&paths.pf_anchor_config(), &anchor)?;
    write_file(&paths.pf_conf_reference_config(), &reference)?;
    write_file(&system_anchor_path, &anchor)?;
    write_file(&system_pf_conf_path, &reference)?;

    let output = run_pv(&["ports:install"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(
        environment
            .operations
            .borrow()
            .iter()
            .any(|operation| { operation.starts_with("install pf ") })
    );
    assert_eq!(
        *environment.active_pf_config.borrow(),
        Some(PfRedirectConfig::new(48080, 48443))
    );
    assert_eq!(
        environment.active_pf_privilege_modes.borrow().as_slice(),
        [
            platform::PrivilegeMode::Interactive,
            platform::PrivilegeMode::Interactive
        ]
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations.borrow().clone()));
    });

    Ok(())
}

#[test]
fn ports_install_skips_active_rule_inspection_before_first_install() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    )
    .with_active_pf_read_failing_when_unloaded();

    let output = run_pv(&["ports:install"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(
        environment
            .operations
            .borrow()
            .iter()
            .any(|operation| { operation.starts_with("install pf ") })
    );
    assert_eq!(
        *environment.active_pf_config.borrow(),
        Some(PfRedirectConfig::new(48080, 48443))
    );
    assert_eq!(
        environment.active_pf_privilege_modes.borrow().as_slice(),
        [platform::PrivilegeMode::Interactive]
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations.borrow().clone()));
    });

    Ok(())
}

#[test]
fn ports_install_fails_on_low_port_conflict_before_writing_prepared_artifacts() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    )
    .with_listener(80);
    let paths = pv_paths(&home);

    let output = run_pv(&["ports:install"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert!(read_optional_file(&paths.pf_anchor_config())?.is_none());
    assert!(read_optional_file(&paths.pf_conf_reference_config())?.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn ports_status_reports_prepared_and_system_pf_states_without_mutating_state() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    );
    let paths = pv_paths(&home);
    let current_anchor = PfRedirectConfig::new(48080, 48443).render_anchor();
    let stale_anchor = PfRedirectConfig::new(45000, 45001).render_anchor();
    let current_reference = PfConfReference.render();

    let missing = run_pv(&["ports:status"], &environment)?;
    let database_after_missing = read_optional_file(paths.db())?;
    let prepared_anchor_after_missing = read_optional_file(&paths.pf_anchor_config())?;
    let prepared_reference_after_missing = read_optional_file(&paths.pf_conf_reference_config())?;

    write_file(&paths.pf_anchor_config(), &current_anchor)?;
    write_file(&paths.pf_conf_reference_config(), &current_reference)?;
    let prepared_only = run_pv(&["ports:status"], &environment)?;

    write_file(&system_anchor_path, &current_anchor)?;
    write_file(&system_pf_conf_path, &current_reference)?;
    let current = run_pv(&["ports:status"], &environment)?;

    write_file(&system_anchor_path, &stale_anchor)?;
    write_file(&system_pf_conf_path, "anchor \"com.prvious.pv\"\n")?;
    let stale_and_conflict = run_pv(&["ports:status"], &environment)?;

    assert_eq!(missing.exit_code, ExitCode::SUCCESS);
    assert_eq!(prepared_only.exit_code, ExitCode::SUCCESS);
    assert_eq!(current.exit_code, ExitCode::SUCCESS);
    assert_eq!(stale_and_conflict.exit_code, ExitCode::SUCCESS);
    assert!(database_after_missing.is_none());
    assert!(prepared_anchor_after_missing.is_none());
    assert!(prepared_reference_after_missing.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((missing, prepared_only, current, stale_and_conflict,));
    });

    Ok(())
}

#[test]
fn ports_uninstall_removes_prepared_and_pv_owned_system_pf_artifacts() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    );
    let paths = pv_paths(&home);
    let anchor = PfRedirectConfig::new(48080, 48443).render_anchor();
    let reference = PfConfReference.render();

    write_file(&paths.pf_anchor_config(), &anchor)?;
    write_file(&paths.pf_conf_reference_config(), &reference)?;
    write_file(&system_anchor_path, &anchor)?;
    write_file(&system_pf_conf_path, &reference)?;

    let output = run_pv(&["ports:uninstall"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert!(read_optional_file(&paths.pf_anchor_config())?.is_none());
    assert!(read_optional_file(&paths.pf_conf_reference_config())?.is_none());
    assert!(read_optional_file(&system_anchor_path)?.is_none());
    assert!(read_optional_file(&system_pf_conf_path)?.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations.borrow().clone()));
    });

    Ok(())
}

#[test]
fn ports_install_reuses_persisted_gateway_ports_even_when_they_have_listeners() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    )
    .with_listener(48080)
    .with_listener(48443);
    let paths = pv_paths(&home);
    let mut database = Database::open(&paths)?;
    let seeded = database.assign_gateway_ports(|port| port == 48080 || port == 48443)?;
    drop(database);

    let output = run_pv(&["ports:install"], &environment)?;
    let prepared_anchor = read_required_file(&paths.pf_anchor_config())?;
    let parsed_anchor = PfRedirectConfig::parse_anchor(&prepared_anchor)
        .ok_or_else(|| anyhow::anyhow!("prepared pf anchor did not parse"))?;
    let assignments = Database::open(&paths)?.assigned_ports()?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(parsed_anchor.http_port, seeded.http.port);
    assert_eq!(parsed_anchor.https_port, seeded.https.port);
    assert!(assignments.iter().any(|assignment| {
        assignment.owner == PortOwner::Gateway(state::GatewayPort::Http)
            && assignment.port == seeded.http.port
    }));
    assert!(assignments.iter().any(|assignment| {
        assignment.owner == PortOwner::Gateway(state::GatewayPort::Https)
            && assignment.port == seeded.https.port
    }));

    Ok(())
}

#[test]
fn ports_install_uses_fallback_gateway_ports_when_preferred_ports_are_busy() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        &system_anchor_path,
        &system_pf_conf_path,
    )
    .with_listener(48080)
    .with_listener(48443);
    let paths = pv_paths(&home);

    let output = run_pv(&["ports:install"], &environment)?;
    let prepared_anchor = read_required_file(&paths.pf_anchor_config())?;
    let parsed_anchor = PfRedirectConfig::parse_anchor(&prepared_anchor)
        .ok_or_else(|| anyhow::anyhow!("prepared pf anchor did not parse"))?;
    let assignments = Database::open(&paths)?.assigned_ports()?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(parsed_anchor.http_port, 45000);
    assert_eq!(parsed_anchor.https_port, 45001);
    assert!(assignments.iter().any(|assignment| {
        assignment.owner == PortOwner::Gateway(state::GatewayPort::Http)
            && assignment.port == parsed_anchor.http_port
    }));
    assert!(assignments.iter().any(|assignment| {
        assignment.owner == PortOwner::Gateway(state::GatewayPort::Https)
            && assignment.port == parsed_anchor.https_port
    }));

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, prepared_anchor));
    });

    Ok(())
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

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

fn read_required_file(path: &Utf8Path) -> anyhow::Result<String> {
    read_optional_file(path)?
        .ok_or_else(|| anyhow::anyhow!("expected fixture file to exist: {path}"))
}

fn read_optional_file(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    state::fs::write_sensitive_file(path, content)?;

    Ok(())
}

fn delete_optional_file(path: &Utf8Path) -> anyhow::Result<()> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn assert_no_privileged_guidance(output: &str) {
    for pattern in ["sudo", "pfctl", "sudo rm", "sudo install"] {
        assert!(
            !output.contains(pattern),
            "output contains privileged guidance `{pattern}`: {output}"
        );
    }
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assertion);
}
