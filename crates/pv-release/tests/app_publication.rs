use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use data_encoding::HEXLOWER;
use insta::{assert_debug_snapshot, assert_snapshot};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::process::Output;

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests execute the pv-release CLI fixture directly"
)]
type StdCommand = std::process::Command;

const APP_PUBLICATION_BASE_URL: &str = "https://downloads.prvious.test";

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct CommandFailure {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[test]
fn stage_app_publication_writes_app_layout_and_upload_plan() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.write_managed_resource_sentinels()?;

    let expected_plan = expected_publication_plan()?;
    assert_snapshot!(expected_plan);

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if !output.status.success() {
        assert_debug_snapshot!(command_failure(&output, fixture.root()));
        bail!("stage-app-publication failed before writing the app publication plan");
    }

    let stage = fixture.stage();
    for expected_path in [
        "pv/0.2.0/pv-darwin-arm64",
        "pv/0.2.0/pv-darwin-amd64",
        "pv/0.2.0/pv-helper-1.0.0-darwin-arm64",
        "pv/0.2.0/pv-helper-1.0.0-darwin-amd64",
        "pv/records/0.2.0/1.0.0/pv-darwin-arm64.json",
        "pv/records/0.2.0/1.0.0/pv-darwin-amd64.json",
        "pv/manifests/runs/987654321/pv-app-manifest.json",
        "pv/manifests/runs/987654321/install.sh",
        "pv-app-manifest.json",
        "install.sh",
    ] {
        assert!(path_exists(&stage.join(expected_path)));
    }

    for (managed_resource_key, expected_content) in managed_resource_sentinels() {
        assert_eq!(
            read_file(&stage.join(managed_resource_key))?,
            expected_content
        );
    }

    assert_eq!(
        read_file(&stage.join("publication-plan.json"))?,
        format!("{expected_plan}\n")
    );
    assert_eq!(
        read_file(&stage.join("pv-app-manifest.json"))?,
        app_manifest_json()?
    );
    assert_eq!(
        read_file(&stage.join("pv/manifests/runs/987654321/pv-app-manifest.json"))?,
        app_manifest_json()?
    );
    assert_snapshot!(installer_entrypoint_summary(&read_file(&stage.join("install.sh"))?), @r#"
    version_present=true
    base_url_present=true
    arm64_url_present=true
    amd64_url_present=true
    example_invalid_absent=true
    "#);

    Ok(())
}

#[test]
fn stage_app_publication_rejects_managed_resource_object_key_before_write() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.write_app_record(
        "darwin-amd64",
        "resources/pv/0.2.0/pv-darwin-amd64",
        b"pv amd64",
        "987654321",
    )?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a Managed Resource object key");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_duplicate_object_key_before_write() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.write_app_record(
        "darwin-amd64",
        "pv/0.2.0/pv-darwin-arm64",
        b"pv amd64",
        "987654321",
    )?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted duplicate app object keys");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_record_from_different_source_run() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.rewrite_record_build_run_id("pv-darwin-amd64.json", "123456789")?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted records from a different source run");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_non_canonical_app_object_key() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_app_record(
        "darwin-arm64",
        "pv/0.2.0/not-the-darwin-arm64-binary",
        b"pv arm64",
        "987654321",
    )?;
    fixture.write_app_record(
        "darwin-amd64",
        "pv/0.2.0/pv-darwin-amd64",
        b"pv amd64",
        "987654321",
    )?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a non-canonical app object key");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_uses_exact_object_key_paths_for_binaries() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    write_bytes(
        &fixture
            .source_binaries
            .join("unrelated/pv/0.2.0/pv-darwin-arm64"),
        b"wrong duplicate arm64",
    )?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if !output.status.success() {
        assert_debug_snapshot!(command_failure(&output, fixture.root()));
        bail!("stage-app-publication should ignore unrelated duplicate basenames");
    }

    assert_eq!(
        read_file(&fixture.stage.join("pv/0.2.0/pv-darwin-arm64"))?,
        "pv arm64"
    );

    Ok(())
}

