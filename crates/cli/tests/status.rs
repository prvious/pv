use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use platform::{KeychainCertificate, PfConfReference, PfRedirectConfig, ResolverConfig};
use platform::{KeychainTrustResult, LaunchAgentConfig};
use state::{
    Database, LinkProjectInput, ManagedResourceTrackInstallInput, ProjectEnvObservedStatus,
    PvPaths, RuntimeObservedStatus, RuntimeSubject,
};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    launch_agent_path: PathBuf,
    resolver_path: PathBuf,
    pf_anchor_path: PathBuf,
    pf_conf_path: PathBuf,
    trusted_certificates: Vec<KeychainCertificate>,
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
            trusted_certificates: Vec::new(),
        }
    }

    fn with_trusted_certificate(mut self, certificate: KeychainCertificate) -> Self {
        self.trusted_certificates.push(certificate);
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
        Ok(None)
    }

    fn trusted_ca_certificates(&self) -> Result<Vec<KeychainCertificate>, platform::PlatformError> {
        Ok(self.trusted_certificates.clone())
    }
}

#[test]
fn status_reports_disabled_daemon_without_setup() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(!state::fs::path_exists(paths.root()));
    assert_status_snapshot(
        "status_reports_disabled_daemon_without_setup",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_reports_current_launch_agent_with_stale_socket_as_down() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_current_launch_agent(&paths, &environment)?;
    write_file(&paths.daemon_socket(), "")?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_status_snapshot(
        "status_reports_current_launch_agent_with_stale_socket_as_down",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_reports_failed_jobs_as_failure() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    let mut database = Database::open(&paths)?;
    let job = database.start_job("reconcile", "project:acme")?;
    database.fail_job(&job.id, "Gateway failed to start")?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_status_snapshot(
        "status_reports_failed_jobs_as_failure",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_reports_dns_and_ports_repair_required_as_failure() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let trusted_certificate = seed_current_local_ca(&paths)?;
    let environment = TestEnvironment::new(&home).with_trusted_certificate(trusted_certificate);
    Database::open(&paths)?;
    write_file(
        &paths.resolver_config(),
        &ResolverConfig::new(35353).render(),
    )?;
    write_file(
        &paths.pf_anchor_config(),
        &PfRedirectConfig::new(48080, 48443).render_anchor(),
    )?;
    write_file(&paths.pf_conf_reference_config(), &PfConfReference.render())?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_status_snapshot(
        "status_reports_dns_and_ports_repair_required_as_failure",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_reports_project_env_failures_as_failure() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project_path = tempdir.path().join("project");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path.clone(),
            primary_hostname: "app.test".to_string(),
            config_path: project_path.join("pv.toml"),
            desired_php_track: Some("8.4".to_string()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Failed,
        Some("missing required env placeholder"),
        &[],
    )?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_status_snapshot(
        "status_reports_project_env_failures_as_failure",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_reports_pending_project_env_as_success() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project_path = tempdir.path().join("project");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    let mut database = Database::open(&paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_path.clone(),
            original_path: project_path.clone(),
            primary_hostname: "app.test".to_string(),
            config_path: project_path.join("pv.toml"),
            desired_php_track: Some("8.4".to_string()),
            additional_hostnames: Vec::new(),
        })?
        .project;
    database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Pending,
        Some("waiting for reconciliation"),
        &[],
    )?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_status_snapshot(
        "status_reports_pending_project_env_as_success",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_reports_runtime_and_resource_states() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_resource_state(&paths)?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_status_snapshot(
        "status_reports_runtime_and_resource_states",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn status_json_redacts_secret_context() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_resource_state(&paths)?;

    let output = run_pv(&["status", "--json"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains("root-secret"));
    assert!(!output.stdout.contains("postgres-secret"));
    assert!(!output.stdout.contains("redis-secret"));
    assert!(!output.stdout.contains("rustfs-secret"));
    assert_status_snapshot("status_json_redacts_secret_context", tempdir.path(), output);

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

fn seed_resource_state(paths: &PvPaths) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;

    for (resource_name, track, installed_version) in [
        ("php", "8.4", "8.4.8-pv1"),
        ("mysql", "8.0", "8.0.36-pv1"),
        ("postgres", "16", "16.4-pv1"),
        ("redis", "7", "7.2.5-pv1"),
        ("mailpit", "1", "1.20.0-pv1"),
        ("rustfs", "1", "1.0.0-pv1"),
    ] {
        let artifact_path = paths
            .resources()
            .join(resource_name)
            .join(track)
            .join("artifact");
        database.record_managed_resource_tracks_desired_and_installed(&[
            ManagedResourceTrackInstallInput {
                resource_name,
                track,
                installed_version,
                current_artifact_path: &artifact_path,
            },
        ])?;
    }

    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("password".to_string(), "root-secret".to_string()),
            ("port".to_string(), "3306".to_string()),
            ("username".to_string(), "root".to_string()),
        ]),
    )?;
    database.record_managed_resource_track_env_context(
        "postgres",
        "16",
        &BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("password".to_string(), "postgres-secret".to_string()),
            ("port".to_string(), "5432".to_string()),
            ("username".to_string(), "postgres".to_string()),
        ]),
    )?;
    database.record_managed_resource_track_env_context(
        "redis",
        "7",
        &BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("password".to_string(), "redis-secret".to_string()),
            ("port".to_string(), "6379".to_string()),
        ]),
    )?;
    database.record_managed_resource_track_env_context(
        "rustfs",
        "1",
        &BTreeMap::from([
            ("access_key".to_string(), "rustfs-access".to_string()),
            ("endpoint".to_string(), "http://127.0.0.1:9000".to_string()),
            ("secret_key".to_string(), "rustfs-secret".to_string()),
        ]),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::PhpWorker {
            php_track: "8.4".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("PHP worker is ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "mysql".to_string(),
            track: "8.0".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "postgres".to_string(),
            track: "16".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "redis".to_string(),
            track: "7".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "mailpit".to_string(),
            track: "1".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "rustfs".to_string(),
            track: "1".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;
    let job = database.start_job("reconcile", "resource:redis:7")?;
    database.fail_job(&job.id, "Redis failed readiness")?;

    Ok(())
}

fn seed_current_local_ca(paths: &PvPaths) -> anyhow::Result<KeychainCertificate> {
    let ca = platform::generate_local_ca()?;
    write_file(&paths.ca_certificate(), &ca.certificate_pem)?;
    write_file(&paths.ca_private_key(), &ca.private_key_pem)?;

    Ok(KeychainCertificate {
        metadata: ca.metadata,
        trust: KeychainTrustResult::TrustRoot,
    })
}

fn seed_current_launch_agent(paths: &PvPaths, environment: &TestEnvironment) -> anyhow::Result<()> {
    let launch_agent = LaunchAgentConfig::new(
        "/bin/pv",
        paths.launchd_stdout_log(),
        paths.launchd_stderr_log(),
    );
    let path = Utf8Path::from_path(&environment.launch_agent_path)
        .ok_or_else(|| anyhow::anyhow!("launch agent path is not UTF-8"))?;
    platform::write_launch_agent_file(path, &launch_agent)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI status tests create fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;

    Ok(())
}

fn assert_status_snapshot(name: &'static str, tempdir: &Utf8Path, snapshot: impl std::fmt::Debug) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
