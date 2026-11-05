use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::support::run_pv;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Answer, Environment, Prompt, PromptKind};
use config::ProjectConfigFile;
use insta::{assert_debug_snapshot, assert_snapshot};
use resources::{ResourceHttpClient, ResourcesError, TargetPlatform};
use state::{
    Database, LinkProjectInput, ManagedResourceDesiredState, ManagedResourceTrackRecord,
    ProjectRecord, PvPaths, RuntimeLifecycleLock, StateError, fs,
};

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    vars: RefCell<BTreeMap<String, OsString>>,
    client: ScriptedClient,
    exec_calls: RefCell<Vec<ExecCall>>,
    terminal: bool,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, client: ScriptedClient) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            vars: RefCell::new(BTreeMap::new()),
            client,
            exec_calls: RefCell::new(Vec::new()),
            terminal: false,
        }
    }

    fn with_terminal(mut self) -> Self {
        self.terminal = true;
        self
    }

    fn with_var(self, key: &str, value: impl Into<OsString>) -> Self {
        self.vars.borrow_mut().insert(key.to_string(), value.into());
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

fn composer_exec_env(home: &Utf8Path, php_track: &str) -> anyhow::Result<Vec<(String, String)>> {
    let paths = pv_paths(home);
    let defaults = resources::php_track_defaults(&paths, php_track)?;

    Ok(vec![
        ("COMPOSER_HOME".to_string(), paths.composer().to_string()),
        (
            "COMPOSER_CACHE_DIR".to_string(),
            paths.composer().join("cache").to_string(),
        ),
        (
            "PATH".to_string(),
            format!("{}:{}", paths.bin(), paths.composer().join("vendor/bin")),
        ),
        ("PHPRC".to_string(), defaults.etc_dir().to_string()),
        (
            "PHP_INI_SCAN_DIR".to_string(),
            defaults.conf_dir().to_string(),
        ),
    ])
}

impl Environment for TestEnvironment {
    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    fn var_os(&self, key: &str) -> Option<OsString> {
        self.vars.borrow().get(key).cloned()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        if cfg!(target_os = "macos") {
            let paths = PvPaths::for_home(
                Utf8Path::from_path(&self.home)
                    .ok_or_else(|| io::Error::other("fixture home is not UTF-8"))?,
            );
            if !matches!(
                RuntimeLifecycleLock::acquire_exclusive(&paths),
                Err(StateError::CoordinationLockHeld { .. })
            ) {
                return Err(io::Error::other(
                    "initialization must hold lifecycle admission",
                ));
            }
        }
        Ok(self.current_dir.borrow().clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        self.terminal
    }

    fn stdout_is_terminal(&self) -> bool {
        self.terminal
    }

    fn stderr_is_terminal(&self) -> bool {
        self.terminal
    }

    fn terminal_width(&self) -> Option<usize> {
        self.terminal.then_some(100)
    }

    fn prompt(&self, prompt: &Prompt<'_>) -> io::Result<Answer> {
        let PromptKind::Confirm { default } = prompt.kind else {
            return Err(io::Error::other("only confirmations are scripted"));
        };

        Ok(Answer::Confirmed(default))
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
        if cfg!(target_os = "macos") {
            // Composer must release initialization admission before invoking PHP.
            let paths = PvPaths::for_home(
                Utf8Path::from_path(&self.home)
                    .ok_or_else(|| io::Error::other("fixture home is not UTF-8"))?,
            );
            let _exclusive =
                RuntimeLifecycleLock::acquire_exclusive(&paths).map_err(io::Error::other)?;
        }
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
        Some(TargetPlatform::DarwinArm64)
    }
}

#[test]
fn composer_install_uses_manifest_default_php_track_without_cached_manifest() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php_artifacts = php_pair_artifacts("8.4.8-pv1");
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let manifest = composer_manifest("8.4", &php_artifacts, &[&composer_artifact]);
    prepare_existing_php_pair_releases(&home, "8.4", &php_artifacts)?;
    prepare_existing_release(&home, "2", &composer_artifact)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    );

    let output = run_pv(&["composer:install"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.contains("error:"));
    assert_eq!(environment.byte_request_count(), 0);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn composer_install_on_a_terminal_reports_the_php_pair() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php_artifacts = php_pair_artifacts("8.4.8-pv1");
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let manifest = composer_manifest("8.4", &php_artifacts, &[&composer_artifact]);
    prepare_existing_php_pair_releases(&home, "8.4", &php_artifacts)?;
    prepare_existing_release(&home, "2", &composer_artifact)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    )
    .with_terminal();

    let output = run_pv(&["composer:install", "--no-color"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.contains("error:"));
    with_tempdir_filters(tempdir.path(), || {
        assert_snapshot!(
            "composer_install_on_a_terminal_reports_the_php_pair",
            output.stdout
        );
        Ok(())
    })?;

    Ok(())
}

#[test]
fn composer_install_prefers_global_php_default_track() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php83_artifacts = php_pair_artifacts("8.3.24-pv1");
    let php84_artifacts = php_pair_artifacts("8.4.8-pv1");
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let manifest = composer_manifest_with_php_tracks(
        &[("8.3", &php83_artifacts), ("8.4", &php84_artifacts)],
        "8.4",
        &[&composer_artifact],
    );
    prepare_existing_php_pair_releases(&home, "8.3", &php83_artifacts)?;
    prepare_existing_release(&home, "2", &composer_artifact)?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.3")?;
    }
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    );

    let output = run_pv(&["composer:install"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.contains("error:"));
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn composer_uninstall_force_prune_queues_removal_intent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    record_installed_composer(&home, "2", &composer_artifact)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["composer:uninstall", "--force", "--prune"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.contains("error:"));
    assert!(records.iter().all(|record| {
        record.desired_state == ManagedResourceDesiredState::Removed
            && record.removal_force
            && record.removal_prune
    }));
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((output, resource_record_snapshots(&records, tempdir.path())?,));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn composer_uninstall_reports_preserved_home_on_both_surfaces() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let terminal_home = tempdir.path().join("terminal-home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    record_installed_composer(&home, "2", &composer_fixture_artifact("2.8.1-pv1"))?;
    record_installed_composer(&terminal_home, "2", &composer_fixture_artifact("2.8.1-pv1"))?;
    let plain_environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());
    let terminal_environment =
        TestEnvironment::new(&terminal_home, &current_dir, ScriptedClient::new()).with_terminal();

    let plain = run_pv(&["composer:uninstall", "--force"], &plain_environment)?;
    let terminal = run_pv(
        &["composer:uninstall", "--force", "--no-color"],
        &terminal_environment,
    )?;

    assert_eq!(plain.exit_code, ExitCode::SUCCESS);
    assert_eq!(terminal.exit_code, ExitCode::SUCCESS);
    with_tempdir_filters(tempdir.path(), || {
        assert_debug_snapshot!((plain, terminal));
        Ok(())
    })?;

    Ok(())
}

