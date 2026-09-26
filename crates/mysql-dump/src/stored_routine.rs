use std::collections::BTreeSet;
use std::ops::Range;

use squonk::dialect::{BuiltinDialect, tokenize_with_builtin};
use squonk::tokenizer::{Punctuation, Token, TokenKind};
use squonk::{ParseConfig, ast::Statement, dialect::MySql};
use thiserror::Error;

const MAX_ROUTINE_BYTES: usize = 8 * 1024 * 1024;
const UPDATE_JOIN: &str = " CROSS JOIN ";

#[derive(Debug, Error)]
pub enum StoredRoutineError {
    #[error("stored routine is too large to inspect")]
    TooLarge,
    #[error("stored routine does not end with its declared delimiter")]
    MissingDelimiter,
    #[error("statement is not a CREATE FUNCTION or CREATE PROCEDURE")]
    WrongStatement,
    #[error("could not tokenize stored routine: {message}")]
    Lexical { message: String },
    #[error("unsupported SELECT INTO target in stored routine")]
    UnsupportedSelectInto,
    #[error("stored routine analysis changed UTF-8 boundaries")]
    InvalidNormalization,
    #[error("unsupported stored routine syntax: {message}")]
    UnsupportedSyntax { message: String },
}

/// Analysis SQL and a map back to the original routine's byte offsets.
/// Only commas in an UPDATE table list may grow into `CROSS JOIN`.
#[derive(Debug, PartialEq, Eq)]
pub struct StoredRoutineAnalysis {
    pub sql: String,
    replacements: Vec<(Range<usize>, usize)>,
}

impl StoredRoutineAnalysis {
    /// Map an unchanged identifier span in analysis SQL to its source bytes.
    pub fn source_span(&self, span: Range<usize>) -> Option<Range<usize>> {
        if span.start >= span.end || self.sql.get(span.clone()).is_none() {
            return None;
        }
        let mut shift = 0;
        for (replacement, original_length) in &self.replacements {
            if span.start < replacement.end && span.end > replacement.start {
                return None;
            }
            if replacement.end <= span.start {
                shift += replacement.len() - original_length;
            } else {
                break;
            }
        }
        Some(span.start.checked_sub(shift)?..span.end.checked_sub(shift)?)
    }
}

/// Make a generated MySQL routine parseable for analysis while keeping every
/// byte offset unchanged. Only `INTO` lists of declared local variables or
/// routine parameters are masked; SELECT expressions remain visible for safety
/// checks. The original SQL must be retained for output. This does not by
/// itself authorize the routine's effects or database references.
pub fn normalize_stored_routine_for_analysis(
    source: &str,
    delimiter: &[u8],
) -> Result<String, StoredRoutineError> {
    let normalized = mask_select_into(source, delimiter)?;
    validate_routine(&normalized)?;
    Ok(normalized)
}

