use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use config::ProjectConfigFile;
use insta::{Settings, assert_debug_snapshot};
use state::{
    Database, EnvContextValues, LinkProjectInput, ProjectManagedResourceInput, ProjectMode,
    ProjectRecord, PvPaths, ResourceAllocationInput,
};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
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
        Ok(self.current_dir.borrow().clone())
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
fn project_env_renders_current_project_values_to_stdout() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    let nested = project.join("nested");
    create_dir(&nested)?;
    write_file(
        &project.join("pv.yml"),
        r#"env:
  APP_URL: "${project_url}"
  APP_ENV: local
  VITE_DEV_SERVER_KEY: "${tls_key}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
  PV_TLS_CA: "${tls_ca}"
"#,
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project_record.path.join("nested"));

    let output = run_pv(&["project:env"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(project_record.id.as_str(), "<project-id>");
    settings.bind(|| {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn project_env_resolves_additional_hostname_and_resource_values() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        r#"hostnames:
  - api.acme.test
mysql:
  version: "8.0"
  env:
    DB_HOST: "${host}"
    DB_PORT: "${port}"
  allocations:
    app:
      env:
        DATABASE_URL: "mysql://${username}:${password}@${host}:${port}/${database}"
        DB_DATABASE: "${database}"
"#,
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    record_mysql_context(&home, &project_record)?;
    let environment = TestEnvironment::new(&home, &project);

    let output = run_pv(&["project:env", "api.acme.test"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_env_json_renders_generated_values() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        r#"hostnames:
  - api.acme.test
mysql:
  version: "8.0"
  env:
    DB_HOST: "${host}"
  allocations:
    app:
      env:
        DATABASE_URL: "mysql://${username}:${password}@${host}:${port}/${database}"
        DB_DATABASE: "${database}"
"#,
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    record_mysql_context(&home, &project_record)?;
    let environment = TestEnvironment::new(&home, &project);

    let output = run_pv(&["project:env", "--json", "api.acme.test"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_env_returns_empty_output_for_no_mappings() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project_record.path);

    let output = run_pv(&["project:env"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_env_json_returns_empty_object_for_no_mappings() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project_record.path);

    let output = run_pv(&["project:env", "--json"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(output.stdout, "{}\n");
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_env_resolves_resource_only_slug_and_reads_configured_env_file() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Resource Project");
    create_dir(&project.join("config"))?;
    write_file(
        &project.join("pv.yml"),
        r#"serve: false
env_file: config/development.env
env:
  APP_NAME: resource-project
  APP_URL: "${project_url}"
"#,
    )?;
    let env_path = project.join("config/development.env");
    let env_before = "USER_VALUE=kept\n";
    write_file(&env_path, env_before)?;
    register_project_with_mode(&home, &project, "ignored.test", ProjectMode::ResourceOnly)?;
    let environment = TestEnvironment::new(&home, &project);

    let output = run_pv(&["project:env", "resource-project"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(output.stdout, "APP_NAME=resource-project\n");
    assert!(output.stderr.is_empty());
    assert_eq!(read_file(&env_path)?, env_before);
    assert!(!output.stdout.contains("project_url"));
    assert_debug_snapshot!((output, env_before));

    Ok(())
}

#[test]
fn project_env_rejects_bare_slug_hostname_ambiguity() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let resource_only = tempdir.path().join("acme");
    let served = tempdir.path().join("web");
    let outside = tempdir.path().join("outside");
    create_dir(&resource_only)?;
    create_dir(&served)?;
    create_dir(&outside)?;
    write_file(
        &resource_only.join("pv.yml"),
        "serve: false\nenv:\n  PROJECT_KIND: resource\n",
    )?;
    write_file(&served.join("pv.yml"), "env:\n  PROJECT_KIND: served\n")?;
    register_project_with_mode(
        &home,
        &resource_only,
        "ignored.test",
        ProjectMode::ResourceOnly,
    )?;
    register_project_with_mode(&home, &served, "acme.test", ProjectMode::Served)?;
    let environment = TestEnvironment::new(&home, &outside);

    let ambiguous = run_pv(&["project:env", "acme"], &environment)?;
    let explicit_hostname = run_pv(&["project:env", "acme.test"], &environment)?;

    assert_eq!(ambiguous.exit_code, ExitCode::FAILURE);
    assert!(ambiguous.stdout.is_empty());
    assert!(ambiguous.stderr.contains("full `acme.test` hostname"));
    assert_eq!(explicit_hostname.exit_code, ExitCode::SUCCESS);
    assert_eq!(explicit_hostname.stdout, "PROJECT_KIND=served\n");
    assert_debug_snapshot!((ambiguous, explicit_hostname));

    Ok(())
}

#[test]
fn project_env_writes_duplicate_warnings_to_stderr_without_mutating_dotenv() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    let env_path = project.join(".env");
    let existing_env = "APP_URL=https://user.test\nOTHER=value\n";
    write_file(&env_path, existing_env)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project_record.path);

    let output = run_pv(&["project:env"], &environment)?;
    let env_after = read_file(&env_path)?;
    let database = Database::open(&pv_paths(&home))?;
    let jobs = database.recent_jobs()?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(env_after, existing_env);
    assert!(jobs.is_empty());
    assert_debug_snapshot!((output, env_after, jobs));

    Ok(())
}

#[test]
fn project_env_json_keeps_duplicate_warnings_on_stderr() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    let env_path = project.join(".env");
    let existing_env = "APP_URL=https://user.test\nOTHER=value\n";
    write_file(&env_path, existing_env)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project_record.path);

    let output = run_pv(&["project:env", "--json"], &environment)?;
    let env_after = read_file(&env_path)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(env_after, existing_env);
    assert_debug_snapshot!((output, env_after));

    Ok(())
}

#[test]
fn project_env_reports_config_errors_to_stderr() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    write_file(&project.join("pv.yml"), "unexpected: true\n")?;
    let environment = TestEnvironment::new(&home, &project_record.path);

    let output = run_pv(&["project:env"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_env_reports_malformed_env_blocks_without_stdout() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    let env_path = project.join(".env");
    let existing_env = "# >>> PV MANAGED\nAPP_URL=old\n";
    write_file(&env_path, existing_env)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project_record.path);

    let output = run_pv(&["project:env"], &environment)?;
    let env_after = read_file(&env_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_eq!(env_after, existing_env);
    assert_debug_snapshot!((output, env_after));

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

fn register_project(
    home: &Utf8Path,
    project: &Utf8Path,
    primary_hostname: &str,
) -> anyhow::Result<ProjectRecord> {
    register_project_with_mode(home, project, primary_hostname, ProjectMode::Served)
}

fn register_project_with_mode(
    home: &Utf8Path,
    project: &Utf8Path,
    primary_hostname: &str,
    mode: ProjectMode,
) -> anyhow::Result<ProjectRecord> {
    let config_file = ProjectConfigFile::read_from_root(project)?;
    let project_path = project_root_from_config_path(&config_file.path)?;
    let desired_php_track = config_file
        .config
        .php
        .as_ref()
        .and_then(|php| php.version_selector())
        .map(str::to_owned);
    let mut database = Database::open(&pv_paths(home))?;
    let result = database.link_project_with_mode(
        LinkProjectInput {
            path: project_path,
            original_path: project.to_path_buf(),
            primary_hostname: primary_hostname.to_string(),
            config_path: config_file.path,
            desired_php_track,
            additional_hostnames: config_file.config.hostnames,
        },
        mode,
    )?;

    Ok(result.project)
}

fn record_mysql_context(home: &Utf8Path, project: &ProjectRecord) -> anyhow::Result<()> {
    let mut database = Database::open(&pv_paths(home))?;
    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &env_values(&[
            ("host", "127.0.0.1"),
            ("password", "root-secret"),
            ("port", "3306"),
            ("username", "root"),
        ]),
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app".to_string(),
            generated_name: "acme_test_app".to_string(),
        }],
    )?;
    database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app",
        &env_values(&[
            ("database", "acme_test_app"),
            ("password", "app-secret"),
            ("username", "app"),
        ]),
    )?;

    Ok(())
}

fn env_values(values: &[(&str, &str)]) -> EnvContextValues {
    values
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<BTreeMap<_, _>>()
}

fn project_root_from_config_path(config_path: &Utf8Path) -> anyhow::Result<Utf8PathBuf> {
    config_path
        .parent()
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("Project config path has no parent: {config_path}"))
}

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI project env tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI project env tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI project env tests read fixture files"
)]
fn read_file(path: &Utf8Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
