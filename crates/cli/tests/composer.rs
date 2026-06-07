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
use insta::assert_debug_snapshot;
use resources::{ResourceHttpClient, ResourcesError, TargetPlatform};
use state::{Database, ManagedResourceDesiredState, ManagedResourceTrackRecord, PvPaths};

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    client: ScriptedClient,
    exec_calls: RefCell<Vec<ExecCall>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, client: ScriptedClient) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            client,
            exec_calls: RefCell::new(Vec::new()),
        }
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
        self.exec_calls.borrow_mut().push(ExecCall {
            program: program.to_path_buf(),
            args: args.to_vec(),
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
    assert!(output.stderr.is_empty());
    assert!(output.stdout.contains("warning: PV daemon is not running"));
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
    assert!(output.stderr.is_empty());
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
fn composer_install_does_not_record_php_pair_when_composer_install_fails() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php_artifacts = php_pair_artifacts("8.4.8-pv1");
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let manifest = composer_manifest("8.4", &php_artifacts, &[&composer_artifact]);
    prepare_existing_php_pair_releases(&home, "8.4", &php_artifacts)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    );

    let output = run_pv(&["composer:install"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(records.is_empty());
    assert_eq!(environment.text_request_count(), 1);
    assert_eq!(environment.byte_request_count(), 2);
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
fn composer_update_updates_track_two_only() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let old_artifact = composer_fixture_artifact("2.8.0-pv1");
    let new_artifact = composer_fixture_artifact("2.8.1-pv1");
    record_installed_composer(&home, "2", &old_artifact)?;
    prepare_existing_release(&home, "2", &new_artifact)?;
    let manifest = composer_only_manifest(&[&old_artifact, &new_artifact]);
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&manifest),
    );

    let output = run_pv(&["composer:update"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert!(output.stdout.contains("warning: PV daemon is not running"));
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
    assert!(output.stderr.is_empty());
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
fn composer_shim_forwards_help_and_version_flags() -> anyhow::Result<()> {
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

    let outputs = [
        run_pv(&["shim:composer", "--help"], &environment)?,
        run_pv(&["shim:composer", "-h"], &environment)?,
        run_pv(&["shim:composer", "--version"], &environment)?,
        run_pv(&["shim:composer", "-V"], &environment)?,
    ];
    let exec_calls = environment.exec_calls();

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
                program: php_release.join("bin/php").as_std_path().to_path_buf(),
                args: vec![
                    composer_release.join("composer.phar").to_string(),
                    arg.to_string(),
                ],
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
fn composer_shim_uses_cached_manifest_default_without_network() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let php_artifacts = php_pair_artifacts("8.4.8-pv1");
    cache_manifest(&home, &php_only_manifest("8.4", &php_artifacts))?;
    let php_release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let composer_artifact = composer_fixture_artifact("2.8.1-pv1");
    let composer_release = record_installed_composer(&home, "2", &composer_artifact)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["shim:composer", "about"], &environment)?;
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
                "about".to_string(),
            ],
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
    }
}

fn composer_fixture_artifact(version: &str) -> FixtureArtifact {
    FixtureArtifact {
        resource_name: "composer".to_string(),
        version: version.to_string(),
        platform: "any".to_string(),
        executable_path: "composer.phar".to_string(),
        sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
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
    let mut database = Database::open(&pv_paths(home))?;
    database.record_managed_resource_track_installed(
        "composer",
        track,
        &artifact.version,
        &release,
    )?;

    Ok(release)
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

fn php_only_manifest(default_track: &str, artifacts: &PhpPairArtifacts) -> String {
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

fn composer_only_manifest(composer_artifacts: &[&FixtureArtifact]) -> String {
    manifest_with_resources(&[manifest_resource(
        "composer",
        "2",
        vec![manifest_track("2", composer_artifacts.to_vec())],
    )])
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
            format!(
                r#"{{
              "artifact_version": "{}",
              "upstream_version": "{}",
              "pv_build_revision": "1",
              "platform": "{}",
              "url": "https://artifacts.example.test/{}-{}-{}.tar.gz",
              "sha256": "{}",
              "size": {},
              "published_at": "{}"
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
