use std::ops::Range;

use squonk::ast::Resolver;
use squonk::ast::{Ident, ObjectName, Statement};
use squonk::dialect::MySql;
use thiserror::Error;

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
    let parsed = squonk::parse_with(source, squonk::ParseConfig::new(MySql)).map_err(|error| {
        RoutingError::UnsupportedSyntax {
            message: error.to_string(),
        }
    })?;
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
    if start >= end || source.get(start..end).is_none() {
        return Err(RoutingError::InvalidSpan);
    }
    let Some(name) = parsed.resolver().try_resolve(identifier.sym) else {
        return Err(RoutingError::InvalidSpan);
    };
    Ok(Some(RoutingReference {
        action,
        name: name.to_owned(),
        span: start..end,
    }))
}

fn only_identifier(name: &ObjectName) -> Result<&Ident, RoutingError> {
    let [identifier] = name.0.as_slice() else {
        return Err(RoutingError::InvalidSpan);
    };
    Ok(identifier)
}
