use std::ffi::OsString;
use std::io::{self, BufRead as _, Write as _};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use platform::{
    KeychainCertificate, KeychainTrustResult, LaunchAgentConfig, PfConfReference, PfRedirectConfig,
    ResolverConfig,
};
use state::{Database, PvPaths, RuntimeObservedStatus, RuntimeSubject};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    launch_agent_path: PathBuf,
    resolver_path: PathBuf,
    pf_anchor_path: PathBuf,
    pf_conf_path: PathBuf,
    active_pf_config: std::cell::RefCell<Option<PfRedirectConfig>>,
    trusted_certificates: std::cell::RefCell<Vec<KeychainCertificate>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: home.as_std_path().to_path_buf(),
            launch_agent_path: home
                .join("Library/LaunchAgents/com.prvious.pv.daemon.plist")
                .as_std_path()
                .to_path_buf(),
            resolver_path: home.join("etc/resolver/test").as_std_path().to_path_buf(),
            pf_anchor_path: home
                .join("etc/pf.anchors/com.prvious.pv")
                .as_std_path()
                .to_path_buf(),
            pf_conf_path: home.join("etc/pf.conf").as_std_path().to_path_buf(),
            active_pf_config: std::cell::RefCell::new(None),
            trusted_certificates: std::cell::RefCell::new(Vec::new()),
        }
    }

    fn launch_agent_path_utf8(&self) -> anyhow::Result<camino::Utf8PathBuf> {
        camino::Utf8PathBuf::from_path_buf(self.launch_agent_path.clone())
            .map_err(|path| anyhow::anyhow!("launch agent path is not UTF-8: {}", path.display()))
    }

    fn resolver_path_utf8(&self) -> anyhow::Result<Utf8PathBuf> {
        utf8_path_buf(&self.resolver_path, "resolver path")
    }

    fn pf_anchor_path_utf8(&self) -> anyhow::Result<Utf8PathBuf> {
        utf8_path_buf(&self.pf_anchor_path, "pf anchor path")
    }

    fn pf_conf_path_utf8(&self) -> anyhow::Result<Utf8PathBuf> {
        utf8_path_buf(&self.pf_conf_path, "pf.conf path")
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
        Ok(self.current_dir.clone())
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

    fn launch_agent_path(&self) -> PathBuf {
        self.launch_agent_path.clone()
    }

    fn resolver_test_path(&self) -> PathBuf {
        self.resolver_path.clone()
    }

    fn pf_anchor_path(&self) -> PathBuf {
        self.pf_anchor_path.clone()
    }

    fn pf_conf_path(&self) -> PathBuf {
        self.pf_conf_path.clone()
    }

    fn active_pf_redirect_config(
        &self,
    ) -> Result<Option<PfRedirectConfig>, platform::PlatformError> {
        Ok(self.active_pf_config.borrow().clone())
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, platform::PlatformError> {
        Ok(self.trusted_certificates.borrow().clone())
    }
}

#[test]
fn doctor_passes_when_required_checks_pass() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    let health_server = spawn_health_server(&paths.daemon_socket())?;

    let output = run_pv(&["doctor"], &environment)?;
    join_health_server(health_server)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_passes_when_required_checks_pass",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn doctor_fails_with_repair_commands() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    let mut database = Database::open(&paths)?;
    let job = database.start_job("reconcile", "system")?;
    database.fail_job(&job.id, "Gateway failed to start")?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Failed,
        Some("Gateway failed to start"),
    )?;

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot("doctor_fails_with_repair_commands", tempdir.path(), output);

    Ok(())
}

#[test]
fn doctor_fails_when_daemon_socket_is_stale() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    write_file(&paths.daemon_socket(), "")?;

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_fails_when_daemon_socket_is_stale",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn doctor_fails_when_system_resolver_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    delete_optional_file(&environment.resolver_path_utf8()?)?;
    let health_server = spawn_health_server(&paths.daemon_socket())?;

    let output = run_pv(&["doctor"], &environment)?;
    join_health_server(health_server)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_fails_when_system_resolver_is_missing",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn doctor_fails_when_active_pf_redirects_are_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    *environment.active_pf_config.borrow_mut() = None;
    let health_server = spawn_health_server(&paths.daemon_socket())?;

    let output = run_pv(&["doctor"], &environment)?;
    join_health_server(health_server)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_fails_when_active_pf_redirects_are_missing",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn doctor_fails_when_system_ca_trust_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    environment.trusted_certificates.borrow_mut().clear();
    let health_server = spawn_health_server(&paths.daemon_socket())?;

    let output = run_pv(&["doctor"], &environment)?;
    join_health_server(health_server)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_fails_when_system_ca_trust_is_missing",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn doctor_warnings_do_not_fail() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, false)?;
    let health_server = spawn_health_server(&paths.daemon_socket())?;

    let output = run_pv(&["doctor"], &environment)?;
    join_health_server(health_server)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot("doctor_warnings_do_not_fail", tempdir.path(), output);

    Ok(())
}

