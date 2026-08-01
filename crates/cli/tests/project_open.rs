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
use state::{Database, ProjectMode, PvPaths};

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
fn link_rejects_invalid_mode_change_without_replacing_served_state() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home, &project);

    let initial_link = run_pv(&["link"], &environment)?;
    write_file(
        &project.join("pv.yml"),
        r#"serve: false
postgres:
  allocations:
    analytics:
      env:
        DATABASE_URL: "postgres://${database}"
    app:
      env:
        DATABASE_URL: "postgres://${database}"
"#,
    )?;
    let invalid_link = run_pv(&["link"], &environment)?;
    let database = Database::open(&paths)?;
    let linked = database
        .project_by_path(&canonical_path(&project)?)?
        .ok_or_else(|| anyhow::anyhow!("expected linked Project"))?;

    assert_eq!(initial_link.exit_code, ExitCode::SUCCESS);
    assert_eq!(invalid_link.exit_code, ExitCode::FAILURE);
    assert_eq!(linked.mode, ProjectMode::Served);
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
    settings.bind(|| {
        assert_debug_snapshot!((initial_link, invalid_link, linked));
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
fn open_rejects_resource_only_target_and_excludes_it_from_picker() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let resource_only = tempdir.path().join("resources");
    let served = tempdir.path().join("web");
    let outside = tempdir.path().join("outside");
    create_dir(&resource_only)?;
    create_dir(&served)?;
    create_dir(&outside)?;
    write_file(&resource_only.join("pv.yml"), "serve: false\n")?;
    let environment = TestEnvironment::new(&home, &resource_only).interactive(["1\n"]);

    let link_resource_only = run_pv(&["link"], &environment)?;
    let explicit_open = run_pv(&["open", "resources"], &environment)?;
    let current_open = run_pv(&["open"], &environment)?;
    environment.set_current_dir(&served);
    let link_served = run_pv(&["link"], &environment)?;
    environment.set_current_dir(&outside);
    let picker_open = run_pv(&["open"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link_resource_only.exit_code, ExitCode::SUCCESS);
    assert_eq!(explicit_open.exit_code, ExitCode::FAILURE);
    assert_eq!(current_open.exit_code, ExitCode::FAILURE);
    assert_eq!(link_served.exit_code, ExitCode::SUCCESS);
    assert_eq!(picker_open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, ["https://web.test"]);
    assert!(!picker_open.stdout.contains("resources"));
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((
            link_resource_only,
            explicit_open,
            current_open,
            link_served,
            picker_open,
            opened_urls
        ));
    });

    Ok(())
}

#[test]
fn open_project_picker_sorts_projects_by_primary_hostname() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let first_by_slug = tempdir.path().join("alpha-project");
    let second_by_slug = tempdir.path().join("zeta-project");
    let outside = tempdir.path().join("outside");
    create_dir(&first_by_slug)?;
    create_dir(&second_by_slug)?;
    create_dir(&outside)?;
    let environment = TestEnvironment::new(&home, &first_by_slug).interactive(["2\n"]);

    let link_zeta = run_pv(&["link", "--hostname", "zeta"], &environment)?;
    environment.set_current_dir(&second_by_slug);
    let link_alpha = run_pv(&["link", "--hostname", "alpha"], &environment)?;
    environment.set_current_dir(&outside);
    let open = run_pv(&["open"], &environment)?;
    let opened_urls = environment.opened_urls();

    assert_eq!(link_zeta.exit_code, ExitCode::SUCCESS);
    assert_eq!(link_alpha.exit_code, ExitCode::SUCCESS);
    assert_eq!(open.exit_code, ExitCode::SUCCESS);
    assert_eq!(opened_urls, vec!["https://zeta.test"]);
    assert!(link_zeta.stderr.is_empty());
    assert!(link_alpha.stderr.is_empty());
    assert!(open.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link_zeta, link_alpha, open, opened_urls));
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
