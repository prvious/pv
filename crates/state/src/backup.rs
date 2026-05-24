use camino::{Utf8Path, Utf8PathBuf};

use crate::{PvPaths, StateError, fs};

const MIGRATION_BACKUP_RETENTION: usize = 5;

pub(crate) fn database(
    paths: &PvPaths,
    create_backup: impl FnOnce(&Utf8Path) -> Result<(), StateError>,
) -> Result<(), StateError> {
    let backup_path = unique_backup_path(paths)?;

    create_backup(&backup_path)?;
    fs::secure_sensitive_file(&backup_path)?;
    prune_migration_backups(paths)?;

    Ok(())
}

fn unique_backup_path(paths: &PvPaths) -> Result<Utf8PathBuf, StateError> {
    let timestamp = backup_timestamp()?;
    let next_suffix = next_backup_suffix(paths, &timestamp)?;

    for suffix in next_suffix..=999 {
        let candidate = backup_path(paths, &timestamp, suffix);

        if !fs::path_exists(&candidate) {
            return Ok(candidate);
        }
    }

    Err(StateError::BackupNameExhausted {
        path: paths.root().to_path_buf(),
    })
}

fn next_backup_suffix(paths: &PvPaths, timestamp: &str) -> Result<usize, StateError> {
    let mut next_suffix = 0;

    for backup in migration_backups(paths)? {
        if let Some((backup_timestamp, suffix)) = migration_backup_parts(&backup)
            && backup_timestamp == timestamp
        {
            next_suffix = next_suffix.max(suffix.saturating_add(1));
        }
    }

    Ok(next_suffix)
}

fn backup_path(paths: &PvPaths, timestamp: &str, suffix: usize) -> Utf8PathBuf {
    if suffix == 0 {
        return paths.root().join(format!("pv.db.{timestamp}.bak"));
    }

    paths.root().join(format!("pv.db.{timestamp}-{suffix}.bak"))
}

pub(crate) fn migration_backups(paths: &PvPaths) -> Result<Vec<String>, StateError> {
    let mut backups = Vec::new();
    let entries = read_backup_directory(paths)?;

    for entry in entries {
        let entry =
            entry.map_err(|source| StateError::filesystem(paths.root().to_path_buf(), source))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();

        if migration_backup_parts(&file_name).is_some() {
            backups.push(file_name.into_owned());
        }
    }

    sort_migration_backup_names(&mut backups);

    Ok(backups)
}

#[expect(
    clippy::disallowed_types,
    reason = "PV backup helper owns migration backup directory traversal"
)]
type ReadDir = std::fs::ReadDir;

#[expect(
    clippy::disallowed_methods,
    reason = "PV backup helper owns migration backup directory traversal"
)]
fn read_backup_directory(paths: &PvPaths) -> Result<ReadDir, StateError> {
    std::fs::read_dir(paths.root())
        .map_err(|source| StateError::filesystem(paths.root().to_path_buf(), source))
}

fn prune_migration_backups(paths: &PvPaths) -> Result<(), StateError> {
    let backups = migration_backups(paths)?;
    let prune_count = backups.len().saturating_sub(MIGRATION_BACKUP_RETENTION);

    for backup in backups.iter().take(prune_count) {
        fs::remove_file(&paths.root().join(backup))?;
    }

    Ok(())
}

fn sort_migration_backup_names(backups: &mut [String]) {
    backups.sort_by(|left, right| {
        migration_backup_sort_key(left).cmp(&migration_backup_sort_key(right))
    });
}

fn migration_backup_sort_key(name: &str) -> (&str, usize) {
    match migration_backup_parts(name) {
        Some((timestamp, suffix)) => (timestamp, suffix),
        None => (name, usize::MAX),
    }
}

fn migration_backup_parts(name: &str) -> Option<(&str, usize)> {
    const TIMESTAMP_LENGTH: usize = "20260522-143012".len();
    let stem = name
        .strip_prefix("pv.db.")
        .and_then(|name| name.strip_suffix(".bak"))?;

    if stem.len() == TIMESTAMP_LENGTH {
        return Some((stem, 0));
    }

    if stem.len() > TIMESTAMP_LENGTH
        && stem.as_bytes().get(TIMESTAMP_LENGTH) == Some(&b'-')
        && let Ok(suffix) = stem[TIMESTAMP_LENGTH + 1..].parse::<usize>()
    {
        return Some((&stem[..TIMESTAMP_LENGTH], suffix));
    }

    None
}

fn backup_timestamp() -> Result<String, StateError> {
    let format = time::macros::format_description!("[year][month][day]-[hour][minute][second]");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}

#[cfg(test)]
mod tests {
    use super::sort_migration_backup_names;

    #[test]
    fn suffixed_backup_names_sort_after_the_unsuffixed_backup_from_the_same_second() {
        let mut backups = vec![
            "pv.db.20260523-120000-3.bak".to_string(),
            "pv.db.20260523-120000.bak".to_string(),
            "pv.db.20260523-120000-1.bak".to_string(),
            "pv.db.20260523-120000-2.bak".to_string(),
        ];

        sort_migration_backup_names(&mut backups);

        assert_eq!(
            backups,
            vec![
                "pv.db.20260523-120000.bak".to_string(),
                "pv.db.20260523-120000-1.bak".to_string(),
                "pv.db.20260523-120000-2.bak".to_string(),
                "pv.db.20260523-120000-3.bak".to_string(),
            ]
        );
    }
}
