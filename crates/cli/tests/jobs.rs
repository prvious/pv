use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use state::{Database, PvPaths};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: home.as_std_path().to_path_buf(),
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
}

#[test]
fn jobs_lists_empty_history() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["jobs"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(!state::fs::path_exists(paths.root()));
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn jobs_lists_recent_history() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_jobs(&paths)?;

    let output = run_pv(&["jobs"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_jobs_snapshot("jobs_lists_recent_history", output);

    Ok(())
}

#[test]
fn jobs_shows_failure_summary() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    let mut database = Database::open(&paths)?;
    let job = database.start_job("reconcile", "project:acme")?;
    database.fail_job(&job.id, "Project config is invalid")?;

    let output = run_pv(&["jobs"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_jobs_snapshot("jobs_shows_failure_summary", output);

    Ok(())
}

#[test]
fn jobs_json_lists_recent_history() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_jobs(&paths)?;

    let output = run_pv(&["jobs", "--json"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_jobs_snapshot("jobs_json_lists_recent_history", output);

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

fn seed_jobs(paths: &PvPaths) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;
    let setup = database.start_job("setup", "system")?;
    database.complete_job(&setup.id, "Installed default resources")?;
    let project = database.start_job("reconcile", "project:acme")?;
    database.fail_job(&project.id, "Gateway failed to start")?;
    database.start_job("install", "resource:redis:7")?;

    Ok(())
}

fn assert_jobs_snapshot(name: &'static str, snapshot: impl std::fmt::Debug) {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
