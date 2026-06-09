use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use state::{Database, PortRequest, PvPaths, RuntimeObservedStatus, RuntimeSubject};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    opened_urls: RefCell<Vec<String>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: home.as_std_path().to_path_buf(),
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
fn rustfs_open_reports_not_running_without_observed_runtime() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["rustfs:open"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(environment.opened_urls().is_empty());
    assert!(output.stderr.is_empty());
    assert_open_snapshot(
        "rustfs_open_reports_not_running_without_observed_runtime",
        output,
    );

    Ok(())
}

#[test]
fn rustfs_open_does_not_open_stale_observed_runtime() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    let stale_console_port = unused_loopback_port()?;
    seed_running_rustfs_console(&paths, stale_console_port)?;

    let output = run_pv(&["rustfs:open"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(environment.opened_urls().is_empty());
    assert!(output.stderr.is_empty());
    assert_open_snapshot("rustfs_open_does_not_open_stale_observed_runtime", output);

    Ok(())
}

#[test]
fn rustfs_open_aliases_open_live_console_port() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    let console_guard = TcpListener::bind(("127.0.0.1", 0))?;
    let console_port = console_guard.local_addr()?.port();
    let console_url = format!("http://127.0.0.1:{console_port}");
    seed_running_rustfs_console(&paths, console_port)?;

    let rustfs_open = run_pv(&["rustfs:open"], &environment)?;
    let s3_open = run_pv(&["s3:open"], &environment)?;

    assert_eq!(rustfs_open.exit_code, ExitCode::SUCCESS);
    assert_eq!(s3_open.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        environment.opened_urls(),
        vec![console_url.clone(), console_url]
    );
    assert!(rustfs_open.stderr.is_empty());
    assert!(s3_open.stderr.is_empty());
    assert_open_snapshot(
        "rustfs_open_aliases_open_live_console_port",
        (rustfs_open, s3_open, environment.opened_urls()),
    );

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

fn assert_open_snapshot(name: &'static str, snapshot: impl std::fmt::Debug) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(
        r"http://127\.0\.0\.1:\d+",
        "http://127.0.0.1:<console_port>",
    );
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
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

fn unused_loopback_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);

    Ok(port)
}

fn seed_running_rustfs_console(paths: &PvPaths, console_port: u16) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;

    database.assign_port(
        PortRequest::resource_port("rustfs", "1.0", "api", 19_090, 19_090, 19_090),
        |_| true,
    )?;
    database.assign_port(
        PortRequest::resource_port(
            "rustfs",
            "1.0",
            "console",
            console_port,
            console_port,
            console_port,
        ),
        |_| true,
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "rustfs".to_string(),
            track: "1.0".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;

    Ok(())
}