/// Also normalize comma-separated UPDATE table lists when the parser cannot
/// accept their MySQL spelling. The original SQL is never replaced by this SQL.
pub fn analyze_stored_routine(
    source: &str,
    delimiter: &[u8],
) -> Result<StoredRoutineAnalysis, StoredRoutineError> {
    let mut sql = mask_select_into(source, delimiter)?;
    if validate_routine(&sql).is_ok() {
        return Ok(StoredRoutineAnalysis {
            sql,
            replacements: Vec::new(),
        });
    }
    let tokens = tokenize_with_builtin(&sql, BuiltinDialect::MySql).map_err(|error| {
        StoredRoutineError::Lexical {
            message: error.to_string(),
        }
    })?;
    let mut masked_tokens: Vec<_> = tokens
        .windows(3)
        .filter(|sequence| {
            token_is(&sql, sequence[0], "DELETE")
                && identifier_candidate(sequence[1])
                && token_is(&sql, sequence[2], "FROM")
        })
        .map(|sequence| sequence[1].span.start() as usize..sequence[1].span.end() as usize)
        .collect();
    masked_tokens.extend(
        tokens
            .windows(3)
            .filter(|sequence| {
                token_is(&sql, sequence[0], "DROP")
                    && token_is(&sql, sequence[1], "TEMPORARY")
                    && token_is(&sql, sequence[2], "TABLE")
            })
            .map(|sequence| sequence[1].span.start() as usize..sequence[1].span.end() as usize),
    );
    masked_tokens.extend(
        tokens
            .windows(2)
            .filter(|sequence| {
                token_is(&sql, sequence[0], "INSERT")
                    && ["LOW_PRIORITY", "HIGH_PRIORITY", "DELAYED"]
                        .iter()
                        .any(|modifier| token_is(&sql, sequence[1], modifier))
            })
            .map(|sequence| sequence[1].span.start() as usize..sequence[1].span.end() as usize),
    );
    let mut masked = sql.into_bytes();
    for range in masked_tokens {
        for byte in &mut masked[range] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    sql = String::from_utf8(masked).map_err(|_| StoredRoutineError::InvalidNormalization)?;
    let tokens = tokenize_with_builtin(&sql, BuiltinDialect::MySql).map_err(|error| {
        StoredRoutineError::Lexical {
            message: error.to_string(),
        }
    })?;
    let mut in_update = false;
    let mut depth = 0_usize;
    let mut edits = Vec::new();
    for (index, token) in tokens.iter().copied().enumerate() {
        if token_is(&sql, token, "INSERT") {
            let mut next_index = index + 1;
            while tokens.get(next_index).is_some_and(|next| {
                ["LOW_PRIORITY", "HIGH_PRIORITY", "DELAYED", "IGNORE"]
                    .iter()
                    .any(|modifier| token_is(&sql, *next, modifier))
            }) {
                next_index += 1;
            }
            if let Some(next) = tokens.get(next_index)
                && identifier_candidate(*next)
                && !token_is(&sql, *next, "INTO")
                && let Some(previous) = tokens.get(next_index - 1)
            {
                let end = previous.span.end() as usize;
                edits.push((end..end, " INTO"));
            }
        }
        if token_is(&sql, token, "UPDATE") {
            in_update = true;
            depth = 0;
        } else if in_update && token_is(&sql, token, "SET") && depth == 0 {
            in_update = false;
        } else if in_update {
            match token_text(&sql, token) {
                Some("(") => depth += 1,
                Some(")") => depth = depth.saturating_sub(1),
                Some(",") if depth == 0 => {
                    let start = token.span.start() as usize;
                    edits.push((start..start + 1, UPDATE_JOIN));
                }
                _ => {}
            }
        }
    }
    if edits.is_empty() {
        return validate_routine(&sql).map(|()| StoredRoutineAnalysis {
            sql,
            replacements: Vec::new(),
        });
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut replacements = Vec::new();
    let mut transformed = String::with_capacity(sql.len());
    let mut cursor = 0;
    for (range, replacement) in edits {
        let Some(gap) = sql.get(cursor..range.start) else {
            return Err(StoredRoutineError::InvalidNormalization);
        };
        transformed.push_str(gap);
        let start = transformed.len();
        transformed.push_str(replacement);
        replacements.push((start..transformed.len(), range.len()));
        cursor = range.end;
    }
    let Some(tail) = sql.get(cursor..) else {
        return Err(StoredRoutineError::InvalidNormalization);
    };
    transformed.push_str(tail);
    validate_routine(&transformed)?;
    Ok(StoredRoutineAnalysis {
        sql: transformed,
        replacements,
    })
}

fn mask_select_into(source: &str, delimiter: &[u8]) -> Result<String, StoredRoutineError> {
    if source.len() > MAX_ROUTINE_BYTES {
        return Err(StoredRoutineError::TooLarge);
    }
    if delimiter.is_empty() {
        return Err(StoredRoutineError::MissingDelimiter);
    }
    let Some(delimiter_start) = source
        .as_bytes()
        .strip_suffix(delimiter)
        .map(|prefix| prefix.len())
    else {
        return Err(StoredRoutineError::MissingDelimiter);
    };
    let mut normalized = source.as_bytes().to_vec();
    normalized[delimiter_start] = b';';
    for byte in &mut normalized[delimiter_start + 1..] {
        *byte = b' ';
    }

    let analysis =
        std::str::from_utf8(&normalized).map_err(|_| StoredRoutineError::InvalidNormalization)?;
    let tokens = tokenize_with_builtin(analysis, BuiltinDialect::MySql).map_err(|error| {
        StoredRoutineError::Lexical {
            message: error.to_string(),
        }
    })?;
    if !tokens
        .first()
        .is_some_and(|token| token_is(source, *token, "CREATE"))
        || !tokens
            .iter()
            .take_while(|token| {
                !matches!(token.kind, TokenKind::Punctuation(Punctuation::Semicolon))
            })
            .any(|token| {
                token_is(source, *token, "FUNCTION") || token_is(source, *token, "PROCEDURE")
            })
    {
        return Err(StoredRoutineError::WrongStatement);
    }
    let mut declared = BTreeSet::new();
    collect_parameter_names(source, &tokens, &mut declared);
    for (index, token) in tokens.iter().enumerate() {
        if token_is(source, *token, "DECLARE") {
            let mut name_index = index + 1;
            while let Some(name) = tokens
                .get(name_index)
                .and_then(|token| token_identifier(source, *token))
            {
                declared.insert(name);
                if tokens
                    .get(name_index + 1)
                    .and_then(|token| token_text(source, *token))
                    != Some(",")
                {
                    break;
                }
                name_index += 2;
            }
        }
    }

    let mut in_select = false;
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token.kind, TokenKind::Punctuation(Punctuation::Semicolon)) {
            in_select = false;
        } else if token_is(source, *token, "SELECT") {
            in_select = true;
        } else if in_select && token_is(source, *token, "INTO") {
            let start = token.span.start() as usize;
            let mut target_index = index + 1;
            let end = loop {
                let Some(target) = tokens.get(target_index) else {
                    return Err(StoredRoutineError::UnsupportedSelectInto);
                };
                let Some(name) = token_identifier(source, *target) else {
                    return Err(StoredRoutineError::UnsupportedSelectInto);
                };
                if matches!(name.as_str(), "outfile" | "dumpfile") || !declared.contains(&name) {
                    return Err(StoredRoutineError::UnsupportedSelectInto);
                }
                match tokens.get(target_index + 1).map(|next| next.kind) {
                    Some(TokenKind::Punctuation(Punctuation::Comma)) => target_index += 2,
                    Some(TokenKind::Punctuation(Punctuation::Dot)) => {
                        return Err(StoredRoutineError::UnsupportedSelectInto);
                    }
                    _ => break target.span.end() as usize,
                }
            };
            for byte in &mut normalized[start..end] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
        }
    }
    let normalized =
        String::from_utf8(normalized).map_err(|_| StoredRoutineError::InvalidNormalization)?;
    Ok(normalized)
}

