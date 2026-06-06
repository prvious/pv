use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use data_encoding::HEXLOWER;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::assert_debug_snapshot;
use pv_release::ReleaseError;
use pv_release::archive::{
    ArchiveValidation, validate_archive, validate_archive_for_record_file,
    validate_archive_for_record_file_with_smoke_hook,
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tar::{Builder, EntryType, Header};

type ArchiveSummary = (String, String, u64, String, Vec<String>);
type ErrorSummary = (String, String, String);

#[test]
fn archive_validation_accepts_single_root_archive_and_computes_digest() -> Result<()> {
    let tempdir = tempdir()?;
    let archive = tempdir.path().join("redis.tar.gz");
    write_archive(
        &archive,
        &[
            ("redis-7.2.5-pv1/LICENSE", b"license" as &[u8]),
            ("redis-7.2.5-pv1/NOTICE", b"notice" as &[u8]),
            ("redis-7.2.5-pv1/bin/redis-server", b"redis" as &[u8]),
        ],
    )?;

    let validation = validate_archive(&archive, &["LICENSE"], &["NOTICE"])?;
    let record = tempdir.path().join("redis.json");
    write_record(
        &record,
        &release_record_json(validation.sha256(), validation.size()),
    )?;

    assert_debug_snapshot!((
        validation_summary(&validation, tempdir.path()),
        validate_archive_for_record_file(&archive, &record),
    ));
    Ok(())
}

#[test]
fn archive_validation_rejects_empty_multi_root_rootless_and_escape_entries() -> Result<()> {
    let tempdir = tempdir()?;
    let empty = tempdir.path().join("empty.tar.gz");
    write_archive(&empty, &[])?;
    let multi_root = tempdir.path().join("multi-root.tar.gz");
    write_archive(&multi_root, &[("one/LICENSE", b""), ("two/LICENSE", b"")])?;
    let rootless = tempdir.path().join("rootless.tar.gz");
    write_archive(&rootless, &[("LICENSE", b"")])?;
    let escape = tempdir.path().join("escape.tar.gz");
    write_unchecked_path_archive(&escape, "root/../../outside", b"escape")?;
    let current_dir = tempdir.path().join("current-dir.tar.gz");
    write_unchecked_path_archive(&current_dir, "root/./LICENSE", b"license")?;
    let absolute = tempdir.path().join("absolute.tar.gz");
    write_unchecked_path_archive(&absolute, "/root/LICENSE", b"license")?;
    let backslash = tempdir.path().join("backslash.tar.gz");
    write_unchecked_path_archive(&backslash, "root\\LICENSE", b"license")?;

    assert_debug_snapshot!((
        validation_outcome(validate_archive(&empty, &["LICENSE"], &[]), tempdir.path()),
        validation_outcome(
            validate_archive(&multi_root, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(
            validate_archive(&rootless, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(validate_archive(&escape, &["LICENSE"], &[]), tempdir.path()),
        validation_outcome(
            validate_archive(&current_dir, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(
            validate_archive(&absolute, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(
            validate_archive(&backslash, &["LICENSE"], &[]),
            tempdir.path()
        ),
    ));

    Ok(())
}

#[test]
fn archive_validation_rejects_special_entries_and_missing_license_files() -> Result<()> {
    let tempdir = tempdir()?;
    let symlink = tempdir.path().join("symlink.tar.gz");
    write_special_archive(&symlink, EntryType::Symlink)?;
    let hardlink = tempdir.path().join("hardlink.tar.gz");
    write_special_archive(&hardlink, EntryType::Link)?;
    let fifo = tempdir.path().join("fifo.tar.gz");
    write_special_archive(&fifo, EntryType::Fifo)?;
    let missing_license = tempdir.path().join("missing-license.tar.gz");
    write_archive(&missing_license, &[("root/README.md", b"readme")])?;
    let missing_notice = tempdir.path().join("missing-notice.tar.gz");
    write_archive(&missing_notice, &[("root/LICENSE", b"license")])?;
    let directory_license = tempdir.path().join("directory-license.tar.gz");
    write_directory_archive(&directory_license, &["root/", "root/LICENSE/"])?;

    assert_debug_snapshot!((
        validation_outcome(
            validate_archive(&symlink, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(
            validate_archive(&hardlink, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(validate_archive(&fifo, &["LICENSE"], &[]), tempdir.path()),
        validation_outcome(
            validate_archive(&missing_license, &["LICENSE"], &[]),
            tempdir.path()
        ),
        validation_outcome(
            validate_archive(&missing_notice, &["LICENSE"], &["NOTICE"]),
            tempdir.path()
        ),
        validation_outcome(
            validate_archive(&directory_license, &["LICENSE"], &[]),
            tempdir.path()
        ),
    ));

    Ok(())
}

#[test]
fn archive_validation_rejects_blocked_runtime_paths_from_archives() -> Result<()> {
    let tempdir = tempdir()?;
    let archive = tempdir.path().join("redis.tar.gz");
    write_archive(
        &archive,
        &[
            ("redis-7.2.5-pv1/LICENSE", b"license" as &[u8]),
            ("redis-7.2.5-pv1/NOTICE", b"notice" as &[u8]),
            (
                "redis-7.2.5-pv1/bin/redis-server",
                b"load /opt/homebrew/lib/libssl.dylib" as &[u8],
            ),
        ],
    )?;
    let (sha256, size) = archive_digest_and_size(&archive)?;
    let record = tempdir.path().join("redis.json");
    write_record(&record, &release_record_json(&sha256, size))?;

    assert_debug_snapshot!(unit_validation_outcome(
        validate_archive_for_record_file(&archive, &record),
        tempdir.path(),
    ));

    Ok(())
}

#[test]
fn archive_validation_runs_smoke_hook_against_extracted_archive_root() -> Result<()> {
    let tempdir = tempdir()?;
    let archive = tempdir.path().join("redis.tar.gz");
    write_archive(
        &archive,
        &[
            ("redis-7.2.5-pv1/LICENSE", b"license" as &[u8]),
            ("redis-7.2.5-pv1/NOTICE", b"notice" as &[u8]),
            ("redis-7.2.5-pv1/bin/redis-server", b"redis" as &[u8]),
        ],
    )?;
    let (sha256, size) = archive_digest_and_size(&archive)?;
    let record = tempdir.path().join("redis.json");
    write_record(&record, &release_record_json(&sha256, size))?;
    let hook = tempdir.path().join("smoke.sh");
    write_executable(
        &hook,
        "#!/bin/sh\ntest -f \"$1/bin/redis-server\" || exit 43\nexit 42\n",
    )?;

    assert_debug_snapshot!(unit_validation_outcome(
        validate_archive_for_record_file_with_smoke_hook(&archive, &record, Some(&hook)),
        tempdir.path(),
    ));

    Ok(())
}

fn validation_summary(validation: &ArchiveValidation, root: &Utf8Path) -> ArchiveSummary {
    (
        relative_path(validation.archive_path(), root),
        validation.sha256().to_string(),
        validation.size(),
        validation.root().to_string(),
        validation.entries().to_vec(),
    )
}

fn validation_outcome(
    result: pv_release::Result<ArchiveValidation>,
    root: &Utf8Path,
) -> Result<ArchiveSummary, ErrorSummary> {
    result
        .map(|validation| validation_summary(&validation, root))
        .map_err(|error| validation_error_summary(error, root))
}

fn unit_validation_outcome(
    result: pv_release::Result<()>,
    root: &Utf8Path,
) -> Result<(), ErrorSummary> {
    result.map_err(|error| validation_error_summary(error, root))
}

fn validation_error_summary(error: ReleaseError, root: &Utf8Path) -> ErrorSummary {
    match error {
        ReleaseError::InvalidArchive { path, reason } => (
            "InvalidArchive".to_string(),
            relative_path(Utf8Path::new(&path), root),
            reason,
        ),
        ReleaseError::Filesystem { path, reason } => (
            "Filesystem".to_string(),
            relative_path(Utf8Path::new(&path), root),
            reason,
        ),
        ReleaseError::ChecksumMismatch {
            path,
            expected,
            actual,
        } => (
            "ChecksumMismatch".to_string(),
            relative_path(Utf8Path::new(&path), root),
            format!("expected {expected}, got {actual}"),
        ),
        ReleaseError::SizeMismatch {
            path,
            expected,
            actual,
        } => (
            "SizeMismatch".to_string(),
            relative_path(Utf8Path::new(&path), root),
            format!("expected {expected}, got {actual}"),
        ),
        ReleaseError::Relocation { path, reason } => (
            "Relocation".to_string(),
            relative_path(Utf8Path::new(&path), root),
            reason,
        ),
        ReleaseError::SmokeHookFailed { hook, status } => (
            "SmokeHookFailed".to_string(),
            relative_path(Utf8Path::new(&hook), root),
            status,
        ),
        error => ("Other".to_string(), String::new(), error.to_string()),
    }
}

fn relative_path(path: &Utf8Path, root: &Utf8Path) -> String {
    match path.strip_prefix(root) {
        Ok(path) => path.to_string(),
        Err(_error) => path.to_string(),
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create fixture archives directly"
)]
fn write_archive(path: &Utf8Path, entries: &[(&str, &[u8])]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    for (path, content) in entries {
        let mut header = Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *content)?;
    }

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create directory-only fixture archives directly"
)]
fn write_directory_archive(path: &Utf8Path, entries: &[&str]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    for path in entries {
        let mut header = Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(EntryType::Directory);
        header.set_cksum();
        builder.append_data(&mut header, path, &[] as &[u8])?;
    }

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create malformed fixture archives directly"
)]
fn write_unchecked_path_archive(path: &Utf8Path, entry_path: &str, content: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    let entry_path = entry_path.as_bytes();

    header.as_mut_bytes()[..entry_path.len()].copy_from_slice(entry_path);
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, content)?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create special-entry fixture archives directly"
)]
fn write_special_archive(path: &Utf8Path, entry_type: EntryType) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_size(0);
    header.set_entry_type(entry_type);
    header.set_cksum();

    if entry_type.is_symlink() || entry_type.is_hard_link() {
        builder.append_link(&mut header, "root/link", "target")?;
    } else {
        builder.append_data(&mut header, "root/fifo", &[] as &[u8])?;
    }

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local fixture metadata records"
)]
fn write_record(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read local fixture archives to seed matching release records"
)]
fn archive_digest_and_size(path: &Utf8Path) -> Result<(String, u64)> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);

    Ok((HEXLOWER.encode(&hasher.finalize()), bytes.len() as u64))
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create executable smoke hook fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create executable smoke hook fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

fn release_record_json(sha256: &str, size: u64) -> String {
    format!(
        r#"{{
  "resource": "redis",
  "track": "7.2",
  "upstream_version": "7.2.5",
  "pv_build_revision": "pv1",
  "artifact_version": "7.2.5-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/redis/7.2/7.2.5-pv1/darwin-arm64/redis-7.2.5-pv1-darwin-arm64.tar.gz",
  "sha256": "{sha256}",
  "size": {size},
  "published_at": "2026-06-06T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {{
    "source_url": "https://download.redis.io/releases/redis-7.2.5.tar.gz",
    "source_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "recipe": "release/artifacts/recipes/redis/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }}
}}"#,
    )
}
