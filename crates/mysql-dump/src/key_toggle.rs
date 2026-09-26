use squonk::dialect::{BuiltinDialect, tokenize_with_builtin};
use squonk::tokenizer::{Punctuation, Token, TokenKind};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeyToggleError {
    #[error("could not tokenize MySQL statement: {message}")]
    Lexical { message: String },
}

/// Recognize only the `ALTER TABLE … DISABLE/ENABLE KEYS` dump optimization.
/// These statements may be omitted from a transformed dump; skipping them
/// changes index-build timing, not the final table definition or data.
pub fn dump_key_toggle(source: &str) -> Result<bool, KeyToggleError> {
    let tokens = tokenize_with_builtin(source, BuiltinDialect::MySql).map_err(|error| {
        KeyToggleError::Lexical {
            message: error.to_string(),
        }
    })?;
    let words: Option<Vec<&str>> = tokens
        .iter()
        .map(|token| token_text(source, *token))
        .collect();
    let Some(words) = words else {
        return Ok(false);
    };
    let matches = match words.as_slice() {
        [alter, table, name, action, keys, semicolon] => {
            keyword(alter, "ALTER")
                && keyword(table, "TABLE")
                && identifier(name, tokens[2])
                && (keyword(action, "DISABLE") || keyword(action, "ENABLE"))
                && keyword(keys, "KEYS")
                && *semicolon == ";"
                && matches!(
                    tokens[5].kind,
                    TokenKind::Punctuation(Punctuation::Semicolon)
                )
        }
        [alter, table, database, dot, name, action, keys, semicolon] => {
            keyword(alter, "ALTER")
                && keyword(table, "TABLE")
                && identifier(database, tokens[2])
                && *dot == "."
                && matches!(tokens[3].kind, TokenKind::Punctuation(Punctuation::Dot))
                && identifier(name, tokens[4])
                && (keyword(action, "DISABLE") || keyword(action, "ENABLE"))
                && keyword(keys, "KEYS")
                && *semicolon == ";"
                && matches!(
                    tokens[7].kind,
                    TokenKind::Punctuation(Punctuation::Semicolon)
                )
        }
        _ => false,
    };
    Ok(matches)
}

fn token_text(source: &str, token: Token) -> Option<&str> {
    source.get(token.span.start() as usize..token.span.end() as usize)
}

fn keyword(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

fn identifier(source: &str, token: Token) -> bool {
    !source.is_empty() && matches!(token.kind, TokenKind::Word | TokenKind::QuotedIdent)
}
