use std::ops::Range;

use squonk::ast::Resolver;
use squonk::ast::{Ident, ObjectName, Statement};
use squonk::dialect::MySql;
use thiserror::Error;

use crate::reference::verified_identifier;

/// A database-routing statement in a framed SQL chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutingAction {
    Use,
    Create,
    Alter,
    Drop,
}

/// The source database name and exact source bytes to patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingReference {
    pub action: RoutingAction,
    pub name: String,
    pub span: Range<usize>,
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("unsupported MySQL statement: {message}")]
    UnsupportedSyntax { message: String },
    #[error("expected exactly one SQL statement in the supplied frame")]
    MultipleStatements,
    #[error("database identifier has no verified source span")]
    InvalidSpan,
}

/// Inspect database routing with the MySQL parser and preserve its identifier span.
///
/// Other statement kinds return `None`; this does not authorize them for import.
pub fn routing_reference(source: &str) -> Result<Option<RoutingReference>, RoutingError> {
    let parsed = match squonk::parse_with(source, squonk::ParseConfig::new(MySql)) {
        Ok(parsed) => parsed,
        Err(error) => {
            if let Some(reference) = generated_create_database_reference(source)? {
                return Ok(Some(reference));
            }
            return Err(RoutingError::UnsupportedSyntax {
                message: error.to_string(),
            });
        }
    };
    let [statement] = parsed.statements() else {
        return Err(RoutingError::MultipleStatements);
    };
    let (action, identifier) = match statement {
        Statement::Use { use_statement, .. } => {
            (RoutingAction::Use, only_identifier(&use_statement.name)?)
        }
        Statement::CreateDatabase { create, .. } => {
            (RoutingAction::Create, only_identifier(&create.name)?)
        }
        Statement::AlterDatabaseOptions { alter, .. } => {
            let Some(identifier) = alter.name.as_ref() else {
                return Ok(None);
            };
            (RoutingAction::Alter, identifier)
        }
        Statement::DropDatabase { drop, .. } => (RoutingAction::Drop, &drop.name),
        _ => return Ok(None),
    };
    let start = identifier.meta.span.start() as usize;
    let end = identifier.meta.span.end() as usize;
    let Some(name) = parsed.resolver().try_resolve(identifier.sym) else {
        return Err(RoutingError::InvalidSpan);
    };
    if !verified_identifier(source, start..end, name) {
        return Err(RoutingError::InvalidSpan);
    }
    Ok(Some(RoutingReference {
        action,
        name: name.to_owned(),
        span: start..end,
    }))
}

/// `mysqldump` wraps database options in versioned comments that squonk does
/// not accept. Strip only those comments for routing inspection, keeping byte
/// offsets stable. The original statement must still pass import policy and is
/// never authorized by this function.
fn generated_create_database_reference(
    source: &str,
) -> Result<Option<RoutingReference>, RoutingError> {
    let mut normalized = source.as_bytes().to_vec();
    let mut cursor = 0;
    let mut found = false;
    while let Some(relative_start) = source[cursor..].find("/*!") {
        let start = cursor + relative_start;
        let Some(relative_end) = source[start + 3..].find("*/") else {
            return Err(RoutingError::UnsupportedSyntax {
                message: "unterminated versioned comment in CREATE DATABASE".to_owned(),
            });
        };
        let end = start + 3 + relative_end + 2;
        for byte in &mut normalized[start..end] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
        cursor = end;
        found = true;
    }
    if !found {
        return Ok(None);
    }
    let normalized =
        String::from_utf8(normalized).map_err(|error| RoutingError::UnsupportedSyntax {
            message: error.to_string(),
        })?;
    let parsed =
        squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).map_err(|error| {
            RoutingError::UnsupportedSyntax {
                message: error.to_string(),
            }
        })?;
    let [Statement::CreateDatabase { create, .. }] = parsed.statements() else {
        return Err(RoutingError::MultipleStatements);
    };
    let identifier = only_identifier(&create.name)?;
    let span = identifier.meta.span.start() as usize..identifier.meta.span.end() as usize;
    let Some(name) = parsed.resolver().try_resolve(identifier.sym) else {
        return Err(RoutingError::InvalidSpan);
    };
    if !verified_identifier(source, span.clone(), name) {
        return Err(RoutingError::InvalidSpan);
    }
    Ok(Some(RoutingReference {
        action: RoutingAction::Create,
        name: name.to_owned(),
        span,
    }))
}

fn only_identifier(name: &ObjectName) -> Result<&Ident, RoutingError> {
    let [identifier] = name.0.as_slice() else {
        return Err(RoutingError::InvalidSpan);
    };
    Ok(identifier)
}