#[test]
fn doctor_is_read_only() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert!(!state::fs::path_exists(paths.root()));
    assert_doctor_snapshot("doctor_is_read_only", tempdir.path(), output);

    Ok(())
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
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

fn seed_required_checks(
    paths: &PvPaths,
    environment: &TestEnvironment,
    include_manifest_cache: bool,
) -> anyhow::Result<()> {
    Database::open(paths)?;

    let launch_agent = LaunchAgentConfig::new(
        "/bin/pv",
        paths.launchd_stdout_log(),
        paths.launchd_stderr_log(),
    );
    platform::write_launch_agent_file(&environment.launch_agent_path_utf8()?, &launch_agent)?;
    state::fs::write_sensitive_file(
        &paths.resolver_config(),
        &ResolverConfig::new(35353).render(),
    )?;
    state::fs::write_sensitive_file(
        &environment.resolver_path_utf8()?,
        &ResolverConfig::new(35353).render(),
    )?;
    let pf_redirect_config = PfRedirectConfig::new(48080, 48443);
    state::fs::write_sensitive_file(
        &paths.pf_anchor_config(),
        &pf_redirect_config.render_anchor(),
    )?;
    state::fs::write_sensitive_file(&paths.pf_conf_reference_config(), &PfConfReference.render())?;
    state::fs::write_sensitive_file(
        &environment.pf_anchor_path_utf8()?,
        &pf_redirect_config.render_anchor(),
    )?;
    state::fs::write_sensitive_file(&environment.pf_conf_path_utf8()?, &PfConfReference.render())?;
    *environment.active_pf_config.borrow_mut() = Some(pf_redirect_config);
    let ca = platform::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &ca.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &ca.private_key_pem)?;
    environment
        .trusted_certificates
        .borrow_mut()
        .push(KeychainCertificate {
            metadata: ca.metadata,
            trust: KeychainTrustResult::TrustRoot,
        });

    if include_manifest_cache {
        state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), "{}")?;
    }

    Ok(())
}

fn utf8_path_buf(path: &Path, label: &str) -> anyhow::Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|path| anyhow::anyhow!("{label} is not UTF-8: {}", path.display()))
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI doctor tests bind a fixture daemon health socket"
)]
fn spawn_health_server(
    socket_path: &Utf8Path,
) -> anyhow::Result<thread::JoinHandle<anyhow::Result<()>>> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path.as_std_path())?;
    let handle = thread::spawn(move || -> anyhow::Result<()> {
        listener.set_nonblocking(true)?;
        let started_at = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _address)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if started_at.elapsed() >= Duration::from_secs(1) {
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        };
        stream.set_nonblocking(false)?;
        let mut request = String::new();
        let mut reader = io::BufReader::new(stream.try_clone()?);
        reader.read_line(&mut request)?;
        writeln!(
            stream,
            "{{\"type\":\"response\",\"protocol_version\":{},\"status\":\"ok\",\"message\":\"daemon healthy\"}}",
            daemon::PROTOCOL_VERSION
        )?;

        Ok(())
    });

    Ok(handle)
}

fn join_health_server(handle: thread::JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
    handle
        .join()
        .map_err(|error| anyhow::anyhow!("health server thread panicked: {error:?}"))?
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI doctor tests create fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI doctor tests delete fixture files"
)]
fn delete_optional_file(path: &Utf8Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn assert_doctor_snapshot(name: &'static str, tempdir: &Utf8Path, snapshot: impl std::fmt::Debug) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"job_[0-9]+", "<job-id>");
    settings.add_filter(r"[0-9a-f]{64}", "<fingerprint>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
