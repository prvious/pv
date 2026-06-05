use std::cell::RefCell;
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
use platform::{LAUNCH_AGENT_LABEL, LaunchAgentConfig};
use serde_json::json;
use state::{PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    current_exe: PathBuf,
    launch_agent_path: PathBuf,
    operations: RefCell<Vec<String>>,
}

impl TestEnvironment {
    fn new(
        home: &Utf8Path,
        current_dir: &Utf8Path,
        current_exe: &Utf8Path,
        launch_agent_path: &Utf8Path,
    ) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            current_exe: current_exe.as_std_path().to_path_buf(),
            launch_agent_path: launch_agent_path.as_std_path().to_path_buf(),
            operations: RefCell::new(Vec::new()),
        }
    }

    fn operations(&self) -> Vec<String> {
        self.operations.borrow().clone()
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
        Ok(self.current_exe.clone())
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

    fn bootstrap_launch_agent(&self, plist_path: &Utf8Path) -> Result<(), platform::PlatformError> {
        self.operations
            .borrow_mut()
            .push(format!("bootstrap {plist_path}"));

        Ok(())
    }

    fn bootout_launch_agent(&self) -> Result<(), platform::PlatformError> {
        self.operations
            .borrow_mut()
            .push(format!("bootout {LAUNCH_AGENT_LABEL}"));

        Ok(())
    }

    fn kickstart_launch_agent(&self) -> Result<(), platform::PlatformError> {
        self.operations
            .borrow_mut()
            .push(format!("kickstart {LAUNCH_AGENT_LABEL}"));

        Ok(())
    }
}

#[test]
fn daemon_enable_installs_pv_launch_agent_and_starts_it() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let daemon = DaemonFixture::start(&paths, 2)?;

    let output = run_pv(&["daemon:enable"], &environment)?;
    let _daemon_requests = daemon.finish()?;
    let plist = read_required_file(&launch_agent_path)?;
    let parsed = LaunchAgentConfig::parse(&plist);

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("LaunchAgent installed"));
    assert!(output.stderr.is_empty());
    assert_eq!(
        parsed,
        Some(LaunchAgentConfig::new(
            current_exe,
            paths.logs().join("launchd.out.log"),
            paths.logs().join("launchd.err.log"),
        ))
    );

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist));
    });

    Ok(())
}

#[test]
fn daemon_enable_waits_for_health_and_submits_reconciliation() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let daemon = DaemonFixture::start(&paths, 2)?;

    let output = run_pv(&["daemon:enable"], &environment)?;
    let daemon_requests = daemon.finish()?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        daemon_requests,
        vec![
            format!(
                r#"{{"protocol_version":{},"command":"health"}}"#,
                daemon::PROTOCOL_VERSION
            ),
            format!(
                r#"{{"protocol_version":{},"command":"run_job","kind":"reconcile","scope":"system"}}"#,
                daemon::PROTOCOL_VERSION
            ),
        ]
    );

    Ok(())
}

#[test]
fn daemon_disable_removes_only_pv_owned_launch_agent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let paths = PvPaths::for_home(&home);
    let config = LaunchAgentConfig::new(
        &current_exe,
        paths.logs().join("launchd.out.log"),
        paths.logs().join("launchd.err.log"),
    );
    write_file(&launch_agent_path, &config.render())?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_optional_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.contains("LaunchAgent removed"));
    assert!(output.stderr.is_empty());
    assert!(plist_after_disable.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist_after_disable));
    });

    Ok(())
}

#[test]
fn daemon_disable_refuses_non_pv_owned_launch_agent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let current_exe = tempdir.path().join("pv");
    let launch_agent_path = tempdir
        .path()
        .join("Library/LaunchAgents/com.prvious.pv.daemon.plist");
    let environment = TestEnvironment::new(&home, &current_dir, &current_exe, &launch_agent_path);
    let conflict =
        "<plist><dict><key>Label</key><string>com.prvious.pv.daemon</string></dict></plist>\n";
    write_file(&launch_agent_path, conflict)?;

    let output = run_pv(&["daemon:disable"], &environment)?;
    let plist_after_disable = read_required_file(&launch_agent_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.contains("not PV-owned"));
    assert!(output.stderr.is_empty());
    assert_eq!(plist_after_disable, conflict);

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, environment.operations(), plist_after_disable));
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

#[derive(Debug)]
struct DaemonFixture {
    requests: Arc<Mutex<Vec<String>>>,
    thread: thread::JoinHandle<anyhow::Result<()>>,
}

impl DaemonFixture {
    fn start(paths: &PvPaths, expected_requests: usize) -> anyhow::Result<Self> {
        state::fs::ensure_layout(paths)?;
        delete_optional_file(&paths.daemon_socket())?;
        let listener = UnixListener::bind(paths.daemon_socket().as_std_path())?;

        listener.set_nonblocking(true)?;

        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let thread = spawn_daemon_fixture_thread(move || {
            for _request_index in 0..expected_requests {
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
                } else {
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
                }
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
    reason = "CLI daemon tests run a synchronous fixture daemon on a short-lived thread"
)]
fn spawn_daemon_fixture_thread(
    operation: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
) -> thread::JoinHandle<anyhow::Result<()>> {
    thread::spawn(operation)
}

fn accept_with_timeout(
    listener: &UnixListener,
) -> anyhow::Result<(UnixStream, std::os::unix::net::SocketAddr)> {
    let started_at = Instant::now();

    loop {
        match listener.accept() {
            Ok(accepted) => return Ok(accepted),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if started_at.elapsed() > Duration::from_secs(3) {
                    return Err(anyhow::anyhow!(
                        "timed out waiting for daemon client request"
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn write_daemon_line(stream: &mut UnixStream, value: serde_json::Value) -> anyhow::Result<()> {
    writeln!(stream, "{value}")?;

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
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assertion);
}
