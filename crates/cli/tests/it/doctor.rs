#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::io::{self, BufRead as _, Write as _};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use crate::support::run_pv;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::Environment;
use insta::{Settings, assert_debug_snapshot, assert_snapshot};
use platform::{
    DaemonRegistrationConfig, HELPER_PROTOCOL_VERSION, KeychainCertificate, KeychainTrustResult,
    PRIVILEGED_HELPER_VERSION, PrivilegedHelperStatus, ResolverConfig, ResolverFileState,
    SYSTEM_RESOLVER_TEST_PATH,
};
use state::{Database, PortRequest, PvPaths, RuntimeObservedStatus, RuntimeSubject};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    daemon_registration_path: PathBuf,
    resolver: std::cell::RefCell<ResolverFileState>,
    trusted_certificates: std::cell::RefCell<Vec<KeychainCertificate>>,
    helper_status: std::cell::RefCell<Option<PrivilegedHelperStatus>>,
    helper_authentication_error: std::cell::Cell<bool>,
    terminal_width: std::cell::Cell<Option<usize>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: home.as_std_path().to_path_buf(),
            daemon_registration_path: home
                .join("Library/LaunchAgents/com.prvious.pv.daemon.plist")
                .as_std_path()
                .to_path_buf(),
            resolver: std::cell::RefCell::new(ResolverFileState::Missing {
                path: SYSTEM_RESOLVER_TEST_PATH.into(),
            }),
            trusted_certificates: std::cell::RefCell::new(Vec::new()),
            helper_status: std::cell::RefCell::new(Some(PrivilegedHelperStatus {
                version: PRIVILEGED_HELPER_VERSION.to_string(),
                protocol_version: HELPER_PROTOCOL_VERSION,
                owner_uid: 501,
            })),
            helper_authentication_error: std::cell::Cell::new(false),
            terminal_width: std::cell::Cell::new(None),
        }
    }

    fn launch_agent_path_utf8(&self) -> anyhow::Result<camino::Utf8PathBuf> {
        camino::Utf8PathBuf::from_path_buf(self.daemon_registration_path.clone())
            .map_err(|path| anyhow::anyhow!("launch agent path is not UTF-8: {}", path.display()))
    }

    fn set_helper_missing(&self) {
        self.helper_status.replace(None);
    }

    fn set_helper_status(&self, version: &str, protocol_version: u32) {
        self.helper_status.replace(Some(PrivilegedHelperStatus {
            version: version.to_string(),
            protocol_version,
            owner_uid: 501,
        }));
    }

    fn set_helper_authentication_error(&self) {
        self.helper_authentication_error.set(true);
    }
}

impl Environment for TestEnvironment {
    /// Another web server holds port 80.
    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError> {
        Ok(platform::LowPortInspection {
            ports: vec![
                platform::LowPortState {
                    port: 80,
                    available: false,
                    owners: vec![platform::PortOwner {
                        pid: 4242,
                        command: "nginx".to_owned(),
                    }],
                },
                platform::LowPortState {
                    port: 443,
                    available: true,
                    owners: Vec::new(),
                },
            ],
        })
    }

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

    fn stdout_is_terminal(&self) -> bool {
        self.terminal_width.get().is_some()
    }

    fn terminal_width(&self) -> Option<usize> {
        self.terminal_width.get()
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn daemon_registration_path(&self) -> PathBuf {
        self.daemon_registration_path.clone()
    }

    fn inspect_system_resolver(&self, _expected: Option<&ResolverConfig>) -> ResolverFileState {
        self.resolver.borrow().clone()
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, platform::PlatformError> {
        Ok(self.trusted_certificates.borrow().clone())
    }

    fn privileged_helper_status(&self) -> Result<PrivilegedHelperStatus, platform::PlatformError> {
        if self.helper_authentication_error.get() {
            return Err(platform::PlatformError::PrivilegedHelperAuthentication(
                "peer UID 502 does not match installing UID 501".to_string(),
            ));
        }

        self.helper_status
            .borrow()
            .clone()
            .ok_or(platform::PlatformError::PrivilegedHelperUnavailable)
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
    state::fs::delete_file(&paths.daemon_socket())?;
    let json_health_server = spawn_health_server(&paths.daemon_socket())?;
    let json = run_pv(&["doctor", "--json"], &environment)?;
    join_health_server(json_health_server)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_eq!(json.exit_code, ExitCode::SUCCESS);
    assert!(json.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_passes_when_required_checks_pass",
        tempdir.path(),
        (output, json),
    );

    Ok(())
}

#[test]
fn doctor_accepts_helper_only_version_from_active_release_metadata() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let environment = TestEnvironment::new(paths.home());
    seed_required_checks(&paths, &environment, true)?;
    environment.set_helper_status("3.1.0", 3);
    write_file(
        &paths
            .app_release_helper(env!("CARGO_PKG_VERSION"))
            .with_file_name("pv-helper.json"),
        "{\n  \"version\": \"3.1.0\",\n  \"protocol_version\": 3,\n  \"sha256\": \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n}\n",
    )?;
    let health_server = spawn_health_server(&paths.daemon_socket())?;

    let output = run_pv(&["doctor"], &environment)?;
    join_health_server(health_server)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(
        output
            .stdout
            .contains("available at version 3.1.0 with protocol 3")
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
    let port_conflict =
        "Port 80 is in use by another program; free it, then run `pv daemon:restart`.";
    database.fail_job(&job.id, port_conflict)?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Failed,
        Some(port_conflict),
    )?;

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot("doctor_fails_with_repair_commands", tempdir.path(), output);

    Ok(())
}

#[test]
fn doctor_on_a_terminal_groups_checks_with_repair_hints() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    // Wide enough that no detail wraps, so wrap points never depend on how
    // long this machine's temp path is.
    environment.terminal_width.set(Some(160));
    let health_server = spawn_health_server(&paths.daemon_socket())?;
    let healthy = run_pv(&["doctor", "--no-color"], &environment)?;
    join_health_server(health_server)?;
    let mut database = Database::open(&paths)?;
    let job = database.start_job("reconcile", "system")?;
    database.fail_job(&job.id, "Gateway failed to start")?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Degraded,
        Some("Gateway restarted twice"),
    )?;
    let failing = run_pv(&["doctor", "--no-color"], &environment)?;

