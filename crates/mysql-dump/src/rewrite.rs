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

/// A verified identifier patch or a complete frame to omit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Patch(Patch),
    Skip(Range<u64>),
}

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("could not read or write dump: {0}")]
    Io(#[from] io::Error),
    #[error("edit at byte {start} has an invalid range or overlaps a previous edit")]
    InvalidRange { start: u64 },
    #[error("source bytes changed at patch byte {start}")]
    SourceMismatch { start: u64 },
    #[error("source ended before edit byte {start}")]
    TruncatedSource { start: u64 },
}

/// Copy a source dump while replacing verified, ordered identifier spans.
///
/// The input and patch iterator can both be streamed. A caller must first
/// preflight the entire source and use an immutable private snapshot here.
pub fn write_patched<R, W, I>(source: R, output: W, patches: I) -> Result<(), RewriteError>
where
    R: Read,
    W: Write,
    I: IntoIterator<Item = Patch>,
{
    write_transformed(source, output, patches.into_iter().map(Edit::Patch))
}

/// Stream verified patches and whole-frame omissions from a private snapshot.
/// Edits must be ordered and non-overlapping, after complete preflight.
pub fn write_transformed<R, W, I>(
    mut source: R,
    mut output: W,
    edits: I,
) -> Result<(), RewriteError>
where
    R: Read,
    W: Write,
    I: IntoIterator<Item = Edit>,
{
    let mut offset = 0_u64;
    for edit in edits {
        let range = match &edit {
            Edit::Patch(patch) => &patch.range,
            Edit::Skip(range) => range,
        };
        let start = range.start;
        let end = range.end;
        let Some(length) = end.checked_sub(start) else {
            return Err(RewriteError::InvalidRange { start });
        };
        if start < offset || length == 0 {
            return Err(RewriteError::InvalidRange { start });
        }
        let gap = start - offset;
        let copied = io::copy(&mut source.by_ref().take(gap), &mut output)?;
        if copied != gap {
            return Err(RewriteError::TruncatedSource { start });
        }
        match edit {
            Edit::Patch(patch) => {
                if length > MAX_PATCH_BYTES || length != patch.expected.len() as u64 {
                    return Err(RewriteError::InvalidRange { start });
                }
                let mut observed = vec![0; patch.expected.len()];
                source.read_exact(&mut observed)?;
                if observed != patch.expected {
                    return Err(RewriteError::SourceMismatch { start });
                }
                output.write_all(&patch.replacement)?;
            }
            Edit::Skip(_) => {
                let skipped = io::copy(&mut source.by_ref().take(length), &mut io::sink())?;
                if skipped != length {
                    return Err(RewriteError::TruncatedSource { start });
                }
            }
        }
        offset = end;
    }
    io::copy(&mut source, &mut output)?;
    Ok(())
}