#[test]
fn stage_app_publication_rejects_corrupt_source_binary_before_write() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    write_bytes(
        &fixture.source_binaries.join("pv/0.2.0/pv-darwin-amd64"),
        b"corrupt amd64",
    )?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a binary that no longer matches its record");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_corrupt_helper_before_write() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    write_bytes(
        &fixture
            .source_binaries
            .join("pv/0.2.0/pv-helper-1.0.0-darwin-amd64"),
        b"corrupt helper",
    )?;

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a helper that no longer matches its record");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_non_newer_stable_manifest_candidate() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json_with_version("0.3.0")?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a candidate older than current stable");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_migrates_newer_candidate_from_schema_one_manifest() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(
        &current_manifest,
        "{\n  \"schema_version\": 1,\n  \"version\": \"0.1.0\"\n}\n",
    )?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if !output.status.success() {
        assert_debug_snapshot!(command_failure(&output, fixture.root()));
        bail!("stage-app-publication rejected the one-time schema-1 migration");
    }

    assert!(path_exists(&fixture.stage.join("publication-plan.json")));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_candidate_older_than_current_stable_installer() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_installer = fixture.root().join("current-install.sh");
    write_file(
        &current_installer,
        &app_installer_script_with_version("0.3.0"),
    )?;

    let output = run_stage_app_publication_with_current_installer(&fixture, &current_installer)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a candidate older than the current stable installer");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_existing_helper_version_with_changed_bytes() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(
        &current_manifest,
        &app_manifest_json_with_version("0.1.0")?.replace(
            &sha256(b"pv helper"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
    )?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication changed bytes for an existing helper version");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_helper_version_regression() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(
        &current_manifest,
        &app_manifest_json_with_version("0.1.0")?
            .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\""),
    )?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication regressed the helper version");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_allows_same_version_current_stable_for_retry() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json()?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if !output.status.success() {
        assert_debug_snapshot!(command_failure(&output, fixture.root()));
        bail!("stage-app-publication should allow same-version stable reruns");
    }

    assert!(path_exists(&fixture.stage.join("publication-plan.json")));

    Ok(())
}

#[test]
fn stage_app_publication_publishes_new_helper_for_current_app() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_app_record_with_helper(
        "darwin-arm64",
        "pv/0.2.0/pv-darwin-arm64",
        b"pv arm64",
        "987654321",
        "1.1.0",
        1,
        b"pv helper 1.1.0",
    )?;
    fixture.write_app_record_with_helper(
        "darwin-amd64",
        "pv/0.2.0/pv-darwin-amd64",
        b"pv amd64",
        "987654321",
        "1.1.0",
        1,
        b"pv helper 1.1.0",
    )?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json()?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if !output.status.success() {
        assert_debug_snapshot!(command_failure(&output, fixture.root()));
        bail!("stage-app-publication rejected a valid helper-only release");
    }

    let manifest = read_file(&fixture.stage.join("pv-app-manifest.json"))?;
    let plan = read_file(&fixture.stage.join("publication-plan.json"))?;
    let installer = read_file(&fixture.stage.join("install.sh"))?;
    assert_snapshot!(format!(
        "manifest:\n{manifest}plan:\n{plan}installer:\n{}",
        helper_only_installer_summary(&installer)
    ));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_equal_version_platform_removal() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_app_record(
        "darwin-arm64",
        "pv/0.2.0/pv-darwin-arm64",
        b"pv arm64",
        "987654321",
    )?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json()?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication removed a platform from an equal-version manifest");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_equal_version_minimum_pv_change() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(
        &current_manifest,
        &app_manifest_json()?.replace(
            "\"minimum_pv_version\": \"0.1.0\"",
            "\"minimum_pv_version\": \"0.1.1\"",
        ),
    )?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication changed equal-version minimum PV eligibility");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_equal_version_base_url_change() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(
        &current_manifest,
        &app_manifest_json()?.replace(APP_PUBLICATION_BASE_URL, "https://old.prvious.test"),
    )?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication changed equal-version stable asset URLs");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_equal_version_app_asset_change() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.write_app_record(
        "darwin-arm64",
        "pv/0.2.0/pv-darwin-arm64",
        b"changed pv arm64",
        "987654321",
    )?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json()?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication changed an equal-version app asset");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_equal_version_helper_protocol_change() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_app_record_with_helper(
        "darwin-arm64",
        "pv/0.2.0/pv-darwin-arm64",
        b"pv arm64",
        "987654321",
        "1.1.0",
        2,
        b"pv helper 1.1.0 arm64",
    )?;
    fixture.write_app_record_with_helper(
        "darwin-amd64",
        "pv/0.2.0/pv-darwin-amd64",
        b"pv amd64",
        "987654321",
        "1.1.0",
        2,
        b"pv helper 1.1.0 amd64",
    )?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json()?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication changed the helper protocol without a new app version");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_empty_candidates_when_current_stable_is_provided() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    let current_manifest = fixture.root().join("current-pv-app-manifest.json");
    write_file(&current_manifest, &app_manifest_json()?)?;

    let output = run_stage_app_publication_with_current_manifest(&fixture, &current_manifest)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted empty candidates with current stable metadata");
    }

    assert_debug_snapshot!(command_failure(&output, fixture.root()));
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn app_manifest_fixture_uses_requested_version_in_asset_urls() -> Result<()> {
    assert_snapshot!(app_manifest_json_with_version("0.3.0")?);

    Ok(())
}