    assert_eq!(healthy.exit_code, ExitCode::SUCCESS);
    assert_eq!(failing.exit_code, ExitCode::FAILURE);
    assert!(healthy.stderr.is_empty());
    assert!(failing.stderr.is_empty());
    doctor_settings(tempdir.path()).bind(|| {
        assert_snapshot!("doctor_on_a_terminal_healthy", healthy.stdout);
        assert_snapshot!("doctor_on_a_terminal_failing", failing.stdout);
    });

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
fn doctor_fails_one_check_and_continues_the_others() -> anyhow::Result<()> {
    type BreakCheck = fn(&TestEnvironment) -> anyhow::Result<()>;
    let cases: [(&'static str, BreakCheck); 4] = [
        (
            "doctor_fails_when_system_resolver_is_missing",
            |environment| {
                environment.resolver.replace(ResolverFileState::Missing {
                    path: SYSTEM_RESOLVER_TEST_PATH.into(),
                });
                Ok(())
            },
        ),
        (
            "doctor_fails_when_system_ca_trust_is_missing",
            |environment| {
                environment.trusted_certificates.borrow_mut().clear();
                Ok(())
            },
        ),
        (
            "doctor_reports_missing_helper_and_continues_other_checks",
            |environment| {
                environment.set_helper_missing();
                Ok(())
            },
        ),
        (
            "doctor_reports_cross_account_helper_recovery",
            |environment| {
                environment.set_helper_authentication_error();
                Ok(())
            },
        ),
    ];

    for (name, break_check) in cases {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        let environment = TestEnvironment::new(&home);
        seed_required_checks(&paths, &environment, true)?;
        break_check(&environment)?;
        let health_server = spawn_health_server(&paths.daemon_socket())?;

        let output = run_pv(&["doctor"], &environment)?;
        join_health_server(health_server)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE, "{name}");
        assert!(
            output.stdout.contains("[pass] Artifact manifest cache"),
            "{name}"
        );
        assert_doctor_snapshot(name, tempdir.path(), output);
    }

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

fn seed_required_checks(
    paths: &PvPaths,
    environment: &TestEnvironment,
    include_manifest_cache: bool,
) -> anyhow::Result<()> {
    Database::open(paths)?.assign_port(PortRequest::dns(35353, 35353, 35353), |_port| true)?;

    let launch_agent = DaemonRegistrationConfig::new(
        "/bin/pv",
        paths.daemon_registration_stdout_log(),
        paths.daemon_registration_stderr_log(),
    );
    platform::write_daemon_registration_file(
        &environment.launch_agent_path_utf8()?,
        &launch_agent,
    )?;
    environment.resolver.replace(ResolverFileState::Current {
        path: SYSTEM_RESOLVER_TEST_PATH.into(),
        port: 35353,
    });
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
    let release_helper = paths.app_release_helper(env!("CARGO_PKG_VERSION"));
    state::fs::write_sensitive_file(&release_helper, "pv helper\n")?;
    state::fs::write_sensitive_file(
        &release_helper.with_file_name("pv-helper.json"),
        "{\n  \"version\": \"3.0.0\",\n  \"protocol_version\": 3,\n  \"sha256\": \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n}\n",
    )?;

    if include_manifest_cache {
        state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), "{}")?;
    }

    Ok(())
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

fn assert_doctor_snapshot(name: &'static str, tempdir: &Utf8Path, snapshot: impl std::fmt::Debug) {
    doctor_settings(tempdir).bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}

fn doctor_settings(tempdir: &Utf8Path) -> Settings {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"job_[0-9]+", "<job-id>");
    settings.add_filter(r"[0-9a-f]{64}", "<fingerprint>");
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings
}
