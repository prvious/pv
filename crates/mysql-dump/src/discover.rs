use std::collections::BTreeSet;
use std::io::{self, Read, Seek, SeekFrom};

use squonk::dialect::{BuiltinDialect, tokenize_with_builtin};
use thiserror::Error;

use crate::reader::{Frame, ReaderError, scan_with};
use crate::routing::routing_reference;

const MAX_DISCOVERY_STATEMENT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum DiscoverError {
    #[error(transparent)]
    Reader(#[from] ReaderError),
    #[error("could not read dump during discovery: {0}")]
    Io(#[from] io::Error),
    #[error("dump changed during database discovery")]
    Changed,
    #[error("database routing at byte {offset} is unsupported")]
    UnsupportedRouting { offset: u64 },
}

/// Discover source database sections before assigning Project targets. Complete
/// statement policy is still enforced by [`crate::preflight_dump`].
pub fn discover_source_databases<R: Read, S: Read + Seek>(
    scan_source: R,
    inspect_source: &mut S,
) -> Result<BTreeSet<String>, DiscoverError> {
    let mut databases = BTreeSet::new();
    scan_with(scan_source, |frame| {
        let Frame::Sql { range, .. } = frame else {
            return Ok(());
        };
        let length = range.end - range.start;
        if length > MAX_DISCOVERY_STATEMENT_BYTES {
            return Ok(());
        }
        inspect_source.seek(SeekFrom::Start(range.start))?;
        let mut source = Vec::with_capacity(length as usize);
        inspect_source.take(length).read_to_end(&mut source)?;
        if source.len() != length as usize {
            return Err(DiscoverError::Changed);
        }
        let Ok(source) = std::str::from_utf8(&source) else {
            return Ok(());
        };
        let Ok(tokens) = tokenize_with_builtin(source, BuiltinDialect::MySql) else {
            return Ok(());
        };
        let words: Vec<_> = tokens
            .iter()
            .take(2)
            .filter_map(|token| source.get(token.span.start() as usize..token.span.end() as usize))
            .collect();
        let word = |index: usize, expected: &str| {
            words
                .get(index)
                .is_some_and(|word| word.eq_ignore_ascii_case(expected))
        };
        let routing = word(0, "USE")
            || (["CREATE", "ALTER", "DROP"].iter().any(|verb| word(0, verb))
                && (word(1, "DATABASE") || word(1, "SCHEMA")));
        if routing {
            let reference = routing_reference(source)
                .map_err(|_| DiscoverError::UnsupportedRouting {
                    offset: range.start,
                })?
                .ok_or(DiscoverError::UnsupportedRouting {
                    offset: range.start,
                })?;
            databases.insert(reference.name);
        }
        Ok(())
    })?;
    Ok(databases)
}
