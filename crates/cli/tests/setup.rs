use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use platform::{
    KeychainCertificate, KeychainTrustResult, LAUNCH_AGENT_LABEL, LaunchAgentConfig,
    LocalCaMetadata, PfConfReference, PfRedirectConfig, ResolverConfig,
};
use serde_json::json;
use state::{Database, ManagedResourceDesiredState, PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    current_exe: PathBuf,
    launch_agent_path: PathBuf,
    resolver_path: PathBuf,
    pf_anchor_path: PathBuf,
    pf_conf_path: PathBuf,
    shell: Option<OsString>,
    certificates: Mutex<Vec<KeychainCertificate>>,
    active_pf_config: Mutex<Option<PfRedirectConfig>>,
    operations: Mutex<Vec<String>>,
    stdin_terminal: bool,
    input: Mutex<VecDeque<String>>,
}

impl TestEnvironment {
    fn new(paths: TestEnvironmentPaths<'_>, shell: Option<OsString>) -> Self {
        Self {
            home: paths.home.as_std_path().to_path_buf(),
            current_dir: paths.current_dir.as_std_path().to_path_buf(),
            current_exe: paths.current_exe.as_std_path().to_path_buf(),
            launch_agent_path: paths.launch_agent_path.as_std_path().to_path_buf(),
            resolver_path: paths.resolver_path.as_std_path().to_path_buf(),
            pf_anchor_path: paths.pf_anchor_path.as_std_path().to_path_buf(),
            pf_conf_path: paths.pf_conf_path.as_std_path().to_path_buf(),
            shell,
            certificates: Mutex::new(Vec::new()),
            active_pf_config: Mutex::new(None),
            operations: Mutex::new(Vec::new()),
            stdin_terminal: false,
            input: Mutex::new(VecDeque::new()),
        }
    }

    fn operations(&self) -> Vec<String> {
        lock(&self.operations).clone()
    }

    fn certificates(&self) -> Vec<KeychainCertificate> {
        lock(&self.certificates).clone()
    }
}

#[derive(Debug)]
struct TestEnvironmentPaths<'path> {
    home: &'path Utf8Path,
    current_dir: &'path Utf8Path,
    current_exe: &'path Utf8Path,
    launch_agent_path: &'path Utf8Path,
    resolver_path: &'path Utf8Path,
    pf_anchor_path: &'path Utf8Path,
    pf_conf_path: &'path Utf8Path,
}