struct AppPublicationFixture {
    tempdir: camino_tempfile::Utf8TempDir,
    source_binaries: Utf8PathBuf,
    candidate_records: Utf8PathBuf,
    stage: Utf8PathBuf,
}

impl AppPublicationFixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        let source_binaries = tempdir.path().join("source-binaries");
        let candidate_records = tempdir.path().join("candidate-records");
        let stage = tempdir.path().join("stage");

        create_dir_all(&source_binaries)?;
        create_dir_all(&candidate_records)?;

        Ok(Self {
            tempdir,
            source_binaries,
            candidate_records,
            stage,
        })
    }

    fn root(&self) -> &Utf8Path {
        self.tempdir.path()
    }

    fn stage(&self) -> &Utf8Path {
        &self.stage
    }

    fn write_valid_inputs(&self) -> Result<()> {
        self.write_app_record(
            "darwin-arm64",
            "pv/0.2.0/pv-darwin-arm64",
            b"pv arm64",
            "987654321",
        )?;
        self.write_app_record(
            "darwin-amd64",
            "pv/0.2.0/pv-darwin-amd64",
            b"pv amd64",
            "987654321",
        )?;

        Ok(())
    }

    fn write_app_record(
        &self,
        platform: &str,
        object_key: &str,
        binary: &[u8],
        build_run_id: &str,
    ) -> Result<()> {
        self.write_app_record_with_helper(
            platform,
            object_key,
            binary,
            build_run_id,
            "1.0.0",
            1,
            b"pv helper",
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "publication fixtures keep app and helper release identities explicit"
    )]
    fn write_app_record_with_helper(
        &self,
        platform: &str,
        object_key: &str,
        binary: &[u8],
        build_run_id: &str,
        helper_version: &str,
        helper_protocol_version: u32,
        helper_binary: &[u8],
    ) -> Result<()> {
        let binary_path = self.source_binaries.join(object_key);
        write_bytes(&binary_path, binary)?;
        let binary_sha256 = sha256(binary);
        let size = binary.len();
        let helper_object_key = format!("pv/0.2.0/pv-helper-{helper_version}-{platform}");
        write_bytes(
            &self.source_binaries.join(&helper_object_key),
            helper_binary,
        )?;
        let record_name = object_key
            .rsplit('/')
            .next()
            .ok_or_else(|| anyhow::anyhow!("object key must include a file name"))?;
        let record = json!({
            "schema_version": 2,
            "channel": "stable",
            "version": "0.2.0",
            "minimum_pv_version": "0.1.0",
            "published_at": "2026-06-13T15:00:00Z",
            "platform": platform,
            "object_key": object_key,
            "sha256": binary_sha256,
            "size": size,
            "helper": {
                "version": helper_version,
                "protocol_version": helper_protocol_version,
                "object_key": helper_object_key,
                "sha256": sha256(helper_binary),
                "size": helper_binary.len(),
            },
            "provenance": {
                "source_url": "https://github.com/prvious/pv/actions/runs/987654321",
                "source_sha256": binary_sha256,
                "recipe": ".github/workflows/app-release.yml",
                "pv_commit": "0123456789abcdef0123456789abcdef01234567",
                "build_run_id": build_run_id
            }
        });
        let record_json = serde_json::to_string_pretty(&record)
            .context("failed to serialize app release record fixture")?;
        write_file(
            &self.candidate_records.join(format!("{record_name}.json")),
            &format!("{record_json}\n"),
        )
    }

    fn rewrite_record_build_run_id(&self, record_name: &str, build_run_id: &str) -> Result<()> {
        let record_path = self.candidate_records.join(record_name);
        let record = read_file(&record_path)?;
        write_file(
            &record_path,
            &record.replace(
                "\"build_run_id\": \"987654321\"",
                &format!("\"build_run_id\": \"{build_run_id}\""),
            ),
        )
    }

    fn write_managed_resource_sentinels(&self) -> Result<()> {
        for (managed_resource_key, content) in managed_resource_sentinels() {
            write_file(&self.stage.join(managed_resource_key), content)?;
        }

        Ok(())
    }
}

