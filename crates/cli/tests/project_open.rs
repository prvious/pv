use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use state::{Database, PvPaths};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    input_lines: RefCell<VecDeque<String>>,
    opened_urls: RefCell<Vec<String>>,
    stdin_terminal: bool,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            input_lines: RefCell::new(VecDeque::new()),
            opened_urls: RefCell::new(Vec::new()),
            stdin_terminal: false,
        }
    }

    fn interactive(mut self, input_lines: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.stdin_terminal = true;
        self.input_lines = RefCell::new(input_lines.into_iter().map(Into::into).collect());

        self
    }

    fn set_current_dir(&self, current_dir: &Utf8Path) {
        *self.current_dir.borrow_mut() = current_dir.as_std_path().to_path_buf();
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
        Ok(self.current_dir.borrow().clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        self.stdin_terminal
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(self
            .input_lines
            .borrow_mut()
            .pop_front()
            .unwrap_or_default())
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        self.opened_urls.borrow_mut().push(url.to_string());

        Ok(())
    }
}

#[test]
fn open_primary_hostname_argument_normalizes_and_opens_project() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    let outside = tempdir.path().join("outside");
    create_dir(&project)?;
    create_dir(&outside)?;
    let environment = TestEnvironment::new(&home, &project);

    let link = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&outside);
    let open = run_pv(&["open", "acme"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, vec!["https://acme.test"]);
    assert!(link.stderr.is_empty());
    assert!(open.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, open, opened_urls));
    });

    Ok(())
}

#[test]
fn link_rejects_update_lock_without_recording_project() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let paths = PvPaths::for_home(home.clone());
    state::fs::ensure_layout(&paths)?;
    let _update_lock = state::UpdateLock::acquire(&paths)?;
    let environment = TestEnvironment::new(&home, &project);

    let link = run_pv(&["link"], &environment)?;
    let database = Database::open(&paths)?;
    let recorded_project = database.project_by_path(&canonical_path(&project)?)?;

    assert_eq!(link.exit_code, ExitCode::FAILURE);
    assert!(link.stdout.is_empty());
    assert!(recorded_project.is_none());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(link);
    });

    Ok(())
}

#[test]
fn open_additional_hostname_argument_opens_exact_hostname() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    let outside = tempdir.path().join("outside");
    create_dir(&project)?;
    create_dir(&outside)?;
    write_file(&project.join("pv.yml"), "hostnames:\n  - api.acme.test\n")?;
    let environment = TestEnvironment::new(&home, &project);

    let link = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&outside);
    let open = run_pv(&["open", "api.acme.test"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, vec!["https://api.acme.test"]);
    assert!(link.stderr.is_empty());
    assert!(open.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, open, opened_urls));
    });

    Ok(())
}

#[test]
fn open_without_hostname_uses_current_project_primary_hostname() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    let nested = project.join("nested");
    create_dir(&nested)?;
    write_file(&project.join("pv.yml"), "hostnames:\n  - api.acme.test\n")?;
    let environment = TestEnvironment::new(&home, &project);

    let link = run_pv(&["link"], &environment)?;
    let canonical_nested = canonical_path(&nested)?;
    environment.set_current_dir(&canonical_nested);
    let open = run_pv(&["open"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, vec!["https://acme.test"]);
    assert!(link.stderr.is_empty());
    assert!(open.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, open, opened_urls));
    });

    Ok(())
}

#[test]
fn open_uses_project_picker_when_outside_a_linked_project() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    let outside = tempdir.path().join("outside");
    create_dir(&project)?;
    create_dir(&outside)?;
    let environment = TestEnvironment::new(&home, &project).interactive(["1\n"]);

    let link = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&outside);
    let open = run_pv(&["open"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, vec!["https://acme.test"]);
    assert!(link.stderr.is_empty());
    assert!(open.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, open, opened_urls));
    });

    Ok(())
}

#[test]
fn open_project_picker_sorts_projects_by_primary_hostname() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let acme = tempdir.path().join("acme");
    let zeta = tempdir.path().join("zeta");
    let outside = tempdir.path().join("outside");
    create_dir(&acme)?;
    create_dir(&zeta)?;
    create_dir(&outside)?;
    let environment = TestEnvironment::new(&home, &zeta).interactive(["2\n"]);

    let link_zeta = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&acme);
    let link_acme = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&outside);
    let open = run_pv(&["open"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link_zeta.exit_code, ExitCode::SUCCESS);
    assert_eq!(link_acme.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, vec!["https://zeta.test"]);
    assert!(link_zeta.stderr.is_empty());
    assert!(link_acme.stderr.is_empty());
    assert!(open.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link_zeta, link_acme, open, opened_urls));
    });

    Ok(())
}

#[test]
fn open_without_current_project_fails_when_non_interactive() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    let outside = tempdir.path().join("outside");
    create_dir(&project)?;
    create_dir(&outside)?;
    let environment = TestEnvironment::new(&home, &project);

    let link = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&outside);
    let open = run_pv(&["open"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::FAILURE);
    assert!(opened_urls.is_empty());
    assert!(link.stderr.is_empty());
    assert!(open.stdout.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, open, opened_urls));
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

#[expect(
    clippy::disallowed_methods,
    reason = "CLI project open tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI project open tests write fixture config files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI project open tests mirror std::env::current_dir canonical paths"
)]
fn canonical_path(path: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    let path = std::fs::canonicalize(path)?;
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| anyhow::anyhow!("non-UTF-8 fixture path `{}`", path.display()))
}