impl Environment for TestEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        if key == "SHELL" {
            self.shell.clone()
        } else {
            None
        }
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(self.current_exe.clone())
    }

    fn stdin_is_terminal(&self) -> bool {
        self.stdin_terminal
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(lock(&self.input).pop_front().unwrap_or_default())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn launch_agent_path(&self) -> PathBuf {
        self.launch_agent_path.clone()
    }

    fn bootstrap_launch_agent(&self, plist_path: &Utf8Path) -> Result<(), platform::PlatformError> {
        lock(&self.operations).push(format!("bootstrap {plist_path}"));

        Ok(())
    }

    fn bootout_launch_agent(&self) -> Result<(), platform::PlatformError> {
        lock(&self.operations).push(format!("bootout {LAUNCH_AGENT_LABEL}"));

        Ok(())
    }

    fn kickstart_launch_agent(&self) -> Result<(), platform::PlatformError> {
        lock(&self.operations).push(format!("kickstart {LAUNCH_AGENT_LABEL}"));

        Ok(())
    }

    fn resolver_test_path(&self) -> PathBuf {
        self.resolver_path.clone()
    }

    fn install_resolver_config(
        &self,
        prepared_path: &Utf8Path,
        system_path: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        let content = state::fs::read_to_string(prepared_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        write_file(system_path, &content)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        lock(&self.operations).push(format!("install resolver {prepared_path} -> {system_path}"));

        Ok(())
    }

    fn remove_resolver_config(
        &self,
        system_path: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        delete_optional_file(system_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        lock(&self.operations).push(format!("remove resolver {system_path}"));

        Ok(())
    }

    fn pf_anchor_path(&self) -> PathBuf {
        self.pf_anchor_path.clone()
    }

    fn pf_conf_path(&self) -> PathBuf {
        self.pf_conf_path.clone()
    }

    fn loopback_tcp_listener_ports(&self) -> Result<BTreeSet<u16>, platform::PlatformError> {
        Ok(BTreeSet::new())
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
        *lock(&self.active_pf_config) = PfRedirectConfig::parse_anchor(&anchor);
        lock(&self.operations).push(format!(
            "install pf {prepared_anchor_path} {prepared_reference_path} -> {system_anchor_path} {system_pf_conf_path}"
        ));

        Ok(())
    }

    fn active_pf_redirect_config(
        &self,
    ) -> Result<Option<PfRedirectConfig>, platform::PlatformError> {
        Ok(lock(&self.active_pf_config).clone())
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
        *lock(&self.active_pf_config) = None;
        lock(&self.operations).push(format!(
            "remove pf {system_anchor_path} {system_pf_conf_path} via {candidate_dir}"
        ));

        Ok(())
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, platform::PlatformError> {
        Ok(lock(&self.certificates).clone())
    }

    fn trust_system_ca(&self, certificate_path: &Utf8Path) -> Result<(), platform::PlatformError> {
        let certificate_pem = state::fs::read_to_string(certificate_path)
            .map_err(|error| platform::PlatformError::SystemIntegration(error.to_string()))?;
        let metadata = LocalCaMetadata::from_certificate_pem(&certificate_pem)?;
        let mut certificates = lock(&self.certificates);

        certificates.retain(|certificate| certificate.metadata.fingerprint != metadata.fingerprint);
        certificates.push(KeychainCertificate {
            metadata: metadata.clone(),
            trust: KeychainTrustResult::TrustRoot,
        });
        lock(&self.operations).push(format!("trust {}", metadata.fingerprint));

        Ok(())
    }

    fn untrust_system_ca(&self, fingerprint: &str) -> Result<(), platform::PlatformError> {
        lock(&self.certificates)
            .retain(|certificate| certificate.metadata.fingerprint != fingerprint);
        lock(&self.operations).push(format!("untrust {fingerprint}"));

        Ok(())
    }
}

#[test]
fn setup_no_path_configures_system_integrations_and_waits_for_reconciliation() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());
    seed_setup_manifest(&fixture.paths)?;
    let daemon = DaemonFixture::start(&fixture.paths)?;

    let output = run_pv(&["setup", "--no-path"], fixture.environment.as_ref())?;
    let daemon_requests = daemon.finish()?;
    let prepared_resolver = read_required_file(&fixture.paths.resolver_config())?;
    let system_resolver = read_required_file(&fixture.system_resolver_path)?;
    let prepared_anchor = read_required_file(&fixture.paths.pf_anchor_config())?;
    let system_anchor = read_required_file(&fixture.system_anchor_path)?;
    let system_pf_conf = read_required_file(&fixture.system_pf_conf_path)?;
    let launch_agent = read_required_file(&fixture.launch_agent_path)?;
    let parsed_resolver = ResolverConfig::parse(&prepared_resolver);
    let parsed_launch_agent = LaunchAgentConfig::parse(&launch_agent);

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(parsed_resolver.is_some());
    assert_eq!(system_resolver, prepared_resolver);
    assert_eq!(system_anchor, prepared_anchor);
    assert_eq!(fixture.environment.certificates().len(), 1);
    assert!(output.stdout.contains("PV setup complete"));
    assert!(daemon_requests.iter().any(|request| {
        request.contains(r#""kind":"reconcile""#) && request.contains(r#""scope":"system""#)
    }));

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            PfRedirectConfig::parse_anchor(&prepared_anchor),
            PfConfReference::parse_block(&system_pf_conf),
            parsed_launch_agent,
            fixture.environment.operations(),
            daemon_requests,
        ));
    });

    Ok(())
}

