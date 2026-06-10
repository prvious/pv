use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use platform::LaunchAgentConfig;
use state::{
    Database, ManagedResourceTrackInstallInput, PvPaths, RuntimeObservedStatus, RuntimeSubject,
};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    launch_agent_path: PathBuf,
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
        }
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
fn status_reports_runtime_and_resource_states() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_resource_state(&paths)?;

    let output = run_pv(&["status"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
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

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains("root-secret"));
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
    let artifact_path = paths.resources().join("mysql/8.0/artifact");
    let mut database = Database::open(paths)?;
    database.record_managed_resource_tracks_desired_and_installed(&[
        ManagedResourceTrackInstallInput {
            resource_name: "mysql",
            track: "8.0",
            installed_version: "1.0.0",
            current_artifact_path: &artifact_path,
        },
    ])?;
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
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "mysql".to_string(),
            track: "8.0".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;

    Ok(())
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
