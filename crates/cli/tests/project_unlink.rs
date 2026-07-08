use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use state::{Database, LinkProjectInput, ProjectRecord, PvPaths};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: current_dir.as_std_path().to_path_buf(),
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
fn unlink_removes_project_tls_directory() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project_path = tempdir.path().join("acme");
    let paths = PvPaths::for_home(home.clone());
    let project = seed_project(&paths, &project_path)?;
    let project_tls_dir = paths.project_tls_dir(&project.id);
    state::fs::write_sensitive_file(
        &paths.project_tls_certificate(&project.id),
        "test certificate\n",
    )?;
    state::fs::write_sensitive_file(&paths.project_tls_private_key(&project.id), "test key\n")?;
    let environment = TestEnvironment::new(&home, &project_path);

    let output = run_pv(&["unlink", "acme.test"], &environment)?;
    let database = Database::open(&paths)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(database.project_by_id(&project.id)?.is_none());
    assert!(!path_exists(&project_tls_dir));
    assert!(path_exists(&project.path));

    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((&output.exit_code, &output.stdout, &output.stderr));
    });

    Ok(())
}

fn seed_project(paths: &PvPaths, project_path: &Utf8Path) -> anyhow::Result<ProjectRecord> {
    state::fs::write_sensitive_file(&project_path.join("pv.yml"), "php: 8.4\n")?;

    let mut database = Database::open(paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_path.to_path_buf(),
            original_path: project_path.to_path_buf(),
            primary_hostname: "acme.test".to_string(),
            config_path: project_path.join("pv.yml"),
            desired_php_track: Some("8.4".to_string()),
            additional_hostnames: Vec::new(),
        })?
        .project;

    Ok(project)
}

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

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}