#[test]
fn setup_records_default_resource_desired_tracks_before_reconciliation() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());

    seed_setup_manifest(&fixture.paths)?;
    let daemon = DaemonFixture::start(&fixture.paths)?;

    let output = run_pv(&["setup", "--no-path"], fixture.environment.as_ref())?;
    let daemon_requests = daemon.finish()?;
    let database = Database::open(&fixture.paths)?;
    let tracks = database.managed_resource_tracks()?;
    let observed = tracks
        .iter()
        .map(|track| {
            (
                track.resource_name.as_str(),
                track.track.as_str(),
                track.desired_state,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(observed, expected_setup_tracks());
    assert!(daemon_requests.iter().any(|request| {
        request.contains(r#""kind":"reconcile""#) && request.contains(r#""scope":"system""#)
    }));

    Ok(())
}

#[test]
fn setup_requires_manifest_before_daemon_registration() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());

    let output = run_pv(&["setup", "--no-path"], fixture.environment.as_ref())?;
    let operations = fixture.environment.operations();

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(
        output
            .stderr
            .contains("setup cannot plan default Managed Resources")
    );
    assert!(
        !operations
            .iter()
            .any(|operation| operation.contains("bootstrap"))
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, operations));
    });

    Ok(())
}

#[test]
fn setup_non_interactive_fails_before_privileged_system_changes() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());

    let output = run_pv(
        &["setup", "--no-path", "--non-interactive"],
        fixture.environment.as_ref(),
    )?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.contains("requires macOS authentication"));
    assert!(fixture.environment.operations().is_empty());
    assert!(read_optional_file(&fixture.system_resolver_path)?.is_none());
    assert!(read_optional_file(&fixture.system_anchor_path)?.is_none());
    assert!(read_optional_file(&fixture.system_pf_conf_path)?.is_none());
    assert!(fixture.environment.certificates().is_empty());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, fixture.environment.operations()));
    });

    Ok(())
}

#[test]
fn uninstall_preserves_user_data_by_default() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());
    seed_setup_manifest(&fixture.paths)?;
    let daemon = DaemonFixture::start(&fixture.paths)?;

    let setup = run_pv(&["setup", "--no-path"], fixture.environment.as_ref())?;
    let _daemon_requests = daemon.finish()?;
    seed_uninstall_files(&fixture.paths)?;

    let uninstall = run_pv(&["uninstall"], fixture.environment.as_ref())?;

    assert_eq!(setup.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::SUCCESS);
    assert!(read_optional_file(&fixture.system_resolver_path)?.is_none());
    assert!(read_optional_file(&fixture.system_anchor_path)?.is_none());
    assert!(read_optional_file(&fixture.system_pf_conf_path)?.is_none());
    assert!(read_optional_file(&fixture.launch_agent_path)?.is_none());
    assert!(fixture.environment.certificates().is_empty());
    assert!(path_exists(fixture.paths.root()));
    assert!(path_exists(fixture.paths.db()));
    assert!(read_optional_file(&fixture.paths.logs().join("daemon.log"))?.is_some());
    assert!(read_optional_file(&fixture.paths.ca_certificate())?.is_some());
    assert!(read_optional_file(&fixture.paths.composer().join("cache.txt"))?.is_some());
    assert!(read_optional_file(&fixture.paths.resources().join("mysql/data.txt"))?.is_some());
    assert!(read_optional_file(&fixture.paths.bin().join("pv"))?.is_none());
    assert!(read_optional_file(&fixture.paths.run().join("runtime.json"))?.is_none());
    assert!(read_optional_file(&fixture.paths.config().join("generated.txt"))?.is_none());
    assert!(read_optional_file(&fixture.paths.downloads().join("artifact.tar"))?.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((uninstall, fixture.environment.operations()));
    });

    Ok(())
}

#[test]
fn uninstall_removes_stale_ca_trust_when_local_ca_files_are_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());
    seed_setup_manifest(&fixture.paths)?;
    let daemon = DaemonFixture::start(&fixture.paths)?;

    let setup = run_pv(&["setup", "--no-path"], fixture.environment.as_ref())?;
    let _daemon_requests = daemon.finish()?;
    let trusted_before = fixture.environment.certificates();
    let fingerprint = trusted_before
        .first()
        .ok_or_else(|| anyhow::anyhow!("setup did not trust a local CA"))?
        .metadata
        .fingerprint
        .clone();

    delete_optional_file(&fixture.paths.ca_certificate())?;
    delete_optional_file(&fixture.paths.ca_private_key())?;

    let uninstall = run_pv(&["uninstall"], fixture.environment.as_ref())?;

    assert_eq!(setup.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::SUCCESS);
    assert!(fixture.environment.certificates().is_empty());
    assert!(
        fixture
            .environment
            .operations()
            .iter()
            .any(|operation| operation == &format!("untrust {fingerprint}"))
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((uninstall, fixture.environment.operations()));
    });

    Ok(())
}

