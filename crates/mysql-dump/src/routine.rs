use std::collections::BTreeSet;
use std::ops::Range;

use squonk::ast::Resolver;
use squonk::ast::{RoutineObjectKind, Statement};
use squonk::dialect::{BuiltinDialect, MySql, tokenize_with_builtin};
use squonk::tokenizer::{Token, TokenKind};
use thiserror::Error;

const MAX_ROUTINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutineAction {
    Create,
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutineKind {
    Function,
    Procedure,
}

/// Name of a routine definition or drop in a framed dump statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutineReference {
    pub action: RoutineAction,
    pub kind: RoutineKind,
    /// Decoded source database qualifier, or `None` for an unqualified name.
    pub database: Option<String>,
    /// Decoded routine identifier, without backticks.
    pub name: String,
    /// Byte range of the raw routine identifier within the original frame.
    pub name_span: Range<usize>,
}

/// Decoded source database and routine name, compared case-sensitively.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoutineName {
    pub database: String,
    pub name: String,
}

/// Requested routines to omit from a dump before any SQL is executed.
#[derive(Debug, Default)]
pub struct RoutineSkips {
    requested: BTreeSet<RoutineName>,
    seen: BTreeSet<RoutineName>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RoutineSkipError {
    #[error("routine skip was requested more than once: {database}.{name}")]
    Duplicate { database: String, name: String },
    #[error("a routine has no source database context")]
    MissingDatabase,
    #[error("requested routine skips are absent from the dump: {names:?}")]
    Absent { names: Vec<RoutineName> },
}

impl RoutineSkips {
    pub fn new(requested: impl IntoIterator<Item = RoutineName>) -> Result<Self, RoutineSkipError> {
        let mut skips = Self::default();
        for name in requested {
            if !skips.requested.insert(name.clone()) {
                return Err(RoutineSkipError::Duplicate {
                    database: name.database,
                    name: name.name,
                });
            }
        }
        Ok(skips)
    }

