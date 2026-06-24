use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use config::ProjectConfigFile;
use insta::assert_debug_snapshot;
use resources::{ResourceHttpClient, ResourcesError, TargetPlatform};
use state::{
    Database, LinkProjectInput, ManagedResourceDesiredState, ManagedResourceTrackRecord,
    ProjectRecord, PvPaths, fs,
};

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    client: ScriptedClient,
    target_platform: Option<TargetPlatform>,
    exec_calls: RefCell<Vec<ExecCall>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, client: ScriptedClient) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            client,
            target_platform: None,
            exec_calls: RefCell::new(Vec::new()),
        }
    }

    fn with_target_platform(mut self, target_platform: TargetPlatform) -> Self {
        self.target_platform = Some(target_platform);
        self
    }

    fn text_request_count(&self) -> usize {
        self.client.text_request_count()
    }

    fn byte_request_count(&self) -> usize {
        self.client.byte_request_count()
    }

    fn exec_calls(&self) -> Vec<ExecCall> {
        self.exec_calls.borrow().clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecCall {
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

fn exec_env_snapshot(env: &[(OsString, OsString)]) -> Vec<(String, String)> {
    env.iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

fn php_exec_env(home: &Utf8Path, track: &str) -> anyhow::Result<Vec<(String, String)>> {
    let defaults = resources::php_track_defaults(&pv_paths(home), track)?;

    Ok(vec![
        ("PHPRC".to_string(), defaults.etc_dir().to_string()),
        (
            "PHP_INI_SCAN_DIR".to_string(),
            defaults.conf_dir().to_string(),
        ),
    ])
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

    fn exec(&self, program: &Path, args: &[String]) -> io::Result<ExitCode> {
        self.exec_with_env(program, args, &[])
    }

    fn exec_with_env(
        &self,
        program: &Path,
        args: &[String],
        env: &[(OsString, OsString)],
    ) -> io::Result<ExitCode> {
        self.exec_calls.borrow_mut().push(ExecCall {
            program: program.to_path_buf(),
            args: args.to_vec(),
            env: exec_env_snapshot(env),
        });

        Ok(ExitCode::SUCCESS)
    }

    fn artifact_manifest_url(&self) -> Option<String> {
        Some(MANIFEST_URL.to_string())
    }

    fn resource_http_client(&self) -> Option<&dyn ResourceHttpClient> {
        Some(&self.client)
    }

    fn target_platform(&self) -> Option<TargetPlatform> {
        self.target_platform
    }
}

#[test]
fn php_shim_fails_clearly_when_resolved_project_track_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.4")?;
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-v"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert!(environment.exec_calls().is_empty());
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn php_shim_execs_resolved_project_track() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.4")?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-v"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        exec_calls,
        vec![ExecCall {
            program: release.join("bin/php").as_std_path().to_path_buf(),
            args: vec!["-v".to_string()],
            env: php_exec_env(&home, "8.4")?,
        }]
    );
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((output, exec_calls));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_shim_uses_project_extension_runtime_overlay() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    fs::write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    fs::write_sensitive_file(&release.join("lib/php/extensions/redis.so"), "")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.4".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: vec!["redis".to_string()],
                ignored_extensions: Vec::new(),
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(exec_calls[0].env.iter().any(|(key, value)| {
        key == "PHP_INI_SCAN_DIR" && value.contains("php-runtimes/8.4+redis/conf.d")
    }));

    Ok(())
}

#[test]
fn php_shim_resolves_project_config_extensions_when_persisted_runtime_is_empty()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.4")?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    fs::write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    fs::write_sensitive_file(&release.join("lib/php/extensions/redis.so"), "")?;
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(exec_calls[0].env.iter().any(|(key, value)| {
        key == "PHP_INI_SCAN_DIR" && value.contains("php-runtimes/8.4+redis/conf.d")
    }));

    Ok(())
}

#[test]
fn php_shim_uses_base_runtime_when_project_config_removes_extensions() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php:\n  version: 8.4\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    fs::write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    fs::write_sensitive_file(&release.join("lib/php/extensions/redis.so"), "")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.4".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: vec!["redis".to_string()],
                ignored_extensions: Vec::new(),
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(exec_calls[0].env, php_exec_env(&home, "8.4")?);

    Ok(())
}

