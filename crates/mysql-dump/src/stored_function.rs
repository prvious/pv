use std::collections::BTreeSet;

use squonk::dialect::{BuiltinDialect, tokenize_with_builtin};
use squonk::tokenizer::{Punctuation, Token, TokenKind};
use thiserror::Error;

const MAX_FUNCTION_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum StoredFunctionError {
    #[error("stored function is too large to inspect")]
    TooLarge,
    #[error("stored function does not end with its declared delimiter")]
    MissingDelimiter,
    #[error("statement is not a CREATE FUNCTION")]
    WrongStatement,
    #[error("could not tokenize stored function: {message}")]
    Lexical { message: String },
    #[error("unsupported SELECT INTO target in stored function")]
    UnsupportedSelectInto,
    #[error("stored function analysis changed UTF-8 boundaries")]
    InvalidNormalization,
}

/// Make a generated MySQL function parseable for analysis while keeping every
/// byte offset unchanged. Only a `SELECT … INTO <declared-local-variable>` clause
/// is masked; the original SQL must be retained for output. This does not by
/// itself authorize the function's effects or database references.
pub fn normalize_stored_function_for_analysis(
    source: &str,
    delimiter: &[u8],
) -> Result<String, StoredFunctionError> {
    if source.len() > MAX_FUNCTION_BYTES {
        return Err(StoredFunctionError::TooLarge);
    }
    if delimiter.is_empty() {
        return Err(StoredFunctionError::MissingDelimiter);
    }
    let Some(delimiter_start) = source
        .as_bytes()
        .strip_suffix(delimiter)
        .map(|prefix| prefix.len())
    else {
        return Err(StoredFunctionError::MissingDelimiter);
    };
    let mut normalized = source.as_bytes().to_vec();
    normalized[delimiter_start] = b';';
    for byte in &mut normalized[delimiter_start + 1..] {
        *byte = b' ';
    }

    let analysis =
        std::str::from_utf8(&normalized).map_err(|_| StoredFunctionError::InvalidNormalization)?;
    let tokens = tokenize_with_builtin(analysis, BuiltinDialect::MySql).map_err(|error| {
        StoredFunctionError::Lexical {
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
            .any(|token| token_is(source, *token, "FUNCTION"))
    {
        return Err(StoredFunctionError::WrongStatement);
    }
    let mut declared = BTreeSet::new();
    for pair in tokens.windows(2) {
        if token_is(source, pair[0], "DECLARE")
            && matches!(pair[1].kind, TokenKind::Word)
            && let Some(name) = token_text(source, pair[1])
        {
            declared.insert(name.to_ascii_lowercase());
        }
    }

    let mut in_select = false;
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token.kind, TokenKind::Punctuation(Punctuation::Semicolon)) {
            in_select = false;
        } else if token_is(source, *token, "SELECT") {
            in_select = true;
        } else if in_select && token_is(source, *token, "INTO") {
            let Some(target) = tokens.get(index + 1) else {
                return Err(StoredFunctionError::UnsupportedSelectInto);
            };
            let Some(name) = token_text(source, *target) else {
                return Err(StoredFunctionError::UnsupportedSelectInto);
            };
            if !matches!(target.kind, TokenKind::Word)
                || !declared.contains(&name.to_ascii_lowercase())
                || tokens.get(index + 2).is_some_and(|next| {
                    matches!(next.kind, TokenKind::Punctuation(Punctuation::Comma))
                })
            {
                return Err(StoredFunctionError::UnsupportedSelectInto);
            }
            let start = token.span.start() as usize;
            let end = target.span.end() as usize;
            for byte in &mut normalized[start..end] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
        }
    }
    String::from_utf8(normalized).map_err(|_| StoredFunctionError::InvalidNormalization)
}

fn token_text(source: &str, token: Token) -> Option<&str> {
    source.get(token.span.start() as usize..token.span.end() as usize)
}

fn token_is(source: &str, token: Token, expected: &str) -> bool {
    matches!(token.kind, TokenKind::Word | TokenKind::Keyword(_))
        && token_text(source, token).is_some_and(|text| text.eq_ignore_ascii_case(expected))
}
