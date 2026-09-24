use std::io::{self, Read, Write};
use std::ops::Range;

use thiserror::Error;

const MAX_PATCH_BYTES: u64 = 1024;

/// Source-verified replacement for one parsed identifier span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub range: Range<u64>,
    pub expected: Vec<u8>,
    pub replacement: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("could not read or write dump: {0}")]
    Io(#[from] io::Error),
    #[error("patch at byte {start} has an invalid range or overlaps a previous patch")]
    InvalidRange { start: u64 },
    #[error("source bytes changed at patch byte {start}")]
    SourceMismatch { start: u64 },
    #[error("source ended before patch byte {start}")]
    TruncatedSource { start: u64 },
}

/// Copy a source dump while replacing verified, ordered identifier spans.
///
/// The input and patch iterator can both be streamed. A caller must first
/// preflight the entire source and use an immutable private snapshot here.
pub fn write_patched<R, W, I>(mut source: R, mut output: W, patches: I) -> Result<(), RewriteError>
where
    R: Read,
    W: Write,
    I: IntoIterator<Item = Patch>,
{
    let mut offset = 0_u64;
    for patch in patches {
        let Some(length) = patch.range.end.checked_sub(patch.range.start) else {
            return Err(RewriteError::InvalidRange {
                start: patch.range.start,
            });
        };
        if patch.range.start < offset
            || length == 0
            || length > MAX_PATCH_BYTES
            || length != patch.expected.len() as u64
        {
            return Err(RewriteError::InvalidRange {
                start: patch.range.start,
            });
        }
        let gap = patch.range.start - offset;
        let copied = io::copy(&mut source.by_ref().take(gap), &mut output)?;
        if copied != gap {
            return Err(RewriteError::TruncatedSource {
                start: patch.range.start,
            });
        }
        let mut observed = vec![0; patch.expected.len()];
        source.read_exact(&mut observed)?;
        if observed != patch.expected {
            return Err(RewriteError::SourceMismatch {
                start: patch.range.start,
            });
        }
        output.write_all(&patch.replacement)?;
        offset = patch.range.end;
    }
    io::copy(&mut source, &mut output)?;
    Ok(())
}
