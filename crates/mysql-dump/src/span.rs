use std::ops::Range;

use sqlparser::ast::Ident;
use sqlparser::tokenizer::Location;

/// Convert SQLParser character locations to verified source byte spans.
pub(crate) struct SourceLines<'a> {
    source: &'a str,
    starts: Vec<usize>,
}

impl<'a> SourceLines<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        Self { source, starts }
    }

    fn offset(&self, location: Location) -> Option<usize> {
        let line = usize::try_from(location.line.checked_sub(1)?).ok()?;
        let column = usize::try_from(location.column.checked_sub(1)?).ok()?;
        let start = *self.starts.get(line)?;
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());
        let source_line = self.source.get(start..end)?;
        let relative = source_line
            .char_indices()
            .nth(column)
            .map(|(offset, _)| offset)
            .or_else(|| (source_line.chars().count() == column).then_some(source_line.len()))?;
        start.checked_add(relative)
    }

    pub(crate) fn span(&self, identifier: &Ident) -> Option<Range<usize>> {
        let start = self.offset(identifier.span.start)?;
        let end = self.offset(identifier.span.end)?;
        let raw = self.source.get(start..end)?;
        let decoded = match identifier.quote_style {
            None => raw.to_owned(),
            Some(quote @ ('`' | '\'' | '"')) => raw
                .strip_prefix(quote)?
                .strip_suffix(quote)?
                .replace(&format!("{quote}{quote}"), &quote.to_string()),
            _ => return None,
        };
        (start < end && decoded == identifier.value).then_some(start..end)
    }
}