#[test]
fn composer_shim_fails_clearly_when_composer_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["shim:composer", "--version"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty());
    assert!(environment.exec_calls().is_empty());
    assert_eq!(environment.text_request_count(), 0);
    assert_eq!(environment.byte_request_count(), 0);
    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn composer_shim_execs_installed_phar_through_php_shim() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php_release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let composer_release = record_installed_composer(&home, "2", &composer_artifact)?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.4")?;
    }
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["shim:composer", "install", "--dry-run"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        exec_calls,
        vec![ExecCall {
            program: php_release.join("bin/php").as_std_path().to_path_buf(),
            args: vec![
                composer_release.join("composer.phar").to_string(),
                "install".to_string(),
                "--dry-run".to_string(),
            ],
            env: composer_exec_env(&home, "8.4")?,
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
fn composer_shim_inherits_project_php_extension_runtime_overlay() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let php_release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let composer_release = record_installed_composer(&home, "2", &composer_artifact)?;
    let composer_phar = composer_release.join("composer.phar");
    fs::write_sensitive_file(
        &php_release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    fs::write_sensitive_file(&php_release.join("lib/php/extensions/redis.so"), "")?;
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

    let output = run_pv(&["shim:composer", "about"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(exec_calls[0].args[0], composer_phar.to_string());
    assert!(exec_calls[0].env.iter().any(|(key, value)| {
        key == "PHP_INI_SCAN_DIR" && value.contains("php-runtimes/8.4+redis/conf.d")
    }));

    Ok(())
}

#[test]
fn composer_shim_sets_pv_owned_env_overlay() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php_release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let composer_release = record_installed_composer(&home, "2", &composer_artifact)?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.record_global_php_default_track("8.4")?;
    }
    let pv_bin = pv_paths(&home).bin().to_string();
    let composer_bin = pv_paths(&home).composer().join("vendor/bin").to_string();
    let defaults = resources::php_track_defaults(&pv_paths(&home), "8.4")?;
    let existing_path = format!("/usr/bin:{pv_bin}:/bin:{composer_bin}");
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new())
        .with_var("PATH", existing_path);

    let output = run_pv(&["shim:composer", "about"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(
        exec_calls,
        vec![ExecCall {
            program: php_release.join("bin/php").as_std_path().to_path_buf(),
            args: vec![
                composer_release.join("composer.phar").to_string(),
                "about".to_string()
            ],
            env: vec![
                (
                    "COMPOSER_HOME".to_string(),
                    pv_paths(&home).composer().to_string()
                ),
                (
                    "COMPOSER_CACHE_DIR".to_string(),
                    pv_paths(&home).composer().join("cache").to_string(),
                ),
                (
                    "PATH".to_string(),
                    format!("{pv_bin}:{composer_bin}:/usr/bin:/bin"),
                ),
                ("PHPRC".to_string(), defaults.etc_dir().to_string()),
                (
                    "PHP_INI_SCAN_DIR".to_string(),
                    defaults.conf_dir().to_string(),
                ),
            ],
        }]
    );

    Ok(())
}

fn managed_resource_records(
    database: &Database,
) -> anyhow::Result<Vec<ManagedResourceTrackRecord>> {
    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .filter(|record| {
            record.resource_name == "composer"
                || record.resource_name == "php"
                || record.resource_name == "frankenphp"
        })
        .collect())
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

#[derive(Debug)]
struct PhpPairArtifacts {
    php: FixtureArtifact,
    frankenphp: FixtureArtifact,
}

fn php_pair_artifacts(version: &str) -> PhpPairArtifacts {
    PhpPairArtifacts {
        php: runtime_fixture_artifact("php", version, "bin/php", TargetPlatform::DarwinArm64),
        frankenphp: runtime_fixture_artifact(
            "frankenphp",
            version,
            "bin/frankenphp",
            TargetPlatform::DarwinArm64,
        ),
    }
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

fn composer_fixture_artifact(version: &str) -> FixtureArtifact {
    FixtureArtifact {
        resource_name: "composer".to_string(),
        version: version.to_string(),
        platform: "any".to_string(),
        executable_path: "composer.phar".to_string(),
        sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        revoked_reason: None,
    }
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
    let release = release_path(home, track, artifact);
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
    point_current_release(home, track, &artifact)?;
    let mut database = Database::open(&pv_paths(home))?;
    database.record_managed_resource_track_installed("php", track, version, &release)?;

    Ok(release)
}

fn record_installed_composer(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
) -> anyhow::Result<Utf8PathBuf> {
    prepare_existing_release(home, track, artifact)?;
    let release = release_path(home, track, artifact);
    point_current_release(home, track, artifact)?;
    let mut database = Database::open(&pv_paths(home))?;
    database.record_managed_resource_track_installed(
        "composer",
        track,
        &artifact.version,
        &release,
    )?;

    Ok(release)
}

fn point_current_release(
    home: &Utf8Path,
    track: &str,
    artifact: &FixtureArtifact,
) -> anyhow::Result<()> {
    let current = pv_paths(home)
        .resources()
        .join(&artifact.resource_name)
        .join(track)
        .join("current");
    fs::symlink_file(
        &Utf8PathBuf::from(format!("releases/{}", artifact.version)),
        &current,
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

fn composer_manifest(
    default_php_track: &str,
    php_artifacts: &PhpPairArtifacts,
    composer_artifacts: &[&FixtureArtifact],
) -> String {
    composer_manifest_with_php_tracks(
        &[(default_php_track, php_artifacts)],
        default_php_track,
        composer_artifacts,
    )
}

fn composer_manifest_with_php_tracks(
    php_tracks: &[(&str, &PhpPairArtifacts)],
    default_php_track: &str,
    composer_artifacts: &[&FixtureArtifact],
) -> String {
    let mut php_track_fixtures = Vec::new();
    let mut frankenphp_track_fixtures = Vec::new();
    for (track, artifacts) in php_tracks {
        php_track_fixtures.push(manifest_track(track, vec![&artifacts.php]));
        frankenphp_track_fixtures.push(manifest_track(track, vec![&artifacts.frankenphp]));
    }

    manifest_with_resources(&[
        manifest_resource("php", default_php_track, php_track_fixtures),
        manifest_resource("frankenphp", default_php_track, frankenphp_track_fixtures),
        manifest_resource(
            "composer",
            "2",
            vec![manifest_track("2", composer_artifacts.to_vec())],
        ),
    ])
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
        "2.8.0-pv1" => "2026-05-26T16:30:00Z",
        "2.8.1-pv1" => "2026-05-27T16:30:00Z",
        "8.3.24-pv1" => "2026-05-27T12:30:00Z",
        "8.4.8-pv1" => "2026-05-26T13:30:00Z",
        _ => "2026-05-27T13:30:00Z",
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
    })?;

    Ok(result.project)
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
    reason = "CLI Composer tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI Composer tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}
