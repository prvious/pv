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

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct AppPublicationCollisionContract {
    rejected_object_key: &'static str,
    expected_error: &'static str,
    must_not_write_managed_resource_keys: Vec<&'static str>,
}

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
        "pv/records/0.2.0/pv-darwin-arm64.json",
        "pv/records/0.2.0/pv-darwin-amd64.json",
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

    Ok(())
}

#[test]
fn stage_app_publication_rejects_managed_resource_object_key_before_write() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.write_app_record(
        "darwin-amd64",
        "pv-darwin-amd64",
        "resources/pv/0.2.0/pv-darwin-amd64",
        b"pv amd64",
    )?;

    let expected_error = "app publication object key is reserved for Managed Resource publication";
    assert_debug_snapshot!(AppPublicationCollisionContract {
        rejected_object_key: "resources/pv/0.2.0/pv-darwin-amd64",
        expected_error,
        must_not_write_managed_resource_keys: managed_resource_keys(),
    });

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted a Managed Resource object key");
    }

    assert!(
        command_failure(&output, fixture.root())
            .stderr
            .contains(expected_error)
    );
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

#[test]
fn stage_app_publication_rejects_duplicate_object_key_before_write() -> Result<()> {
    let fixture = AppPublicationFixture::new()?;
    fixture.write_valid_inputs()?;
    fixture.write_app_record(
        "darwin-amd64",
        "pv-darwin-amd64",
        "pv/0.2.0/pv-darwin-arm64",
        b"pv amd64",
    )?;

    let expected_error = "app publication object key collides between app binary records";
    assert_debug_snapshot!(AppPublicationCollisionContract {
        rejected_object_key: "pv/0.2.0/pv-darwin-arm64",
        expected_error,
        must_not_write_managed_resource_keys: managed_resource_keys(),
    });

    let output = run_stage_app_publication(&fixture)?;
    assert_stage_app_publication_available(&output)?;
    if output.status.success() {
        bail!("stage-app-publication accepted duplicate app object keys");
    }

    assert!(
        command_failure(&output, fixture.root())
            .stderr
            .contains(expected_error)
    );
    assert!(!path_exists(fixture.stage()));

    Ok(())
}

struct AppPublicationFixture {
    tempdir: camino_tempfile::Utf8TempDir,
    source_binaries: Utf8PathBuf,
    candidate_records: Utf8PathBuf,
    app_manifest: Utf8PathBuf,
    installer: Utf8PathBuf,
    stage: Utf8PathBuf,
}

impl AppPublicationFixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        let source_binaries = tempdir.path().join("source-binaries");
        let candidate_records = tempdir.path().join("candidate-records");
        let app_manifest = tempdir.path().join("pv-app-manifest.json");
        let installer = tempdir.path().join("install.sh");
        let stage = tempdir.path().join("stage");

        create_dir_all(&source_binaries)?;
        create_dir_all(&candidate_records)?;

        Ok(Self {
            tempdir,
            source_binaries,
            candidate_records,
            app_manifest,
            installer,
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
            "pv-darwin-arm64",
            "pv/0.2.0/pv-darwin-arm64",
            b"pv arm64",
        )?;
        self.write_app_record(
            "darwin-amd64",
            "pv-darwin-amd64",
            "pv/0.2.0/pv-darwin-amd64",
            b"pv amd64",
        )?;
        write_file(&self.app_manifest, &app_manifest_json()?)?;
        write_file(&self.installer, "#!/usr/bin/env bash\nset -euo pipefail\n")?;

        Ok(())
    }

    fn write_app_record(
        &self,
        platform: &str,
        binary_name: &str,
        object_key: &str,
        binary: &[u8],
    ) -> Result<()> {
        let binary_path = self.source_binaries.join(binary_name);
        write_bytes(&binary_path, binary)?;
        let sha256 = sha256(binary);
        let size = binary.len();
        let record = json!({
            "schema_version": 1,
            "channel": "stable",
            "version": "0.2.0",
            "minimum_pv_version": "0.1.0",
            "published_at": "2026-06-13T15:00:00Z",
            "platform": platform,
            "object_key": object_key,
            "sha256": sha256,
            "size": size,
            "provenance": {
                "source_url": "https://github.com/prvious/pv/actions/runs/987654321",
                "source_sha256": sha256,
                "recipe": ".github/workflows/app-release.yml",
                "pv_commit": "0123456789abcdef0123456789abcdef01234567",
                "build_run_id": "987654321"
            }
        });
        let record_json = serde_json::to_string_pretty(&record)
            .context("failed to serialize app release record fixture")?;
        write_file(
            &self.candidate_records.join(format!("{binary_name}.json")),
            &format!("{record_json}\n"),
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
    StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .arg("stage-app-publication")
        .arg("--source-binaries")
        .arg(&fixture.source_binaries)
        .arg("--candidate-records")
        .arg(&fixture.candidate_records)
        .arg("--app-manifest")
        .arg(&fixture.app_manifest)
        .arg("--installer")
        .arg(&fixture.installer)
        .arg("--stage")
        .arg(&fixture.stage)
        .arg("--source-run-id")
        .arg("987654321")
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
                "local_path": "pv/records/0.2.0/pv-darwin-amd64.json",
                "object_key": "pv/records/0.2.0/pv-darwin-amd64.json"
            },
            {
                "local_path": "pv/0.2.0/pv-darwin-arm64",
                "object_key": "pv/0.2.0/pv-darwin-arm64"
            },
            {
                "local_path": "pv/records/0.2.0/pv-darwin-arm64.json",
                "object_key": "pv/records/0.2.0/pv-darwin-arm64.json"
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
    let manifest = json!({
        "schema_version": 1,
        "channel": "stable",
        "version": "0.2.0",
        "minimum_pv_version": "0.1.0",
        "published_at": "2026-06-13T15:00:00Z",
        "assets": [
            {
                "platform": "darwin-arm64",
                "url": "https://artifacts-staging.pv.prvious.dev/pv/0.2.0/pv-darwin-arm64",
                "sha256": sha256(b"pv arm64"),
                "size": 8
            },
            {
                "platform": "darwin-amd64",
                "url": "https://artifacts-staging.pv.prvious.dev/pv/0.2.0/pv-darwin-amd64",
                "sha256": sha256(b"pv amd64"),
                "size": 8
            }
        ]
    });
    let manifest = serde_json::to_string_pretty(&manifest)
        .context("failed to serialize app manifest fixture")?;
    Ok(format!("{manifest}\n"))
}

fn managed_resource_keys() -> Vec<&'static str> {
    vec![
        "manifest.json",
        "resources/...",
        "records/...",
        "revocations/...",
    ]
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