#[test]
fn php_shim_reuses_persisted_empty_extension_runtime() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    fs::write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    fs::write_sensitive_file(&release.join("lib/php/extensions/redis.so"), "")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.4".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: Vec::new(),
                ignored_extensions: vec!["redis".to_string()],
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(exec_calls[0].env, php_exec_env(&home, "8.4")?);

    Ok(())
}

#[test]
fn php_shim_resolves_config_track_when_recomputing_extensions() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  version: 8.4\n  extensions: [xdebug]\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let old_release = record_installed_php(&home, "8.3", "8.3.25-pv1")?;
    fs::write_sensitive_file(
        &old_release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    fs::write_sensitive_file(&old_release.join("lib/php/extensions/redis.so"), "")?;
    let new_release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    fs::write_sensitive_file(
        &new_release.join("share/pv/php-extensions.json"),
        r#"[{"name":"xdebug","load_kind":"zend_extension","path":"lib/php/extensions/xdebug.so"}]"#,
    )?;
    fs::write_sensitive_file(&new_release.join("lib/php/extensions/xdebug.so"), "")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.3".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: vec!["redis".to_string()],
                ignored_extensions: Vec::new(),
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        exec_calls[0].program,
        new_release.join("bin/php").as_std_path().to_path_buf(),
    );
    assert!(exec_calls[0].env.iter().any(|(key, value)| {
        key == "PHP_INI_SCAN_DIR" && value.contains("php-runtimes/8.4+xdebug/conf.d")
    }));

    Ok(())
}

#[test]
fn php_shim_fails_when_persisted_loaded_extension_metadata_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.4".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: vec!["redis".to_string()],
                ignored_extensions: Vec::new(),
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(environment.exec_calls().is_empty());
    assert!(output.stderr.contains("persisted PHP extension `redis`"));

    Ok(())
}

#[test]
fn php_shim_sets_only_php_ini_env_overlay() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.4")?;
    }
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["shim:php", "--ini"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        exec_calls,
        vec![ExecCall {
            program: release.join("bin/php").as_std_path().to_path_buf(),
            args: vec!["--ini".to_string()],
            env: php_exec_env(&home, "8.4")?,
        }]
    );
    let defaults = resources::php_track_defaults(&pv_paths(&home), "8.4")?;
    assert_eq!(
        fs::read_to_string(defaults.php_ini())?,
        resources::PHP_TRACK_DEFAULT_INI
    );

    Ok(())
}