    /// Return true only when the whole DROP or CREATE frame must be omitted.
    /// An unqualified name uses the source database currently selected by `USE`
    /// or the plain dump header. No source bytes are rewritten here.
    pub fn observe(
        &mut self,
        reference: &RoutineReference,
        active_database: Option<&str>,
    ) -> Result<bool, RoutineSkipError> {
        if self.requested.is_empty() {
            return Ok(false);
        }
        let database = reference
            .database
            .as_deref()
            .or(active_database)
            .ok_or(RoutineSkipError::MissingDatabase)?;
        let name = RoutineName {
            database: database.to_owned(),
            name: reference.name.clone(),
        };
        if self.requested.contains(&name) {
            self.seen.insert(name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Validate every requested name and return the names for the import plan.
    pub fn finish(self) -> Result<Vec<RoutineName>, RoutineSkipError> {
        let absent: Vec<RoutineName> = self.requested.difference(&self.seen).cloned().collect();
        if !absent.is_empty() {
            return Err(RoutineSkipError::Absent { names: absent });
        }
        Ok(self.requested.into_iter().collect())
    }
}

#[derive(Debug, Error)]
pub enum RoutineError {
    #[error("routine statement is too large to inspect")]
    TooLarge,
    #[error("routine statement does not end with its declared delimiter")]
    MissingDelimiter,
    #[error("could not tokenize routine statement: {message}")]
    Lexical { message: String },
    #[error("routine name or statement boundary is unsupported")]
    InvalidName,
    #[error("could not parse routine drop: {message}")]
    UnsupportedDrop { message: String },
}

/// Identify a routine in one framed statement without authorizing its body.
/// This lets preflight omit an explicitly skipped routine even when its body uses
/// syntax the SQL parser cannot inspect. The caller must skip the entire frame.
/// Other statements return `None`; the complete frame is still tokenized.
pub fn routine_reference(
    source: &str,
    delimiter: &[u8],
) -> Result<Option<RoutineReference>, RoutineError> {
    if source.len() > MAX_ROUTINE_BYTES {
        return Err(RoutineError::TooLarge);
    }
    if delimiter.is_empty() {
        return Err(RoutineError::MissingDelimiter);
    }
    let Some(prefix) = source.as_bytes().strip_suffix(delimiter) else {
        return Err(RoutineError::MissingDelimiter);
    };
    let Some(prefix) = source.get(..prefix.len()) else {
        return Err(RoutineError::MissingDelimiter);
    };
    let mut analysis = String::with_capacity(source.len());
    analysis.push_str(prefix);
    analysis.push(';');
    let tokens = tokenize_with_builtin(&analysis, BuiltinDialect::MySql).map_err(|error| {
        RoutineError::Lexical {
            message: error.to_string(),
        }
    })?;
    let Some(first) = tokens.first() else {
        return Ok(None);
    };
    if token_is(&analysis, *first, "CREATE") {
        return create_reference(source, &analysis, &tokens);
    }
    if !token_is(&analysis, *first, "DROP") {
        return Ok(None);
    }
    let Some(second) = tokens.get(1) else {
        return Err(RoutineError::InvalidName);
    };
    if !token_is(&analysis, *second, "PROCEDURE") && !token_is(&analysis, *second, "FUNCTION") {
        return Ok(None);
    }
    drop_reference(source, &analysis)
}

fn create_reference(
    source: &str,
    analysis: &str,
    tokens: &[Token],
) -> Result<Option<RoutineReference>, RoutineError> {
    let Some(second) = tokens.get(1) else {
        return Err(RoutineError::InvalidName);
    };
    let kind_index = if token_is(analysis, *second, "DEFINER") {
        let Some(equals) = tokens.get(2) else {
            return Err(RoutineError::InvalidName);
        };
        if token_text(analysis, *equals) != Some("=") {
            return Err(RoutineError::InvalidName);
        }
        let Some(user) = tokens.get(3) else {
            return Err(RoutineError::InvalidName);
        };
        let mut next = 4;
        if token_is(analysis, *user, "CURRENT_USER")
            && tokens
                .get(next)
                .is_some_and(|token| token_text(analysis, *token) == Some("("))
        {
            if !tokens
                .get(next + 1)
                .is_some_and(|token| token_text(analysis, *token) == Some(")"))
            {
                return Err(RoutineError::InvalidName);
            }
            next += 2;
        } else if let Some(host) = tokens
            .get(next)
            .and_then(|token| token_text(analysis, *token))
            && host.starts_with('@')
        {
            if host == "@" {
                if tokens.get(next + 1).is_none() {
                    return Err(RoutineError::InvalidName);
                }
                next += 2;
            } else {
                next += 1;
            }
        }
        next
    } else {
        1
    };
    let Some(kind_token) = tokens.get(kind_index) else {
        return Ok(None);
    };
    let kind = if token_is(analysis, *kind_token, "PROCEDURE") {
        RoutineKind::Procedure
    } else if token_is(analysis, *kind_token, "FUNCTION") {
        RoutineKind::Function
    } else {
        return Ok(None);
    };
    let name_index = if tokens
        .get(kind_index + 1)
        .is_some_and(|token| token_is(analysis, *token, "IF"))
    {
        if !tokens
            .get(kind_index + 2)
            .is_some_and(|token| token_is(analysis, *token, "NOT"))
            || !tokens
                .get(kind_index + 3)
                .is_some_and(|token| token_is(analysis, *token, "EXISTS"))
        {
            return Err(RoutineError::InvalidName);
        }
        kind_index + 4
    } else {
        kind_index + 1
    };
    let Some(first_name) = tokens.get(name_index) else {
        return Err(RoutineError::InvalidName);
    };
    let (database, name_token) = if tokens
        .get(name_index + 1)
        .and_then(|token| token_text(analysis, *token))
        == Some(".")
    {
        let database = identifier(source, *first_name).ok_or(RoutineError::InvalidName)?;
        let name = tokens
            .get(name_index + 2)
            .ok_or(RoutineError::InvalidName)?;
        (Some(database), *name)
    } else {
        (None, *first_name)
    };
    let after_name = if database.is_some() {
        name_index + 3
    } else {
        name_index + 1
    };
    if !tokens
        .get(after_name)
        .is_some_and(|token| token_text(analysis, *token) == Some("("))
    {
        return Err(RoutineError::InvalidName);
    }
    let name = identifier(source, name_token).ok_or(RoutineError::InvalidName)?;
    let span = name_token.span.start() as usize..name_token.span.end() as usize;
    Ok(Some(RoutineReference {
        action: RoutineAction::Create,
        kind,
        database,
        name,
        name_span: span,
    }))
}

fn drop_reference(source: &str, analysis: &str) -> Result<Option<RoutineReference>, RoutineError> {
    let parsed =
        squonk::parse_with(analysis, squonk::ParseConfig::new(MySql)).map_err(|error| {
            RoutineError::UnsupportedDrop {
                message: error.to_string(),
            }
        })?;
    let [Statement::DropRoutine { kind, routines, .. }] = parsed.statements() else {
        return Err(RoutineError::InvalidName);
    };
    let [routine] = routines.as_slice() else {
        return Err(RoutineError::InvalidName);
    };
    let kind = match kind {
        RoutineObjectKind::Function => RoutineKind::Function,
        RoutineObjectKind::Procedure => RoutineKind::Procedure,
        RoutineObjectKind::Routine => return Err(RoutineError::InvalidName),
    };
    let (database, name_identifier) = match routine.name.0.as_slice() {
        [name] => (None, name),
        [database, name] => {
            let database_name = parsed
                .resolver()
                .try_resolve(database.sym)
                .ok_or(RoutineError::InvalidName)?;
            (Some(database_name.to_owned()), name)
        }
        _ => return Err(RoutineError::InvalidName),
    };
    let name = parsed
        .resolver()
        .try_resolve(name_identifier.sym)
        .ok_or(RoutineError::InvalidName)?;
    let span = name_identifier.meta.span.start() as usize..name_identifier.meta.span.end() as usize;
    if source.get(span.clone()).is_none() {
        return Err(RoutineError::InvalidName);
    }
    Ok(Some(RoutineReference {
        action: RoutineAction::Drop,
        kind,
        database,
        name: name.to_owned(),
        name_span: span,
    }))
}

fn identifier(source: &str, token: Token) -> Option<String> {
    let raw = token_text(source, token)?;
    match token.kind {
        TokenKind::QuotedIdent if raw.starts_with('`') && raw.ends_with('`') => {
            Some(raw[1..raw.len() - 1].replace("``", "`"))
        }
        TokenKind::Word | TokenKind::Keyword(_) => Some(raw.to_owned()),
        _ => None,
    }
}

fn token_text(source: &str, token: Token) -> Option<&str> {
    source.get(token.span.start() as usize..token.span.end() as usize)
}

fn token_is(source: &str, token: Token, expected: &str) -> bool {
    matches!(token.kind, TokenKind::Word | TokenKind::Keyword(_))
        && token_text(source, token).is_some_and(|text| text.eq_ignore_ascii_case(expected))
}