#[test]
fn uninstall_prune_requires_confirmation_without_force() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());

    state::fs::ensure_layout(&fixture.paths)?;
    write_file(&fixture.paths.logs().join("daemon.log"), "keep me\n")?;

    let output = run_pv(&["uninstall", "--prune"], fixture.environment.as_ref())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(path_exists(fixture.paths.root()));
    assert!(read_optional_file(&fixture.paths.logs().join("daemon.log"))?.is_some());
    assert!(fixture.environment.operations().is_empty());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn uninstall_prune_force_removes_all_pv_state() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new(tempdir.path());
    seed_setup_manifest(&fixture.paths)?;
    let daemon = DaemonFixture::start(&fixture.paths)?;

    let setup = run_pv(&["setup", "--no-path"], fixture.environment.as_ref())?;
    let _daemon_requests = daemon.finish()?;
    seed_uninstall_files(&fixture.paths)?;

    let uninstall = run_pv(
        &["uninstall", "--prune", "--force"],
        fixture.environment.as_ref(),
    )?;

    assert_eq!(setup.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::SUCCESS);
    assert!(!path_exists(fixture.paths.root()));
    assert!(read_optional_file(&fixture.system_resolver_path)?.is_none());
    assert!(read_optional_file(&fixture.system_anchor_path)?.is_none());
    assert!(read_optional_file(&fixture.system_pf_conf_path)?.is_none());
    assert!(read_optional_file(&fixture.launch_agent_path)?.is_none());
    assert!(fixture.environment.certificates().is_empty());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((uninstall, fixture.environment.operations()));
    });

    Ok(())
}

#[test]
fn setup_yes_creates_and_uninstall_removes_shell_profile_block() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let fixture = Fixture::new_with_shell(tempdir.path(), "/bin/zsh");
    seed_setup_manifest(&fixture.paths)?;
    let daemon = DaemonFixture::start(&fixture.paths)?;
    let profile_path = fixture.paths.home().join(".zprofile");

    write_file(&profile_path, "export EXISTING=1\n\n")?;

    let setup = run_pv(&["setup", "--yes"], fixture.environment.as_ref())?;
    let _daemon_requests = daemon.finish()?;
    let profile_after_setup = read_required_file(&profile_path)?;

    let second_daemon = DaemonFixture::start(&fixture.paths)?;
    let second_setup = run_pv(
        &["setup", "--yes", "--non-interactive"],
        fixture.environment.as_ref(),
    )?;
    let _second_daemon_requests = second_daemon.finish()?;
    let profile_after_second_setup = read_required_file(&profile_path)?;

    let uninstall = run_pv(&["uninstall"], fixture.environment.as_ref())?;
    let profile_after_uninstall = read_required_file(&profile_path)?;

    assert_eq!(setup.exit_code, ExitCode::SUCCESS);
    assert_eq!(second_setup.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::SUCCESS);
    assert!(profile_after_setup.contains("# >>> PV ENV"));
    assert!(profile_after_setup.contains("env --shell zsh"));
    assert_eq!(profile_after_second_setup, profile_after_setup);
    assert_eq!(profile_after_uninstall, "export EXISTING=1\n\n");
    assert!(!profile_after_uninstall.contains("# >>> PV ENV"));

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            setup,
            profile_after_setup,
            second_setup,
            profile_after_second_setup,
            uninstall,
            profile_after_uninstall,
            fixture.environment.operations(),
        ));
    });

    Ok(())
}

#[derive(Debug)]
struct Fixture {
    paths: PvPaths,
    launch_agent_path: camino::Utf8PathBuf,
    system_resolver_path: camino::Utf8PathBuf,
    system_anchor_path: camino::Utf8PathBuf,
    system_pf_conf_path: camino::Utf8PathBuf,
    environment: Arc<TestEnvironment>,
}