#[test]
fn php_shim_execs_global_default_track_outside_project() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let release = record_installed_php(&home, "8.3", "8.3.12-pv1")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.3")?;
    }
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-r", "echo 1;"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        exec_calls,
        vec![ExecCall {
            program: release.join("bin/php").as_std_path().to_path_buf(),
            args: vec!["-r".to_string(), "echo 1;".to_string()],
            env: php_exec_env(&home, "8.3")?,
        }]
    );
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((output, exec_calls));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_shim_forwards_help_and_version_flags() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.4")?;
    }
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let outputs = [
        run_pv(&["shim:php", "--help"], &environment)?,
        run_pv(&["shim:php", "-h"], &environment)?,
        run_pv(&["shim:php", "--version"], &environment)?,
        run_pv(&["shim:php", "-V"], &environment)?,
    ];
    let exec_calls = environment.exec_calls();
    let env = php_exec_env(&home, "8.4")?;

    assert!(
        outputs
            .iter()
            .all(|output| output.exit_code == ExitCode::SUCCESS)
    );
    assert!(outputs.iter().all(|output| output.stdout.is_empty()));
    assert!(outputs.iter().all(|output| output.stderr.is_empty()));
    assert_eq!(
        exec_calls,
        ["--help", "-h", "--version", "-V"]
            .into_iter()
            .map(|arg| ExecCall {
                program: release.join("bin/php").as_std_path().to_path_buf(),
                args: vec![arg.to_string()],
                env: env.clone(),
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((outputs, exec_calls));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_shim_uses_cached_manifest_default_without_network() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    cache_manifest(&home, &php_pair_manifest("8.4", &artifacts))?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["shim:php", "--ini"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        exec_calls,
        vec![ExecCall {
            program: release.join("bin/php").as_std_path().to_path_buf(),
            args: vec!["--ini".to_string()],
            env: php_exec_env(&home, "8.4")?,
        }]
    );
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((output, exec_calls));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_use_updates_project_config_state_and_reports_missing_daemon() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "hostnames:\n  - api.acme.test\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &project_record.path,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );

    let output = run_pv(&["php:use", "8.4"], &environment)?;
    let config_file = ProjectConfigFile::read_from_root(&project)?;
    let database = Database::open(&pv_paths(&home))?;
    let project_after = database
        .project_by_id(&project_record.id)?
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_eq!(
        config_file
            .config
            .php
            .as_ref()
            .and_then(|php| php.version_selector()),
        Some("8.4")
    );
    assert_eq!(project_after.desired_php_track.as_deref(), Some("8.4"));
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            config_file.config,
            project_snapshot(&project_after, tempdir.path())?,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_use_latest_preserves_alias_in_config_and_records_resolved_track() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "hostnames:\n  - api.acme.test\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &project_record.path,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );

    let output = run_pv(&["php:use", "latest"], &environment)?;
    let config_file = ProjectConfigFile::read_from_root(&project)?;
    let config_after = read_file(&project.join("pv.yml"))?;
    let database = Database::open(&pv_paths(&home))?;
    let project_after = database
        .project_by_id(&project_record.id)?
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_eq!(
        config_file
            .config
            .php
            .as_ref()
            .and_then(|php| php.version_selector()),
        Some("latest")
    );
    assert_eq!(project_after.desired_php_track.as_deref(), Some("8.4"));
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            config_after,
            config_file.config,
            project_snapshot(&project_after, tempdir.path())?,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_use_global_records_default_and_reports_missing_daemon() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );

    let output = run_pv(&["php:use", "8.4", "--global"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let default_track = database.global_php_default_track()?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_eq!(default_track.as_deref(), Some("8.4"));
    assert_debug_snapshot!((
        output,
        default_track,
        resource_record_snapshots(&records, tempdir.path())?,
        environment.text_request_count(),
        environment.byte_request_count(),
    ));

    Ok(())
}

#[test]
fn php_use_global_install_failure_leaves_default_and_state_unchanged() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let existing_artifacts = php_pair_artifacts("8.3.24-pv1")?;
    let target_artifacts = php_pair_artifacts("8.4.8-pv1")?;
    record_installed_php_pair(&home, "8.3", &existing_artifacts)?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.3")?;
    }
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &target_artifacts)),
    );

    let output = run_pv(&["php:use", "8.4", "--global"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let default_track = database.global_php_default_track()?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_eq!(default_track.as_deref(), Some("8.3"));
    assert_eq!(environment.text_request_count(), 1);
    assert_eq!(environment.byte_request_count(), 2);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            default_track,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_use_install_failure_leaves_project_config_and_state_unchanged() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let original_config = "php: '8.3'\nhostnames:\n  - api.acme.test\n";
    write_file(&project.join("pv.yml"), original_config)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.3")?;
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["php:use", "8.4"], &environment)?;
    let config_after = read_file(&project.join("pv.yml"))?;
    let database = Database::open(&pv_paths(&home))?;
    let project_after = database
        .project_by_id(&project_record.id)?
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_eq!(config_after, original_config);
    assert_eq!(project_after.desired_php_track.as_deref(), Some("8.3"));
    assert!(records.is_empty());
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            config_after,
            project_snapshot(&project_after, tempdir.path())?,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_use_config_conflict_leaves_project_config_and_state_unchanged() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let preferred_config = "php: '8.3'\nhostnames:\n  - api.acme.test\n";
    let alternate_config = "php: '8.2'\n";
    write_file(&project.join("pv.yml"), preferred_config)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.3")?;
    write_file(&project.join("pv.yaml"), alternate_config)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &project_record.path,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );

    let output = run_pv(&["php:use", "8.4"], &environment)?;
    let preferred_after = read_file(&project.join("pv.yml"))?;
    let alternate_after = read_file(&project.join("pv.yaml"))?;
    let database = Database::open(&pv_paths(&home))?;
    let project_after = database
        .project_by_id(&project_record.id)?
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert_eq!(preferred_after, preferred_config);
    assert_eq!(alternate_after, alternate_config);
    assert_eq!(project_after.desired_php_track.as_deref(), Some("8.3"));
    assert!(records.is_empty());
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            preferred_after,
            alternate_after,
            project_snapshot(&project_after, tempdir.path())?,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_install_uses_manifest_default_and_installs_pair_without_network() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );

    let output = run_pv(&["php:install"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!((
        output,
        resource_record_snapshots(&records, tempdir.path())?,
        environment.text_request_count(),
        environment.byte_request_count(),
    ));

    Ok(())
}

#[test]
fn php_install_warns_when_newest_artifact_is_revoked() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let fallback_artifacts = php_pair_artifacts("8.4.8-pv1")?;
    let revoked_artifacts = PhpPairArtifacts {
        php: revoked_artifact(
            runtime_fixture_artifact("php", "8.4.9-pv1", "bin/php", TargetPlatform::DarwinArm64),
            "failed smoke test",
        ),
        frankenphp: revoked_artifact(
            runtime_fixture_artifact(
                "frankenphp",
                "8.4.9-pv1",
                "bin/frankenphp",
                TargetPlatform::DarwinArm64,
            ),
            "failed smoke test",
        ),
    };
    let manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![manifest_track(
                "8.4",
                vec![&fallback_artifacts.php, &revoked_artifacts.php],
            )],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![manifest_track(
                "8.4",
                vec![
                    &fallback_artifacts.frankenphp,
                    &revoked_artifacts.frankenphp,
                ],
            )],
        ),
    ]);
    prepare_existing_php_pair_releases(&home, "8.4", &fallback_artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    );

    let output = run_pv(&["php:install", "8.4"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_eq!(environment.byte_request_count(), 0);
    assert_debug_snapshot!((
        output,
        resource_record_snapshots(&records, tempdir.path())?,
        environment.text_request_count(),
        environment.byte_request_count(),
    ));

    Ok(())
}

#[test]
fn php_update_reports_missing_daemon_after_resource_update() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let old_artifacts = php_pair_artifacts("8.4.7-pv1")?;
    let new_artifacts = php_pair_artifacts("8.4.8-pv1")?;
    record_installed_php_pair(&home, "8.4", &old_artifacts)?;
    prepare_existing_php_pair_releases(&home, "8.4", &new_artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &new_artifacts)),
    );

    let output = run_pv(&["php:update"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!((
        output,
        resource_record_snapshots(&records, tempdir.path())?,
        environment.text_request_count(),
        environment.byte_request_count(),
    ));

    Ok(())
}

#[test]
fn php_update_warns_when_newest_artifact_is_revoked() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let old_artifacts = php_pair_artifacts("8.4.7-pv1")?;
    let fallback_artifacts = php_pair_artifacts("8.4.8-pv1")?;
    let revoked_artifacts = PhpPairArtifacts {
        php: revoked_artifact(
            runtime_fixture_artifact("php", "8.4.9-pv1", "bin/php", TargetPlatform::DarwinArm64),
            "failed smoke test",
        ),
        frankenphp: revoked_artifact(
            runtime_fixture_artifact(
                "frankenphp",
                "8.4.9-pv1",
                "bin/frankenphp",
                TargetPlatform::DarwinArm64,
            ),
            "failed smoke test",
        ),
    };
    let manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![manifest_track(
                "8.4",
                vec![&fallback_artifacts.php, &revoked_artifacts.php],
            )],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![manifest_track(
                "8.4",
                vec![
                    &fallback_artifacts.frankenphp,
                    &revoked_artifacts.frankenphp,
                ],
            )],
        ),
    ]);
    record_installed_php_pair(&home, "8.4", &old_artifacts)?;
    prepare_existing_php_pair_releases(&home, "8.4", &fallback_artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    );

    let output = run_pv(&["php:update"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_eq!(environment.byte_request_count(), 0);
    assert_debug_snapshot!((
        output,
        resource_record_snapshots(&records, tempdir.path())?,
        environment.text_request_count(),
        environment.byte_request_count(),
    ));

    Ok(())
}

#[test]
fn php_install_uses_injected_target_platform() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifacts = php_pair_artifacts_for_platform("8.4.8-pv1", TargetPlatform::DarwinAmd64)?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    )
    .with_target_platform(TargetPlatform::DarwinAmd64);

    let output = run_pv(&["php:install", "8.4"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_debug_snapshot!((
        output,
        resource_record_snapshots(&records, tempdir.path())?,
        environment.text_request_count(),
        environment.byte_request_count(),
    ));

    Ok(())
}

#[test]
fn php_uninstall_refuses_project_selected_track_without_force() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &project,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );
    let install = run_pv(&["php:install", "8.4"], &environment)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.4")?;

    let uninstall = run_pv(&["php:uninstall", "8.4"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(install.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::FAILURE);
    assert!(uninstall.stdout.is_empty());
    assert!(
        records
            .iter()
            .all(|record| record.desired_state == ManagedResourceDesiredState::Installed)
    );
    assert_debug_snapshot!((
        uninstall,
        resource_record_snapshots(&records, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn php_uninstall_refuses_global_default_track_without_force() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );
    let install = run_pv(&["php:install", "8.4"], &environment)?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.4")?;
    }

    let uninstall = run_pv(&["php:uninstall", "8.4"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(install.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::FAILURE);
    assert!(uninstall.stdout.is_empty());
    assert!(
        records
            .iter()
            .all(|record| record.desired_state == ManagedResourceDesiredState::Installed)
    );
    assert_debug_snapshot!((
        uninstall,
        resource_record_snapshots(&records, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn php_uninstall_refuses_manifest_default_track_inherited_by_project_without_force()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "hostnames:\n  - api.acme.test\n")?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    cache_manifest(&home, &php_pair_manifest("8.4", &artifacts))?;
    record_installed_php_pair(&home, "8.4", &artifacts)?;
    let _project_record = register_project(&home, &project, "acme.test")?;
    let environment = TestEnvironment::new(&home, &project, ScriptedClient::new());

    let uninstall = run_pv(&["php:uninstall", "8.4"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(uninstall.exit_code, ExitCode::FAILURE);
    assert!(uninstall.stdout.is_empty());
    assert!(
        records
            .iter()
            .all(|record| record.desired_state == ManagedResourceDesiredState::Installed)
    );
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            uninstall,
            resource_record_snapshots(&records, tempdir.path())?,
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_uninstall_force_proceeds_for_project_selected_track() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &project,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );
    let install = run_pv(&["php:install", "8.4"], &environment)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    select_project_php_track(&home, &project_record, "8.4")?;

    let uninstall = run_pv(&["php:uninstall", "8.4", "--force"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(install.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::SUCCESS);
    assert!(uninstall.stderr.is_empty());
    assert!(records.iter().all(|record| {
        record.desired_state == ManagedResourceDesiredState::Removed && record.removal_force
    }));
    assert_debug_snapshot!((
        uninstall,
        resource_record_snapshots(&records, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn php_uninstall_force_prune_queues_both_removal_intents() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );
    let install = run_pv(&["php:install", "8.4"], &environment)?;

    let uninstall = run_pv(
        &["php:uninstall", "8.4", "--force", "--prune"],
        &environment,
    )?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(install.exit_code, ExitCode::SUCCESS);
    assert_eq!(uninstall.exit_code, ExitCode::SUCCESS);
    assert!(uninstall.stderr.is_empty());
    assert!(records.iter().all(|record| {
        record.desired_state == ManagedResourceDesiredState::Removed
            && record.removal_force
            && record.removal_prune
    }));
    assert_debug_snapshot!((
        uninstall,
        resource_record_snapshots(&records, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn php_list_marks_global_default_track() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let explicit_project = tempdir.path().join("explicit");
    let default_project = tempdir.path().join("default");
    create_dir(&explicit_project)?;
    create_dir(&default_project)?;
    write_file(
        &explicit_project.join("pv.yml"),
        "hostnames:\n  - api.explicit.test\n",
    )?;
    write_file(
        &default_project.join("pv.yml"),
        "hostnames:\n  - api.default.test\n",
    )?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    prepare_existing_php_pair_releases(&home, "8.4", &artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &explicit_project,
        ScriptedClient::new().with_text(&php_pair_manifest("8.4", &artifacts)),
    );
    let install = run_pv(&["php:install", "8.4"], &environment)?;
    let explicit_project_record = register_project(&home, &explicit_project, "explicit.test")?;
    let _default_project_record = register_project(&home, &default_project, "default.test")?;
    select_project_php_track(&home, &explicit_project_record, "8.4")?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.4")?;
    }

    let list = run_pv(&["php:list"], &environment)?;

    assert_eq!(install.exit_code, ExitCode::SUCCESS);
    assert_eq!(list.exit_code, ExitCode::SUCCESS);
    assert!(list.stderr.is_empty());
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!(list);
        Ok(())
    })?;

    Ok(())
}

#[test]
fn php_list_marks_manifest_default_track_for_inherited_projects() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let default_project = tempdir.path().join("default");
    create_dir(&default_project)?;
    write_file(
        &default_project.join("pv.yml"),
        "hostnames:\n  - api.default.test\n",
    )?;
    let artifacts = php_pair_artifacts("8.4.8-pv1")?;
    cache_manifest(&home, &php_pair_manifest("8.4", &artifacts))?;
    record_installed_php_pair(&home, "8.4", &artifacts)?;
    let _default_project_record = register_project(&home, &default_project, "default.test")?;
    let environment = TestEnvironment::new(&home, &default_project, ScriptedClient::new());

    let list = run_pv(&["php:list"], &environment)?;

    assert_eq!(list.exit_code, ExitCode::SUCCESS);
    assert!(list.stderr.is_empty());
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!(list);
        Ok(())
    })?;

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
    let config_file = ProjectConfigFile::read_from_root(project)?;
    let project_path = project_root_from_config_path(&config_file.path)?;
    let mut database = Database::open(&pv_paths(home))?;
    let result = database.link_project(LinkProjectInput {
        path: project_path,
        original_path: project.to_path_buf(),
        primary_hostname: primary_hostname.to_string(),
        config_path: config_file.path,
        desired_php_track: None,
        additional_hostnames: config_file.config.hostnames,
    })?;

    Ok(result.project)
}

fn select_project_php_track(
    home: &Utf8Path,
    project: &ProjectRecord,
    track: &str,
) -> anyhow::Result<()> {
    let mut database = Database::open(&pv_paths(home))?;
    database.replace_project_desired_php_track(&project.id, Some(track))?;

    Ok(())
}

fn managed_resource_records(
    database: &Database,
) -> anyhow::Result<Vec<ManagedResourceTrackRecord>> {
    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .filter(|record| record.resource_name == "php" || record.resource_name == "frankenphp")
        .collect())
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct ProjectSnapshot {
    primary_hostname: String,
    path: String,
    config_path: String,
    desired_php_track: Option<String>,
}

fn project_snapshot(project: &ProjectRecord, root: &Utf8Path) -> anyhow::Result<ProjectSnapshot> {
    Ok(ProjectSnapshot {
        primary_hostname: project.primary_hostname.clone(),
        path: normalize_path(&project.path, root),
        config_path: normalize_path(&project.config_path, root),
        desired_php_track: project.desired_php_track.clone(),
    })
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct ResourceRecordSnapshot {
    resource_name: String,
    track: String,
    desired_state: String,
    installed_version: Option<String>,
    current_artifact_path: Option<String>,
    usage_count: i64,
    removal_prune: bool,
    removal_force: bool,
}

fn resource_record_snapshots(
    records: &[ManagedResourceTrackRecord],
    root: &Utf8Path,
) -> anyhow::Result<Vec<ResourceRecordSnapshot>> {
    records
        .iter()
        .map(|record| resource_record_snapshot(record, root))
        .collect()
}

fn resource_record_snapshot(
    record: &ManagedResourceTrackRecord,
    root: &Utf8Path,
) -> anyhow::Result<ResourceRecordSnapshot> {
    Ok(ResourceRecordSnapshot {
        resource_name: record.resource_name.clone(),
        track: record.track.clone(),
        desired_state: format!("{:?}", record.desired_state),
        installed_version: record.installed_version.clone(),
        current_artifact_path: record
            .current_artifact_path
            .as_ref()
            .map(|path| path.strip_prefix(root).map(Utf8Path::to_string))
            .transpose()?,
        usage_count: record.usage_count,
        removal_prune: record.removal_prune,
        removal_force: record.removal_force,
    })
}

fn normalize_path(path: &Utf8Path, root: &Utf8Path) -> String {
    let path = path.as_str();
    let root = root.as_str();
    let private_root = format!("/private{root}");

    if let Some(stripped) = path.strip_prefix(root) {
        return stripped.trim_start_matches('/').to_string();
    }
    if let Some(stripped) = path.strip_prefix(&private_root) {
        return stripped.trim_start_matches('/').to_string();
    }

    path.to_string()
}

#[derive(Debug)]
struct PhpPairArtifacts {
    php: FixtureArtifact,
    frankenphp: FixtureArtifact,
}

fn php_pair_artifacts(version: &str) -> anyhow::Result<PhpPairArtifacts> {
    php_pair_artifacts_for_platform(version, TargetPlatform::DarwinArm64)
}

fn php_pair_artifacts_for_platform(
    version: &str,
    target_platform: TargetPlatform,
) -> anyhow::Result<PhpPairArtifacts> {
    Ok(PhpPairArtifacts {
        php: runtime_fixture_artifact("php", version, "bin/php", target_platform),
        frankenphp: runtime_fixture_artifact(
            "frankenphp",
            version,
            "bin/frankenphp",
            target_platform,
        ),
    })
}

fn php_pair_manifest(default_track: &str, artifacts: &PhpPairArtifacts) -> String {
    manifest_with_resources(&[
        manifest_resource(
            "php",
            default_track,
            vec![manifest_track(default_track, vec![&artifacts.php])],
        ),
        manifest_resource(
            "frankenphp",
            default_track,
            vec![manifest_track(default_track, vec![&artifacts.frankenphp])],
        ),
    ])
}

#[derive(Clone, Debug)]
struct FixtureArtifact {
    resource_name: String,
    version: String,
    platform: String,
    executable_path: String,
    sha256: String,
    revoked_reason: Option<String>,
}

fn runtime_fixture_artifact(
    resource_name: &str,
    version: &str,
    executable_path: &str,
    target_platform: TargetPlatform,
) -> FixtureArtifact {
    FixtureArtifact {
        resource_name: resource_name.to_string(),
        version: version.to_string(),
        platform: target_platform.as_str().to_string(),
        executable_path: executable_path.to_string(),
        sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        revoked_reason: None,
    }
}

fn revoked_artifact(mut artifact: FixtureArtifact, reason: &str) -> FixtureArtifact {
    artifact.revoked_reason = Some(reason.to_string());
    artifact
}

fn prepare_existing_php_pair_releases(
    home: &Utf8Path,
    track: &str,
    artifacts: &PhpPairArtifacts,
) -> anyhow::Result<()> {
    prepare_existing_release(home, track, &artifacts.php)?;
    prepare_existing_release(home, track, &artifacts.frankenphp)?;

    Ok(())
}

fn prepare_existing_release(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
) -> anyhow::Result<()> {
    let release = pv_paths(home)
        .resources()
        .join(&artifact.resource_name)
        .join(track)
        .join("releases")
        .join(&artifact.version);
    let executable = release.join(&artifact.executable_path);
    let parent = executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("fixture executable has no parent: {executable}"))?;
    create_dir(parent)?;
    write_file(&executable, "fixture executable\n")
}

fn record_installed_php(
    home: &Utf8Path,
    track: &str,
    version: &str,
) -> anyhow::Result<Utf8PathBuf> {
    let artifact = runtime_fixture_artifact("php", version, "bin/php", TargetPlatform::DarwinArm64);
    prepare_existing_release(home, track, &artifact)?;
    let release = release_path(home, track, &artifact);
    let mut database = Database::open(&pv_paths(home))?;
    database.record_managed_resource_track_installed("php", track, version, &release)?;

    Ok(release)
}

fn record_installed_php_pair(
    home: &Utf8Path,
    track: &str,
    artifacts: &PhpPairArtifacts,
) -> anyhow::Result<()> {
    prepare_existing_php_pair_releases(home, track, artifacts)?;
    let mut database = Database::open(&pv_paths(home))?;
    let php_release = release_path(home, track, &artifacts.php);
    database.record_managed_resource_track_installed(
        "php",
        track,
        &artifacts.php.version,
        &php_release,
    )?;
    let frankenphp_release = release_path(home, track, &artifacts.frankenphp);
    database.record_managed_resource_track_installed(
        "frankenphp",
        track,
        &artifacts.frankenphp.version,
        &frankenphp_release,
    )?;

    Ok(())
}

fn release_path(home: &Utf8Path, track: &str, artifact: &FixtureArtifact) -> Utf8PathBuf {
    pv_paths(home)
        .resources()
        .join(&artifact.resource_name)
        .join(track)
        .join("releases")
        .join(&artifact.version)
}

fn cache_manifest(home: &Utf8Path, manifest: &str) -> anyhow::Result<()> {
    let paths = pv_paths(home);
    let downloads = paths.downloads();
    create_dir(downloads)?;
    write_file(&downloads.join("manifest.json"), manifest)
}

struct ManifestResourceFixture<'a> {
    name: &'a str,
    default_track: &'a str,
    tracks: Vec<ManifestTrackFixture<'a>>,
}

struct ManifestTrackFixture<'a> {
    name: &'a str,
    artifacts: Vec<&'a FixtureArtifact>,
}

fn manifest_resource<'a>(
    name: &'a str,
    default_track: &'a str,
    tracks: Vec<ManifestTrackFixture<'a>>,
) -> ManifestResourceFixture<'a> {
    ManifestResourceFixture {
        name,
        default_track,
        tracks,
    }
}

fn manifest_track<'a>(
    name: &'a str,
    artifacts: Vec<&'a FixtureArtifact>,
) -> ManifestTrackFixture<'a> {
    ManifestTrackFixture { name, artifacts }
}

fn manifest_with_resources(resources: &[ManifestResourceFixture<'_>]) -> String {
    let resources = resources
        .iter()
        .map(|resource| {
            let tracks = resource
                .tracks
                .iter()
                .map(manifest_track_json)
                .collect::<Vec<_>>()
                .join(",");

            format!(
                r#"{{
      "name": "{}",
      "default_track": "{}",
      "tracks": [
        {tracks}
      ]
    }}"#,
                resource.name, resource.default_track,
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"
{{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {resources}
  ]
}}
"#
    )
}

fn manifest_track_json(track: &ManifestTrackFixture<'_>) -> String {
    let artifacts = track
        .artifacts
        .iter()
        .map(|artifact| {
            let revocation = artifact
                .revoked_reason
                .as_ref()
                .map_or_else(String::new, |reason| {
                    format!(
                        r#",
              "revoked": true,
              "revocation_reason": "{reason}""#
                    )
                });

            format!(
                r#"{{
              "artifact_version": "{}",
              "upstream_version": "{}",
              "pv_build_revision": "1",
              "platform": "{}",
              "url": "https://artifacts.example.test/{}-{}-{}.tar.gz",
              "sha256": "{}",
              "size": {},
              "published_at": "{}"{revocation}
            }}"#,
                artifact.version,
                artifact.version.trim_end_matches("-pv1"),
                artifact.platform,
                artifact.resource_name,
                artifact.version,
                artifact.platform,
                artifact.sha256,
                0,
                published_at_for(&artifact.version),
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"{{
          "name": "{}",
          "artifacts": [
            {artifacts}
          ]
        }}"#,
        track.name,
    )
}