fn validate_routine(normalized: &str) -> Result<(), StoredRoutineError> {
    let parsed = squonk::parse_with(normalized, ParseConfig::new(MySql)).map_err(|error| {
        StoredRoutineError::UnsupportedSyntax {
            message: error.to_string(),
        }
    })?;
    if !matches!(
        parsed.statements(),
        [Statement::CreateFunction { .. } | Statement::CreateProcedure { .. }]
    ) {
        return Err(StoredRoutineError::WrongStatement);
    }
    Ok(())
}

fn token_text(source: &str, token: Token) -> Option<&str> {
    source.get(token.span.start() as usize..token.span.end() as usize)
}

fn token_identifier(source: &str, token: Token) -> Option<String> {
    let raw = token_text(source, token)?;
    match token.kind {
        TokenKind::Word => Some(raw.to_ascii_lowercase()),
        TokenKind::QuotedIdent => raw
            .strip_prefix('`')
            .and_then(|name| name.strip_suffix('`'))
            .map(|name| name.replace("``", "`").to_ascii_lowercase()),
        _ => None,
    }
}

fn identifier_candidate(token: Token) -> bool {
    matches!(
        token.kind,
        TokenKind::Word | TokenKind::QuotedIdent | TokenKind::Keyword(_)
    )
}

fn collect_parameter_names(source: &str, tokens: &[Token], declared: &mut BTreeSet<String>) {
    let Some(kind_index) = tokens.iter().position(|token| {
        token_is(source, *token, "FUNCTION") || token_is(source, *token, "PROCEDURE")
    }) else {
        return;
    };
    let Some(open) = tokens
        .iter()
        .enumerate()
        .skip(kind_index + 1)
        .find_map(|(index, token)| (token_text(source, *token) == Some("(")).then_some(index))
    else {
        return;
    };
    let mut depth = 1_usize;
    let mut expects_name = true;
    for token in &tokens[open + 1..] {
        match token_text(source, *token) {
            Some("(") => depth += 1,
            Some(")") => {
                let Some(next_depth) = depth.checked_sub(1) else {
                    break;
                };
                depth = next_depth;
                if depth == 0 {
                    break;
                }
            }
            Some(",") if depth == 1 => expects_name = true,
            _ if depth == 1 && expects_name => {
                if token_is(source, *token, "IN")
                    || token_is(source, *token, "OUT")
                    || token_is(source, *token, "INOUT")
                {
                    continue;
                }
                if let Some(name) = token_identifier(source, *token) {
                    declared.insert(name);
                }
                expects_name = false;
            }
            _ => {}
        }
    }
}

fn token_is(source: &str, token: Token, expected: &str) -> bool {
    matches!(token.kind, TokenKind::Word | TokenKind::Keyword(_))
        && token_text(source, token).is_some_and(|text| text.eq_ignore_ascii_case(expected))
}