impl Fixture {
    fn new(root: &Utf8Path) -> Self {
        Self::new_inner(root, None)
    }

    fn new_with_shell(root: &Utf8Path, shell: &str) -> Self {
        Self::new_inner(root, Some(OsString::from(shell)))
    }

    fn new_inner(root: &Utf8Path, shell: Option<OsString>) -> Self {
        let home = root.join("home");
        let current_dir = root.join("work");
        let current_exe = root.join("bin/pv");
        let paths = PvPaths::for_home(&home);
        let launch_agent_path = root.join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
        let system_resolver_path = root.join("etc/resolver/test");
        let system_anchor_path = root.join("etc/pf.anchors/com.prvious.pv");
        let system_pf_conf_path = root.join("etc/pf.conf");
        let environment = Arc::new(TestEnvironment::new(
            TestEnvironmentPaths {
                home: &home,
                current_dir: &current_dir,
                current_exe: &current_exe,
                launch_agent_path: &launch_agent_path,
                resolver_path: &system_resolver_path,
                pf_anchor_path: &system_anchor_path,
                pf_conf_path: &system_pf_conf_path,
            },
            shell,
        ));

        Self {
            paths,
            launch_agent_path,
            system_resolver_path,
            system_anchor_path,
            system_pf_conf_path,
            environment,
        }
    }
}

#[derive(Debug)]
struct DaemonFixture {
    requests: Arc<Mutex<Vec<String>>>,
    thread: thread::JoinHandle<anyhow::Result<()>>,
}

impl DaemonFixture {
    fn start(paths: &PvPaths) -> anyhow::Result<Self> {
        state::fs::ensure_layout(paths)?;
        delete_optional_file(&paths.daemon_socket())?;
        let listener = UnixListener::bind(paths.daemon_socket().as_std_path())?;

        listener.set_nonblocking(true)?;

        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let thread = spawn_daemon_fixture_thread(move || {
            let mut job_requests = 0;

            for _request_index in 0..3 {
                let (mut stream, _address) = accept_with_timeout(&listener)?;
                let mut request = String::new();
                let mut reader = BufReader::new(stream.try_clone()?);

                reader.read_line(&mut request)?;
                lock(&thread_requests).push(request.trim().to_string());

                if request.contains(r#""command":"health""#) {
                    write_daemon_line(
                        &mut stream,
                        json!({
                            "type": "response",
                            "protocol_version": daemon::PROTOCOL_VERSION,
                            "status": "ok",
                            "message": "daemon healthy",
                        }),
                    )?;
                    continue;
                }

                job_requests += 1;
                if job_requests == 1 {
                    write_daemon_line(
                        &mut stream,
                        json!({
                            "type": "response",
                            "protocol_version": daemon::PROTOCOL_VERSION,
                            "status": "accepted",
                            "message": "job accepted",
                            "job_id": "job_enable_1",
                        }),
                    )?;
                    continue;
                }

                write_daemon_line(
                    &mut stream,
                    json!({
                        "type": "response",
                        "protocol_version": daemon::PROTOCOL_VERSION,
                        "status": "accepted",
                        "message": "job accepted",
                        "job_id": "job_setup_1",
                    }),
                )?;
                write_daemon_line(
                    &mut stream,
                    json!({
                        "type": "job_started",
                        "job_id": "job_setup_1",
                        "kind": "reconcile",
                        "scope": "system",
                    }),
                )?;
                write_daemon_line(
                    &mut stream,
                    json!({
                        "type": "progress",
                        "job_id": "job_setup_1",
                        "message": "stub job completed without reconciliation work",
                    }),
                )?;
                write_daemon_line(
                    &mut stream,
                    json!({
                        "type": "job_completed",
                        "job_id": "job_setup_1",
                        "summary": "stub job completed",
                    }),
                )?;
            }

            Ok(())
        });

        Ok(Self { requests, thread })
    }