fn published_at_for(version: &str) -> &'static str {
    match version {
        "8.3.24-pv1" => "2026-05-27T12:30:00Z",
        "8.4.7-pv1" => "2026-05-25T13:30:00Z",
        "8.4.8-pv1" => "2026-05-26T13:30:00Z",
        "8.4.9-pv1" => "2026-05-27T13:30:00Z",
        _ => "2026-05-26T13:30:00Z",
    }
}

#[derive(Debug)]
struct ScriptedClient {
    text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    text_request_count: Cell<usize>,
    byte_request_count: Cell<usize>,
}

impl ScriptedClient {
    fn new() -> Self {
        Self {
            text_responses: RefCell::new(VecDeque::new()),
            text_request_count: Cell::new(0),
            byte_request_count: Cell::new(0),
        }
    }

    fn with_text(self, text: &str) -> Self {
        self.text_responses
            .borrow_mut()
            .push_back(Ok(text.to_string()));
        self
    }

    fn text_request_count(&self) -> usize {
        self.text_request_count.get()
    }

    fn byte_request_count(&self) -> usize {
        self.byte_request_count.get()
    }
}

impl ResourceHttpClient for ScriptedClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        self.text_request_count
            .set(self.text_request_count.get() + 1);
        self.text_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted text response".to_string(),
                })
            })
    }

    fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
        let _writer = writer;
        self.byte_request_count
            .set(self.byte_request_count.get() + 1);
        Err(ResourcesError::HttpRequestFailed {
            url: url.to_string(),
            reason: "no scripted byte response".to_string(),
        })
    }
}

fn with_tempdir_filters(
    root: &Utf8Path,
    assertions: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(root.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assertions)
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
    reason = "CLI PHP tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI PHP tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI PHP tests read fixture files"
)]
fn read_file(path: &Utf8Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
