use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use std::process::{ExitStatus, Output};

#[expect(
    clippy::disallowed_types,
    reason = "release tooling CLI tests execute the pv-release binary"
)]
type StdCommand = std::process::Command;

const STAGING_BASE_URL: &str = "https://artifacts-staging.pv.prvious.dev";

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct InvalidAppReleaseRecordSnapshot {
    name: &'static str,
    status: Option<i32>,
    stderr: String,
}

struct AppManifestFixture {
    tempdir: Utf8TempDir,
    records: Utf8PathBuf,
    output: Utf8PathBuf,
}

#[test]
fn app_release_records_generate_self_update_manifest() -> Result<()> {
    let fixture = AppManifestFixture::new()?;
    fixture.write_record("01-pv-darwin-arm64.json", PV_APP_ARM64)?;
    fixture.write_record("02-pv-darwin-amd64.json", PV_APP_AMD64)?;

    let output = fixture.generate_manifest()?;
    assert_command_success(&output, "generate-app-manifest")?;
    let manifest_json = read_file(&fixture.output)?;

    assert_snapshot!(manifest_json);

    Ok(())
}

#[test]
fn app_release_record_writer_serializes_binary_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let binary = tempdir.path().join("pv");
    let record = tempdir.path().join("records/pv/0.2.0/pv-darwin-arm64.json");
    write_binary_file(&binary, b"pv app bytes")?;

    let output = StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .arg("write-app-release-record")
        .arg("--record")
        .arg(record.as_str())
        .arg("--binary")
        .arg(binary.as_str())
        .arg("--version")
        .arg("0.2.0")
        .arg("--minimum-pv-version")
        .arg("0.1.0")
        .arg("--published-at")
        .arg("2026-06-11T12:00:00Z")
        .arg("--platform")
        .arg("darwin-arm64")
        .arg("--object-key")
        .arg("pv/0.2.0/pv-darwin-arm64")
        .arg("--source-url")
        .arg("https://github.com/prvious/pv/archive/refs/tags/v0.2.0.tar.gz")
        .arg("--source-sha256")
        .arg("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        .arg("--recipe")
        .arg(".github/workflows/app-release.yml")
        .arg("--pv-commit")
        .arg("0123456789abcdef0123456789abcdef01234567")
        .arg("--build-run-id")
        .arg("123456789")
        .output()
        .context("failed to execute pv-release write-app-release-record")?;

    assert_command_success(&output, "write-app-release-record")?;
    let json = read_file(&record)?;
    assert_snapshot!(json);

    Ok(())
}

#[test]
fn app_release_generators_accept_relative_output_file_names() -> Result<()> {
    let fixture = AppManifestFixture::new()?;
    fixture.write_record("01-pv-darwin-arm64.json", PV_APP_ARM64)?;
    fixture.write_record("02-pv-darwin-amd64.json", PV_APP_AMD64)?;

    let manifest_output = StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .current_dir(fixture.tempdir.path())
        .arg("generate-app-manifest")
        .arg("--records")
        .arg(fixture.records.as_str())
        .arg("--output")
        .arg("pv-app-manifest.json")
        .arg("--base-url")
        .arg(STAGING_BASE_URL)
        .output()
        .context("failed to execute pv-release generate-app-manifest")?;
    assert_command_success(&manifest_output, "generate-app-manifest")?;

    let installer_output = StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
        .current_dir(fixture.tempdir.path())
        .arg("generate-app-installer")
        .arg("--records")
        .arg(fixture.records.as_str())
        .arg("--output")
        .arg("install.sh")
        .arg("--base-url")
        .arg(STAGING_BASE_URL)
        .output()
        .context("failed to execute pv-release generate-app-installer")?;
    assert_command_success(&installer_output, "generate-app-installer")?;

    assert!(
        fixture
            .tempdir
            .path()
            .join("pv-app-manifest.json")
            .is_file()
    );
    assert!(fixture.tempdir.path().join("install.sh").is_file());

    Ok(())
}

