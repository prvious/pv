use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use state::{Database, ManagedResourceTrackInstallInput, PvPaths};

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
fn logs_defaults_to_daemon_sources() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    write_log(&paths.logs().join("daemon.log"), "daemon one\ndaemon two\n")?;
    write_log(&paths.logs().join("launchd.out.log"), "stdout one\n")?;
    write_log(&paths.logs().join("launchd.err.log"), "stderr one\n")?;

    let output = run_pv(&["logs", "-n", "1"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn logs_rejects_negative_lines() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["logs", "-n", "-1"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn logs_reports_missing_selected_source() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["logs", "--gateway"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn logs_gateway_uses_combined_fallback() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    write_log(&paths.gateway_log(), "gateway one\ngateway two\n")?;

    let output = run_pv(&["logs", "--gateway", "-n", "1"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn logs_resource_alias_infers_single_installed_track() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_installed_track(&paths, "postgres", "16")?;
    write_log(&paths.resource_log("postgres", "16"), "postgres ready\n")?;

    let output = run_pv(&["logs", "--resource", "pg"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn logs_resource_alias_requires_track_when_ambiguous() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_installed_track(&paths, "postgres", "15")?;
    seed_installed_track(&paths, "postgres", "16")?;

    let output = run_pv(&["logs", "--resource", "pg"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_debug_snapshot!(output);

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
    reason = "CLI logs tests create fixture log files"
)]
fn write_log(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;

    Ok(())
}

fn seed_installed_track(paths: &PvPaths, resource_name: &str, track: &str) -> anyhow::Result<()> {
    let artifact_path = paths
        .resources()
        .join(resource_name)
        .join(track)
        .join("artifact");
    let mut database = Database::open(paths)?;
    database.record_managed_resource_tracks_desired_and_installed(&[
        ManagedResourceTrackInstallInput {
            resource_name,
            track,
            installed_version: "1.0.0",
            current_artifact_path: &artifact_path,
        },
    ])?;

    Ok(())
}
