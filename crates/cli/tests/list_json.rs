use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};
use state::{
    Database, LinkProjectInput, ManagedResourceTrackInstallInput, PortRequest,
    ProjectManagedResourceInput, PvPaths, RuntimeObservedStatus, RuntimeSubject,
};

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
fn list_json_outputs_linked_projects() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "hostnames:\n  - api.acme.test\nmysql:\n  version: \"8.0\"\n",
    )?;
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home, &project);
    seed_project(&paths, &project)?;

    let output = run_pv(&["list", "--json"], &environment)?;
    let json = parse_json_output(output)?;

    assert_list_json_snapshot("list_json_outputs_linked_projects", tempdir.path(), &json);

    Ok(())
}

#[test]
fn resource_list_json_outputs_installed_tracks_and_aliases() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home, &home);
    seed_resource_tracks(&paths)?;
    let commands = [
        "php:list",
        "redis:list",
        "mysql:list",
        "postgres:list",
        "pg:list",
        "mailpit:list",
        "mail:list",
        "rustfs:list",
        "s3:list",
    ];
    let mut outputs = BTreeMap::new();

    for command in commands {
        let output = run_pv(&[command, "--json"], &environment)?;
        outputs.insert(command, parse_json_output(output)?);
    }

    assert_list_json_snapshot(
        "resource_list_json_outputs_installed_tracks_and_aliases",
        tempdir.path(),
        &outputs,
    );

    Ok(())
}

#[test]
fn resource_list_json_documents_composer_and_frankenphp_omissions() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = PvPaths::for_home(home.clone());
    let environment = TestEnvironment::new(&home, &home);
    seed_resource_tracks(&paths)?;
    let commands = [
        "php:list",
        "redis:list",
        "mysql:list",
        "postgres:list",
        "pg:list",
        "mailpit:list",
        "mail:list",
        "rustfs:list",
        "s3:list",
    ];
    let mut listed_resources = Vec::new();

    for command in commands {
        let output = run_pv(&[command, "--json"], &environment)?;
        let json = parse_json_output(output)?;
        collect_listed_resources(&json, &mut listed_resources);
    }

    listed_resources.sort();
    listed_resources.dedup();
    assert!(!listed_resources.contains(&"composer".to_string()));
    assert!(!listed_resources.contains(&"frankenphp".to_string()));
    assert_list_json_snapshot(
        "resource_list_json_documents_composer_and_frankenphp_omissions",
        tempdir.path(),
        &BTreeMap::from([
            (
                "installed_but_without_public_list_command",
                vec!["composer".to_string(), "frankenphp".to_string()],
            ),
            ("listed_resources", listed_resources),
        ]),
    );

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

fn parse_json_output(output: RunOutput) -> anyhow::Result<serde_json::Value> {
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());

    Ok(serde_json::from_str(&output.stdout)?)
}

fn seed_project(paths: &PvPaths, project_path: &Utf8Path) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;
    let project = database
        .link_project(LinkProjectInput {
            path: project_path.to_path_buf(),
            original_path: project_path.to_path_buf(),
            primary_hostname: "acme.test".to_string(),
            config_path: project_path.join("pv.yml"),
            desired_php_track: Some("8.4".to_string()),
            additional_hostnames: vec!["api.acme.test".to_string()],
        })?
        .project;
    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;

    Ok(())
}

fn seed_resource_tracks(paths: &PvPaths) -> anyhow::Result<()> {
    let mut database = Database::open(paths)?;
    let installs = [
        ("php", "8.4", "8.4.8-pv1"),
        ("frankenphp", "8.4", "8.4.8-pv1"),
        ("composer", "2", "2.8.1-pv1"),
        ("redis", "7", "7.2.5-pv1"),
        ("mysql", "8.0", "8.0.36-pv1"),
        ("postgres", "16", "16.4-pv1"),
        ("mailpit", "1", "1.20.0-pv1"),
        ("rustfs", "1", "1.0.0-pv1"),
    ];

    for (resource_name, track, installed_version) in installs {
        let artifact_path = paths
            .resources()
            .join(resource_name)
            .join(track)
            .join("artifact");
        database.record_managed_resource_tracks_desired_and_installed(&[
            ManagedResourceTrackInstallInput {
                resource_name,
                track,
                installed_version,
                current_artifact_path: &artifact_path,
            },
        ])?;
    }

    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "mysql".to_string(),
            track: "8.0".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("Managed Resource runtime is ready"),
    )?;
    database.assign_port(
        PortRequest::resource_port("mysql", "8.0", "tcp", 3306, 45000, 48999),
        |_| true,
    )?;

    Ok(())
}

fn collect_listed_resources(json: &serde_json::Value, listed_resources: &mut Vec<String>) {
    let Some(tracks) = json.get("tracks").and_then(serde_json::Value::as_array) else {
        return;
    };

    for track in tracks {
        if let Some(resource_name) = track.get("resource").and_then(serde_json::Value::as_str) {
            listed_resources.push(resource_name.to_string());
        } else {
            listed_resources.push("php".to_string());
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI list JSON tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI list JSON tests write fixture config files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}

fn assert_list_json_snapshot(
    name: &'static str,
    tempdir: &Utf8Path,
    snapshot: &impl std::fmt::Debug,
) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.add_filter(r#"String\("[a-z0-9]{10}"\)"#, r#"String("<project-id>")"#);
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