fn run_stage_app_publication(fixture: &AppPublicationFixture) -> Result<Output> {
    run_stage_app_publication_with_extra_args(fixture, [])
}

fn run_stage_app_publication_with_current_manifest(
    fixture: &AppPublicationFixture,
    current_manifest: &Utf8Path,
) -> Result<Output> {
    run_stage_app_publication_with_extra_args(
        fixture,
        ["--current-app-manifest", current_manifest.as_str()],
    )
}

fn run_stage_app_publication_with_current_installer(
    fixture: &AppPublicationFixture,
    current_installer: &Utf8Path,
) -> Result<Output> {
    run_stage_app_publication_with_extra_args(
        fixture,
        ["--current-app-installer", current_installer.as_str()],
    )
}

fn run_stage_app_publication_with_extra_args<'a>(
    fixture: &AppPublicationFixture,
    extra_args: impl IntoIterator<Item = &'a str>,
) -> Result<Output> {
    StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .arg("stage-app-publication")
        .arg("--source-binaries")
        .arg(&fixture.source_binaries)
        .arg("--candidate-records")
        .arg(&fixture.candidate_records)
        .arg("--stage")
        .arg(&fixture.stage)
        .arg("--source-run-id")
        .arg("987654321")
        .arg("--base-url")
        .arg(APP_PUBLICATION_BASE_URL)
        .args(extra_args)
        .output()
        .context("failed to execute pv-release stage-app-publication")
}

fn assert_stage_app_publication_available(output: &Output) -> Result<()> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("unrecognized subcommand 'stage-app-publication'")
    {
        bail!(
            "TODO #86 has not implemented `pv-release stage-app-publication` yet.\n{}",
            stderr
        );
    }

    Ok(())
}

fn command_failure(output: &Output, root: &Utf8Path) -> CommandFailure {
    CommandFailure {
        status_code: output.status.code(),
        stdout: normalize_output(&String::from_utf8_lossy(&output.stdout), root),
        stderr: normalize_output(&String::from_utf8_lossy(&output.stderr), root),
    }
}

fn normalize_output(output: &str, root: &Utf8Path) -> String {
    output.replace(root.as_str(), "<tempdir>")
}

fn expected_publication_plan() -> Result<String> {
    serde_json::to_string_pretty(&json!({
        "immutable_uploads": [
            {
                "local_path": "pv/0.2.0/pv-darwin-amd64",
                "object_key": "pv/0.2.0/pv-darwin-amd64"
            },
            {
                "local_path": "pv/0.2.0/pv-helper-1.0.0-darwin-amd64",
                "object_key": "pv/0.2.0/pv-helper-1.0.0-darwin-amd64"
            },
            {
                "local_path": "pv/records/0.2.0/1.0.0/pv-darwin-amd64.json",
                "object_key": "pv/records/0.2.0/1.0.0/pv-darwin-amd64.json"
            },
            {
                "local_path": "pv/0.2.0/pv-darwin-arm64",
                "object_key": "pv/0.2.0/pv-darwin-arm64"
            },
            {
                "local_path": "pv/0.2.0/pv-helper-1.0.0-darwin-arm64",
                "object_key": "pv/0.2.0/pv-helper-1.0.0-darwin-arm64"
            },
            {
                "local_path": "pv/records/0.2.0/1.0.0/pv-darwin-arm64.json",
                "object_key": "pv/records/0.2.0/1.0.0/pv-darwin-arm64.json"
            },
            {
                "local_path": "pv/manifests/runs/987654321/pv-app-manifest.json",
                "object_key": "pv/manifests/runs/987654321/pv-app-manifest.json"
            },
            {
                "local_path": "pv/manifests/runs/987654321/install.sh",
                "object_key": "pv/manifests/runs/987654321/install.sh"
            }
        ],
        "stable_app_manifest": {
            "local_path": "pv-app-manifest.json",
            "object_key": "pv-app-manifest.json"
        },
        "stable_installer": {
            "local_path": "install.sh",
            "object_key": "install.sh"
        }
    }))
    .context("failed to serialize expected app publication plan")
}

