use camino::Utf8Path;
use camino::Utf8PathBuf;
use camino_tempfile::Utf8TempDir;
use data_encoding::HEXLOWER;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use tar::{Archive, EntryType};

#[derive(Debug)]
pub struct ArchiveValidation {
    archive_path: Utf8PathBuf,
    sha256: String,
    size: u64,
    root: String,
    entries: Vec<String>,
}

struct ExtractedArchive {
    _tempdir: Utf8TempDir,
    root: Utf8PathBuf,
}

impl ArchiveValidation {
    pub fn archive_path(&self) -> &Utf8Path {
        &self.archive_path
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }
}

impl ExtractedArchive {
    fn root(&self) -> &Utf8Path {
        &self.root
    }
}

pub fn validate_archive(
    archive_path: &Utf8Path,
    license_files: &[&str],
    notice_files: &[&str],
) -> crate::Result<ArchiveValidation> {
    let (sha256, size) = digest_and_size(archive_path)?;
    let file = open_file(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut roots = BTreeSet::new();
    let mut entries = Vec::new();
    let mut regular_file_entries = BTreeSet::new();

    for entry in archive
        .entries()
        .map_err(|error| invalid_archive(archive_path, error))?
    {
        let entry = entry.map_err(|error| invalid_archive(archive_path, error))?;
        let entry_type = entry.header().entry_type();
        if !matches!(entry_type, EntryType::Regular | EntryType::Directory) {
            return Err(invalid_archive(
                archive_path,
                format!("unsupported entry type `{entry_type:?}`"),
            ));
        }

        let path = archive_entry_path(archive_path, &entry)?;
        let components = archive_path_components(archive_path, &path)?;
        if components.len() < 2 && !entry_type.is_dir() {
            return Err(invalid_archive(
                archive_path,
                format!("entry `{path}` must be under a top-level directory"),
            ));
        }

        let normalized_path = path.trim_end_matches('/').to_string();
        roots.insert(components[0].to_string());
        if entry_type.is_file() {
            regular_file_entries.insert(normalized_path.clone());
        }
        entries.push(normalized_path);
    }

    let root = match roots.len() {
        0 => {
            return Err(invalid_archive(archive_path, "archive is empty"));
        }
        1 => roots
            .into_iter()
            .next()
            .ok_or_else(|| invalid_archive(archive_path, "archive is empty"))?,
        count => {
            return Err(invalid_archive(
                archive_path,
                format!("expected exactly one top-level directory, found {count}"),
            ));
        }
    };

    for required in license_files.iter().chain(notice_files.iter()) {
        let expected = format!("{root}/{required}");
        if !regular_file_entries.contains(&expected) {
            return Err(invalid_archive(
                archive_path,
                format!("missing required metadata file `{required}`"),
            ));
        }
    }

    Ok(ArchiveValidation {
        archive_path: archive_path.to_path_buf(),
        sha256,
        size,
        root,
        entries,
    })
}

pub fn validate_archive_for_record_file(
    archive: &Utf8Path,
    record: &Utf8Path,
) -> crate::Result<()> {
    validate_archive_for_record_file_with_smoke_hook(archive, record, None)
}

pub fn validate_archive_for_record_file_with_smoke_hook(
    archive: &Utf8Path,
    record: &Utf8Path,
    smoke_hook: Option<&Utf8Path>,
) -> crate::Result<()> {
    let json = read_to_string(record)?;
    let record = crate::record::ReleaseRecord::from_json(record, &json)?;
    let license_files = record
        .license_files()
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let notice_files = record
        .notice_files()
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let validation = validate_archive(archive, &license_files, &notice_files)?;
    record.verify_archive(&validation)?;
    if let Some(smoke_hook) = smoke_hook {
        let extracted = extract_archive_for_smoke(archive, validation.root())?;
        crate::smoke::run_smoke_hook(smoke_hook, extracted.root())?;
    }

    Ok(())
}

fn extract_archive_for_smoke(
    archive_path: &Utf8Path,
    expected_root: &str,
) -> crate::Result<ExtractedArchive> {
    let tempdir = Utf8TempDir::new().map_err(|error| filesystem_error(archive_path, error))?;
    let file = open_file(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|error| invalid_archive(archive_path, error))?
    {
        let mut entry = entry.map_err(|error| invalid_archive(archive_path, error))?;
        let entry_type = entry.header().entry_type();
        if !matches!(entry_type, EntryType::Regular | EntryType::Directory) {
            return Err(invalid_archive(
                archive_path,
                format!("unsupported entry type `{entry_type:?}`"),
            ));
        }

        let path = archive_entry_path(archive_path, &entry)?;
        let components = archive_path_components(archive_path, &path)?;
        if components[0] != expected_root {
            return Err(invalid_archive(
                archive_path,
                format!("entry `{path}` is outside expected root `{expected_root}`"),
            ));
        }

        let normalized_path = path.trim_end_matches('/');
        let output_path = tempdir.path().join(normalized_path);
        if entry_type.is_dir() {
            create_dir_all(&output_path)?;
        } else {
            if let Some(parent) = output_path.parent() {
                create_dir_all(parent)?;
            }
            entry
                .unpack(&output_path)
                .map_err(|error| invalid_archive(archive_path, error))?;
        }
    }

    let root = tempdir.path().join(expected_root);
    if !root.is_dir() {
        return Err(invalid_archive(
            archive_path,
            format!("expected archive root `{expected_root}` was not extracted"),
        ));
    }

    Ok(ExtractedArchive {
        _tempdir: tempdir,
        root,
    })
}

fn digest_and_size(path: &Utf8Path) -> crate::Result<(String, u64)> {
    let mut file = open_file(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0;
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| filesystem_error(path, error))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    Ok((HEXLOWER.encode(&digest), size))
}

fn archive_entry_path<R: Read>(
    archive_path: &Utf8Path,
    entry: &tar::Entry<'_, R>,
) -> crate::Result<String> {
    let path = entry.path_bytes();
    let path = std::str::from_utf8(&path).map_err(|error| invalid_archive(archive_path, error))?;

    Ok(path.to_string())
}

fn archive_path_components<'a>(
    archive_path: &Utf8Path,
    path: &'a str,
) -> crate::Result<Vec<&'a str>> {
    if path.is_empty() {
        return Err(invalid_archive(archive_path, "entry path is empty"));
    }
    if path.starts_with('/') {
        return Err(invalid_archive(
            archive_path,
            format!("absolute entry path `{path}`"),
        ));
    }
    if path.contains('\\') {
        return Err(invalid_archive(
            archive_path,
            format!("backslash entry path `{path}`"),
        ));
    }

    let components = path.trim_end_matches('/').split('/').collect::<Vec<_>>();
    if components.is_empty() || components.iter().any(|component| component.is_empty()) {
        return Err(invalid_archive(
            archive_path,
            format!("empty path component in `{path}`"),
        ));
    }
    if components
        .iter()
        .any(|component| matches!(*component, "." | ".."))
    {
        return Err(invalid_archive(
            archive_path,
            format!("unsafe entry path `{path}`"),
        ));
    }

    Ok(components)
}

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns direct archive file reads for local artifact validation"
)]
fn open_file(path: &Utf8Path) -> crate::Result<std::fs::File> {
    std::fs::File::open(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads repository-local release metadata records"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling creates temporary archive extraction directories for smoke hooks"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

fn invalid_archive(path: &Utf8Path, reason: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::InvalidArchive {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