#[test]
fn app_release_records_reject_invalid_record_metadata() -> Result<()> {
    let cases = [
        InvalidAppReleaseRecordCase {
            name: "invalid_version",
            records: vec![(
                "invalid-version.json",
                PV_APP_ARM64.replace("\"version\": \"0.2.0\"", "\"version\": \"0.02.0\""),
            )],
        },
        InvalidAppReleaseRecordCase {
            name: "invalid_platform",
            records: vec![(
                "invalid-platform.json",
                PV_APP_ARM64.replace(
                    "\"platform\": \"darwin-arm64\"",
                    "\"platform\": \"linux-amd64\"",
                ),
            )],
        },
        InvalidAppReleaseRecordCase {
            name: "invalid_checksum",
            records: vec![(
                "invalid-checksum.json",
                PV_APP_ARM64.replace(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "not-a-sha256",
                ),
            )],
        },
        InvalidAppReleaseRecordCase {
            name: "invalid_size",
            records: vec![(
                "invalid-size.json",
                PV_APP_ARM64.replace("\"size\": 12345678", "\"size\": 0"),
            )],
        },
        InvalidAppReleaseRecordCase {
            name: "duplicate_platform",
            records: vec![
                ("01-pv-darwin-arm64.json", PV_APP_ARM64.to_string()),
                (
                    "02-pv-darwin-arm64.json",
                    PV_APP_AMD64
                        .replace(
                            "\"platform\": \"darwin-amd64\"",
                            "\"platform\": \"darwin-arm64\"",
                        )
                        .replace("pv/0.2.0/pv-darwin-amd64", "pv/0.2.0/pv-darwin-arm64"),
                ),
            ],
        },
        InvalidAppReleaseRecordCase {
            name: "inconsistent_metadata",
            records: vec![
                ("01-pv-darwin-arm64.json", PV_APP_ARM64.to_string()),
                (
                    "02-pv-darwin-amd64.json",
                    PV_APP_AMD64.replace("0.2.0", "0.3.0"),
                ),
            ],
        },
    ];

    let failures = cases
        .into_iter()
        .map(|case| {
            let fixture = AppManifestFixture::new()?;
            for (name, content) in case.records {
                fixture.write_record(name, &content)?;
            }

            let output = fixture.generate_manifest()?;
            Ok(InvalidAppReleaseRecordSnapshot {
                name: case.name,
                status: status_code(output.status),
                stderr: normalize_command_stderr(&output.stderr, fixture.tempdir.path())?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(failures);

    Ok(())
}

struct InvalidAppReleaseRecordCase {
    name: &'static str,
    records: Vec<(&'static str, String)>,
}

impl AppManifestFixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        let records = tempdir.path().join("records");
        let output = tempdir.path().join("pv-app-manifest.json");
        create_dir_all(&records)?;

        Ok(Self {
            tempdir,
            records,
            output,
        })
    }

    fn write_record(&self, name: &str, content: &str) -> Result<()> {
        write_file(&self.records.join(name), content)
    }

    fn generate_manifest(&self) -> Result<Output> {
        StdCommand::new(env!("CARGO_BIN_EXE_pv-release"))
            .arg("generate-app-manifest")
            .arg("--records")
            .arg(self.records.as_str())
            .arg("--output")
            .arg(self.output.as_str())
            .arg("--base-url")
            .arg(STAGING_BASE_URL)
            .output()
            .context("failed to execute pv-release generate-app-manifest")
    }
}

fn assert_command_success(output: &Output, label: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        status_code(output.status),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn status_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

fn normalize_command_stderr(stderr: &[u8], tempdir: &Utf8Path) -> Result<String> {
    let stderr = String::from_utf8(stderr.to_vec())?;
    Ok(stderr.replace(tempdir.as_str(), "<tempdir>"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local fixture metadata records"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local fixture binaries"
)]
fn write_binary_file(path: &Utf8Path, content: &[u8]) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated app manifests"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

const PV_APP_ARM64: &str = r#"{
  "schema_version": 1,
  "channel": "stable",
  "version": "0.2.0",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "platform": "darwin-arm64",
  "object_key": "pv/0.2.0/pv-darwin-arm64",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "size": 12345678,
  "provenance": {
    "source_url": "https://github.com/prvious/pv/archive/refs/tags/v0.2.0.tar.gz",
    "source_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "recipe": ".github/workflows/app-release.yml",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "123456789"
  }
}"#;

const PV_APP_AMD64: &str = r#"{
  "schema_version": 1,
  "channel": "stable",
  "version": "0.2.0",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "platform": "darwin-amd64",
  "object_key": "pv/0.2.0/pv-darwin-amd64",
  "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "size": 12345679,
  "provenance": {
    "source_url": "https://github.com/prvious/pv/archive/refs/tags/v0.2.0.tar.gz",
    "source_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "recipe": ".github/workflows/app-release.yml",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "123456789"
  }
}"#;
