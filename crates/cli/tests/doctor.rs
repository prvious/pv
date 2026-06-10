use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use platform::{LaunchAgentConfig, PfConfReference, PfRedirectConfig, ResolverConfig};
use state::{Database, PvPaths, RuntimeObservedStatus, RuntimeSubject};

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

    fn launch_agent_path_utf8(&self) -> anyhow::Result<camino::Utf8PathBuf> {
        camino::Utf8PathBuf::from_path_buf(self.launch_agent_path.clone())
            .map_err(|path| anyhow::anyhow!("launch agent path is not UTF-8: {}", path.display()))
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
fn doctor_passes_when_required_checks_pass() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot(
        "doctor_passes_when_required_checks_pass",
        tempdir.path(),
        output,
    );

    Ok(())
}

#[test]
fn doctor_fails_with_repair_commands() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, true)?;
    state::fs::delete_file(&paths.daemon_socket())?;
    let mut database = Database::open(&paths)?;
    let job = database.start_job("reconcile", "system")?;
    database.fail_job(&job.id, "Gateway failed to start")?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Failed,
        Some("Gateway failed to start"),
    )?;

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot("doctor_fails_with_repair_commands", tempdir.path(), output);

    Ok(())
}

#[test]
fn doctor_warnings_do_not_fail() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);
    seed_required_checks(&paths, &environment, false)?;

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_doctor_snapshot("doctor_warnings_do_not_fail", tempdir.path(), output);

    Ok(())
}

#[test]
fn doctor_is_read_only() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home);

    let output = run_pv(&["doctor"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert!(!state::fs::path_exists(paths.root()));
    assert_doctor_snapshot("doctor_is_read_only", tempdir.path(), output);

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

fn seed_required_checks(
    paths: &PvPaths,
    environment: &TestEnvironment,
    include_manifest_cache: bool,
) -> anyhow::Result<()> {
    Database::open(paths)?;

    let launch_agent = LaunchAgentConfig::new(
        "/bin/pv",
        paths.launchd_stdout_log(),
        paths.launchd_stderr_log(),
    );
    platform::write_launch_agent_file(&environment.launch_agent_path_utf8()?, &launch_agent)?;
    write_file(&paths.daemon_socket(), "")?;
    state::fs::write_sensitive_file(
        &paths.resolver_config(),
        &ResolverConfig::new(35353).render(),
    )?;
    state::fs::write_sensitive_file(
        &paths.pf_anchor_config(),
        &PfRedirectConfig::new(48080, 48443).render_anchor(),
    )?;
    state::fs::write_sensitive_file(&paths.pf_conf_reference_config(), &PfConfReference.render())?;
    let ca = platform::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &ca.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &ca.private_key_pem)?;

    if include_manifest_cache {
        state::fs::write_sensitive_file(&paths.downloads().join("manifest.json"), "{}")?;
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI doctor tests create fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;

    Ok(())
}

fn assert_doctor_snapshot(name: &'static str, tempdir: &Utf8Path, snapshot: impl std::fmt::Debug) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r"job_[0-9]+", "<job-id>");
    settings.add_filter(r"[0-9a-f]{64}", "<fingerprint>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