fn app_manifest_json() -> Result<String> {
    app_manifest_json_with_version("0.2.0")
}

fn app_manifest_json_with_version(version: &str) -> Result<String> {
    Ok(format!(
        r#"{{
  "schema_version": 2,
  "channel": "stable",
  "version": "{version}",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-13T15:00:00Z",
  "assets": [
    {{
      "platform": "darwin-amd64",
      "url": "{APP_PUBLICATION_BASE_URL}/pv/{version}/pv-darwin-amd64",
      "sha256": "{}",
      "size": 8,
      "helper": {{
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "{APP_PUBLICATION_BASE_URL}/pv/{version}/pv-helper-1.0.0-darwin-amd64",
        "sha256": "{}",
        "size": 9
      }}
    }},
    {{
      "platform": "darwin-arm64",
      "url": "{APP_PUBLICATION_BASE_URL}/pv/{version}/pv-darwin-arm64",
      "sha256": "{}",
      "size": 8,
      "helper": {{
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "{APP_PUBLICATION_BASE_URL}/pv/{version}/pv-helper-1.0.0-darwin-arm64",
        "sha256": "{}",
        "size": 9
      }}
    }}
  ]
}}"#,
        sha256(b"pv amd64"),
        sha256(b"pv helper"),
        sha256(b"pv arm64"),
        sha256(b"pv helper"),
    ))
}

fn app_installer_script_with_version(version: &str) -> String {
    format!("#!/usr/bin/env bash\nPV_VERSION='{version}'\n")
}

fn installer_entrypoint_summary(installer: &str) -> String {
    format!(
        "\
version_present={}
base_url_present={}
arm64_url_present={}
amd64_url_present={}
example_invalid_absent={}",
        installer.contains("PV_VERSION='0.2.0'"),
        installer.contains(APP_PUBLICATION_BASE_URL),
        installer.contains(&format!(
            "{APP_PUBLICATION_BASE_URL}/pv/0.2.0/pv-darwin-arm64"
        )),
        installer.contains(&format!(
            "{APP_PUBLICATION_BASE_URL}/pv/0.2.0/pv-darwin-amd64"
        )),
        !installer.contains("example.invalid"),
    )
}

fn helper_only_installer_summary(installer: &str) -> String {
    format!(
        "version_present={}\nminimum_version_present={}\nhelper_version_present={}\narm64_helper_url_present={}\namd64_helper_url_present={}\nold_helper_url_absent={}",
        installer.contains("PV_VERSION='0.2.0'"),
        installer.contains("PV_MINIMUM_PV_VERSION='0.1.0'"),
        installer.contains("PV_HELPER_VERSION='1.1.0'"),
        installer.contains(&format!(
            "{APP_PUBLICATION_BASE_URL}/pv/0.2.0/pv-helper-1.1.0-darwin-arm64"
        )),
        installer.contains(&format!(
            "{APP_PUBLICATION_BASE_URL}/pv/0.2.0/pv-helper-1.1.0-darwin-amd64"
        )),
        !installer.contains("pv-helper-1.0.0"),
    )
}

fn managed_resource_sentinels() -> [(&'static str, &'static str); 4] {
    [
        ("manifest.json", "managed resource manifest\n"),
        (
            "resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz",
            "managed resource archive\n",
        ),
        (
            "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json",
            "managed resource record\n",
        ),
        (
            "revocations/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json",
            "managed resource revocation\n",
        ),
    ]
}

fn sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    HEXLOWER.encode(&hasher.finalize())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local app publication fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local app publication fixtures"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local app publication binary fixtures"
)]
fn write_bytes(path: &Utf8Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated app publication outputs"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}
