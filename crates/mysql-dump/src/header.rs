use std::io::{self, BufRead, BufReader, Read};

use thiserror::Error;

const MAX_HEADER_BYTES: u64 = 64 * 1024;

#[derive(Debug, Error)]
pub enum HeaderError {
    #[error("could not read dump header: {0}")]
    Io(#[from] io::Error),
    #[error("dump header is not UTF-8")]
    InvalidUtf8,
    #[error("dump header names conflicting source databases")]
    ConflictingDatabases,
}

/// Read the optional database name from the leading `mysqldump` or phpMyAdmin
/// comments. Reads at most 64 KiB, stopping at the first non-comment line.
pub fn source_database_header(input: impl Read) -> Result<Option<String>, HeaderError> {
    let mut reader = BufReader::new(input.take(MAX_HEADER_BYTES));
    let mut database: Option<String> = None;
    let mut line = Vec::new();
    let mut first_line = true;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let content = if first_line {
            line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&line)
        } else {
            &line
        };
        first_line = false;
        let trimmed = content.trim_ascii();
        if trimmed.is_empty() {
            continue;
        }
        let Some(comment) = trimmed.strip_prefix(b"--") else {
            break;
        };
        if !comment.is_empty() && !comment[0].is_ascii_whitespace() {
            break;
        }
        let comment = std::str::from_utf8(comment.trim_ascii_start())
            .map_err(|_| HeaderError::InvalidUtf8)?;
        let candidate = if let Some(rest) = comment.strip_prefix("Host:") {
            rest.split_once("Database:").map(|(_, name)| name.trim())
        } else {
            comment.strip_prefix("Database:").map(str::trim)
        };
        let Some(candidate) = candidate.filter(|name| !name.is_empty()) else {
            continue;
        };
        let candidate = candidate
            .strip_prefix('`')
            .and_then(|name| name.strip_suffix('`'))
            .unwrap_or(candidate)
            .replace("``", "`");
        if database
            .as_ref()
            .is_some_and(|previous| previous != &candidate)
        {
            return Err(HeaderError::ConflictingDatabases);
        }
        database = Some(candidate);
    }
    Ok(database)
}
