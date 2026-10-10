//! Rotates a runtime log the monitor captured.
//!
//! The runtime appends to the same open file the monitor retains, so rotation copies and
//! truncates that file in place rather than renaming it. Output written between the copy and the
//! truncation is lost; that window is accepted.

use std::io::{self, Write as _};
use std::os::unix::fs::FileExt as _;

use camino::{Utf8Path, Utf8PathBuf};
use state::fs;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::DaemonError;

/// How many archives a log keeps.
const KEEP: usize = 5;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
/// `YYYYMMDD-HHMMSS-mmm`.
const STAMP_LENGTH: usize = 19;

/// Once `log` reaches `threshold` bytes, archives exactly those bytes as
/// `<stem>-YYYYMMDD-HHMMSS-mmm.log` in UTC, beside `path`, truncates `log`, and keeps the
/// newest archives. A failure leaves the active log untouched. `before_copy` runs once the
/// archive's temporary file exists.
#[expect(
    clippy::disallowed_types,
    reason = "rotation copies from the monitor's retained log descriptor"
)]
pub(super) fn rotate_if_needed(
    log: &std::fs::File,
    path: &Utf8Path,
    threshold: u64,
    before_copy: impl FnOnce(),
) -> Result<(), DaemonError> {
    let size = log.metadata()?.len();
    if size < threshold {
        return Ok(());
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "log path has no file name").into(),
        );
    };
    let stem = name.strip_suffix(".log").unwrap_or(name);
    // A dot file, so `pv logs` never shows a partial archive.
    let temporary = parent.join(format!(".{name}.rotating"));
    fs::remove_file_if_exists(&temporary)?;
    let mut archive = fs::create_new_file(&temporary)?;
    before_copy();
    if let Err(error) = copy_prefix(log, &mut archive, size) {
        let _remove_result = fs::remove_file_if_exists(&temporary);
        return Err(error);
    }
    let stamp = OffsetDateTime::now_utc().format(format_description!(
        "[year][month][day]-[hour][minute][second]-[subsecond digits:3]"
    ))?;
    fs::rename(&temporary, &parent.join(format!("{stem}-{stamp}.log")))?;
    log.set_len(0)?;

    prune(parent, stem)
}

#[expect(
    clippy::disallowed_types,
    reason = "rotation copies from the monitor's retained log descriptor"
)]
fn copy_prefix(
    log: &std::fs::File,
    archive: &mut std::fs::File,
    size: u64,
) -> Result<(), DaemonError> {
    let mut buffer = vec![0; COPY_BUFFER_BYTES];
    let mut offset = 0;
    while offset < size {
        let wanted =
            usize::try_from(size - offset).map_or(buffer.len(), |left| left.min(buffer.len()));
        let read = log.read_at(&mut buffer[..wanted], offset)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the log shrank while it was rotated",
            )
            .into());
        }
        archive.write_all(&buffer[..read])?;
        offset += u64::try_from(read).map_err(io::Error::other)?;
    }
    archive.sync_all()?;

    Ok(())
}

fn prune(parent: &Utf8Path, stem: &str) -> Result<(), DaemonError> {
    let prefix = format!("{stem}-");
    let mut archives = fs::read_dir_paths(parent)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| is_archive(name, &prefix))
        })
        .collect::<Vec<Utf8PathBuf>>();
    // Stamps sort in time order.
    archives.sort();
    let excess = archives.len().saturating_sub(KEEP);
    for archive in archives.iter().take(excess) {
        fs::remove_file_if_exists(archive)?;
    }

    Ok(())
}

fn is_archive(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(".log"))
        .is_some_and(|stamp| {
            stamp.len() == STAMP_LENGTH
                && stamp
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'-')
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use camino_tempfile::tempdir;

    use super::*;

    #[test]
    fn rotation_keeps_the_newest_archives_and_appends_after_truncation() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let path = tempdir.path().join("php-8.3.log");
        let log = fs::open_private_log_file(&path)?;
        let unrelated = tempdir.path().join("php-8.4-20260101-000000-000.log");
        fs::write_sensitive_file(&unrelated, "another runtime\n")?;

        for round in 0..7 {
            writeln!(&log, "round {round}")?;
            rotate_if_needed(&log, &path, 1, || {})?;
            // Archive names have millisecond stamps.
            std::thread::sleep(Duration::from_millis(2));
        }
        writeln!(&log, "after rotation")?;
        rotate_if_needed(&log, &path, 1024, || {})?;

        let mut archives = fs::read_dir_paths(tempdir.path())?
            .into_iter()
            .filter(|archive| {
                archive
                    .file_name()
                    .is_some_and(|name| is_archive(name, "php-8.3-"))
            })
            .collect::<Vec<_>>();
        archives.sort();
        let contents = archives
            .iter()
            .map(|archive| fs::read_to_string(archive))
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(
            contents,
            [
                "round 2\n",
                "round 3\n",
                "round 4\n",
                "round 5\n",
                "round 6\n"
            ]
        );
        assert_eq!(fs::read_to_string(&path)?, "after rotation\n");
        assert!(fs::path_exists(&unrelated));

        Ok(())
    }

    #[test]
    fn failed_rotation_keeps_the_active_log() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let path = tempdir.path().join("gateway.log");
        let log = fs::open_private_log_file(&path)?;
        writeln!(&log, "kept")?;
        // A directory where the archive's temporary file must go.
        fs::ensure_user_dir(&tempdir.path().join(".gateway.log.rotating"))?;

        let rotated = rotate_if_needed(&log, &path, 1, || {});

        assert!(rotated.is_err());
        assert_eq!(fs::read_to_string(&path)?, "kept\n");

        Ok(())
    }
}
