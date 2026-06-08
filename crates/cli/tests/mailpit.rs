use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use state::{
    Database, LinkProjectInput, ProjectManagedResourceInput, PvPaths, RuntimeObservedStatus,
    RuntimeSubject,
};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    opened_urls: RefCell<Vec<String>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: current_dir.as_std_path().to_path_buf(),
            opened_urls: RefCell::new(Vec::new()),
        }
    }

    fn opened_urls(&self) -> Vec<String> {
        self.opened_urls.borrow().clone()
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

    fn open_url(&self, url: &str) -> io::Result<()> {
        self.opened_urls.borrow_mut().push(url.to_string());

        Ok(())
    }
}

#[test]
fn mailpit_open_reports_exact_message_when_not_running() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let environment = TestEnvironment::new(&home, &current_dir);

    let mailpit_output = run_pv(&["mailpit:open"], &environment)?;
    let mail_output = run_pv(&["mail:open"], &environment)?;

    assert_eq!(mailpit_output.exit_code, ExitCode::SUCCESS);
    assert_eq!(mail_output.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        mailpit_output.stdout,
        "Mailpit is not running for any linked Project\n"
    );
    assert_eq!(
        mail_output.stdout,
        "Mailpit is not running for any linked Project\n"
    );
    assert!(mailpit_output.stderr.is_empty());
    assert!(mail_output.stderr.is_empty());
    assert!(environment.opened_urls().is_empty());

    Ok(())
}

#[test]
fn mailpit_open_opens_running_dashboard_without_mutating_state() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "mailpit:\n  version: \"1.0\"\n")?;
    let paths = PvPaths::for_home(home.clone());
    let project = record_linked_mailpit_project(&paths, &project)?;
    let environment = TestEnvironment::new(&home, &project.path);
    let before = observed_mailpit_state(&paths)?;

    let output = run_pv(&["mailpit:open"], &environment)?;
    let after = observed_mailpit_state(&paths)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        environment.opened_urls(),
        vec!["http://127.0.0.1:8025".to_string()]
    );
    assert_eq!(after, before);

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

fn record_linked_mailpit_project(
    paths: &PvPaths,
    project_path: &Utf8Path,
) -> anyhow::Result<state::ProjectRecord> {
    let mut database = Database::open(paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_path.to_path_buf(),
            original_path: project_path.to_path_buf(),
            primary_hostname: "acme.test".to_string(),
            config_path: project_path.join("pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?
        .project;

    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mailpit".to_string(),
            track: "1.0".to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "mailpit",
        "1.0",
        &BTreeMap::from([
            ("smtp_host".to_string(), "127.0.0.1".to_string()),
            ("smtp_port".to_string(), "1025".to_string()),
            (
                "dashboard_url".to_string(),
                "http://127.0.0.1:8025".to_string(),
            ),
        ]),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "mailpit".to_string(),
            track: "1.0".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;

    Ok(project)
}

fn observed_mailpit_state(
    paths: &PvPaths,
) -> anyhow::Result<(
    state::ManagedResourceTrackRecord,
    Vec<state::RuntimeObservedStateRecord>,
)> {
    let database = Database::open(paths)?;

    Ok((
        database.managed_resource_track("mailpit", "1.0")?,
        database.runtime_observed_states()?,
    ))
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI Mailpit tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI Mailpit tests write fixture config files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}