    fn finish(self) -> anyhow::Result<Vec<String>> {
        self.thread
            .join()
            .map_err(|_error| anyhow::anyhow!("daemon fixture thread panicked"))??;

        Ok(lock(&self.requests).clone())
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI setup tests run a synchronous fixture daemon on a short-lived thread"
)]
fn spawn_daemon_fixture_thread(
    operation: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
) -> thread::JoinHandle<anyhow::Result<()>> {
    thread::spawn(operation)
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

fn seed_uninstall_files(paths: &PvPaths) -> anyhow::Result<()> {
    write_file(&paths.logs().join("daemon.log"), "log\n")?;
    write_file(&paths.composer().join("cache.txt"), "composer\n")?;
    write_file(&paths.resources().join("mysql/data.txt"), "resource data\n")?;
    write_file(&paths.bin().join("pv"), "binary\n")?;
    write_file(&paths.run().join("runtime.json"), "{}\n")?;
    write_file(&paths.config().join("generated.txt"), "generated\n")?;
    write_file(&paths.downloads().join("artifact.tar"), "download\n")?;

    Ok(())
}

fn seed_setup_manifest(paths: &PvPaths) -> anyhow::Result<()> {
    state::fs::ensure_layout(paths)?;
    state::fs::write_sensitive_file(
        &paths.downloads().join("manifest.json"),
        &serde_json::to_string(&json!({
            "schema_version": 1,
            "minimum_pv_version": "0.1.0",
            "resources": [
                setup_manifest_resource("frankenphp", "1.5"),
                setup_manifest_resource("php", "8.4"),
                setup_manifest_resource("mysql", "8.4"),
                setup_manifest_resource("postgres", "17"),
                setup_manifest_resource("redis", "7.2"),
                setup_manifest_resource("mailpit", "1"),
                setup_manifest_resource("rustfs", "1"),
                setup_manifest_resource("composer", "2"),
            ],
        }))?,
    )?;

    Ok(())
}

fn setup_manifest_resource(name: &str, track: &str) -> serde_json::Value {
    json!({
        "name": name,
        "default_track": track,
        "tracks": [
            {
                "name": track,
                "artifacts": [
                    {
                        "artifact_version": format!("{track}.0-pv1"),
                        "upstream_version": format!("{track}.0"),
                        "pv_build_revision": "pv1",
                        "platform": "darwin-arm64",
                        "url": format!(
                            "https://artifacts.example.test/{name}-{track}.0-pv1-darwin-arm64.tar.gz"
                        ),
                        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "size": 12345,
                        "published_at": "2026-05-26T14:30:00Z"
                    }
                ]
            }
        ]
    })
}

fn expected_setup_tracks() -> Vec<(&'static str, &'static str, ManagedResourceDesiredState)> {
    vec![
        ("composer", "2", ManagedResourceDesiredState::Installed),
        ("frankenphp", "1.5", ManagedResourceDesiredState::Installed),
        ("mailpit", "1", ManagedResourceDesiredState::Installed),
        ("mysql", "8.4", ManagedResourceDesiredState::Installed),
        ("php", "8.4", ManagedResourceDesiredState::Installed),
        ("postgres", "17", ManagedResourceDesiredState::Installed),
        ("redis", "7.2", ManagedResourceDesiredState::Installed),
        ("rustfs", "1", ManagedResourceDesiredState::Installed),
    ]
}

fn accept_with_timeout(
    listener: &UnixListener,
) -> anyhow::Result<(UnixStream, std::os::unix::net::SocketAddr)> {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        match listener.accept() {
            Ok(accepted) => return Ok(accepted),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                anyhow::bail!("daemon fixture did not receive a request")
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn write_daemon_line(stream: &mut UnixStream, value: serde_json::Value) -> io::Result<()> {
    writeln!(stream, "{value}")
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"[a-f0-9]{64}", "<fingerprint>");
    settings.add_filter(
        r"DNS resolver port: [0-9]+",
        "DNS resolver port: <dns-port>",
    );
    settings.add_filter(r"on port [0-9]+", "on port <dns-port>");
    settings.add_filter(
        r"\.zprofile\.[0-9]+\.pv\.bak",
        ".zprofile.<timestamp>.pv.bak",
    );
    settings.bind(assertion);
}
