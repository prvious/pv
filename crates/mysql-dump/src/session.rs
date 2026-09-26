use squonk::ast::Resolver;
use squonk::ast::{
    Expr, SessionStatement, SessionVariableKind, SetVariableAssignment, SetVariableValue,
    Statement, SystemVariableScope, SystemVariableScopeKind,
};
use squonk::dialect::MySql;
use thiserror::Error;

/// Treatment of a parsed connection setup statement in the transformed dump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionSetup {
    Keep,
    /// A user-variable SQL-mode restore is unnecessary for the import-only connection.
    Skip,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("unsupported MySQL session setup: {message}")]
    UnsupportedSyntax { message: String },
    #[error("session setup changes server state or uses an unsafe expression")]
    UnsafeSetup,
}

/// Classify one framed statement's connection setup using the MySQL AST.
///
/// This is one preflight check, not authorization to run the rest of a dump.
/// Callers must omit [`SessionSetup::Skip`] frames from executable output.
pub fn session_setup(source: &str) -> Result<Option<SessionSetup>, SessionError> {
    let parsed = squonk::parse_with(source, squonk::ParseConfig::new(MySql)).map_err(|error| {
        SessionError::UnsupportedSyntax {
            message: error.to_string(),
        }
    })?;
    let [statement] = parsed.statements() else {
        return Err(SessionError::UnsupportedSyntax {
            message: "expected exactly one statement".to_owned(),
        });
    };
    let Statement::Session { session, .. } = statement else {
        return Ok(None);
    };
    let resolver = parsed.resolver();
    match session.as_ref() {
        SessionStatement::SetNames { .. } | SessionStatement::SetCharacterSet { .. } => {
            Ok(Some(SessionSetup::Keep))
        }
        SessionStatement::SetVariables { assignments, .. } => {
            let mut skip = false;
            for assignment in assignments {
                match assignment {
                    SetVariableAssignment::SystemVariable {
                        scope, name, value, ..
                    } => {
                        if !connection_scope(*scope) {
                            return Err(SessionError::UnsafeSetup);
                        }
                        let [name] = name.0.as_slice() else {
                            return Err(SessionError::UnsafeSetup);
                        };
                        let Some(name) = resolver.try_resolve(name.sym) else {
                            return Err(SessionError::UnsafeSetup);
                        };
                        let normalized = name.to_ascii_lowercase();
                        if !matches!(
                            normalized.as_str(),
                            "character_set_client"
                                | "character_set_results"
                                | "collation_connection"
                                | "time_zone"
                                | "unique_checks"
                                | "foreign_key_checks"
                                | "sql_mode"
                                | "sql_notes"
                        ) || !(safe_value(value)
                            || (matches!(
                                normalized.as_str(),
                                "character_set_client"
                                    | "character_set_results"
                                    | "collation_connection"
                            ) && safe_charset_value(value, source, &normalized)))
                        {
                            return Err(SessionError::UnsafeSetup);
                        }
                        if normalized == "sql_mode" {
                            let SetVariableValue::Expr { expr, .. } = value else {
                                return Err(SessionError::UnsafeSetup);
                            };
                            match expr.as_ref() {
                                Expr::Literal { literal, .. } => {
                                    let span = literal.meta.span;
                                    let Some(text) =
                                        source.get(span.start() as usize..span.end() as usize)
                                    else {
                                        return Err(SessionError::UnsafeSetup);
                                    };
                                    if !safe_sql_mode_literal(text) {
                                        return Err(SessionError::UnsafeSetup);
                                    }
                                }
                                Expr::SessionVariable {
                                    kind: SessionVariableKind::User,
                                    ..
                                } => skip = true,
                                _ => return Err(SessionError::UnsafeSetup),
                            }
                        }
                    }
                    SetVariableAssignment::UserVariable { name, value, .. } => {
                        let Some(name) = resolver.try_resolve(name.sym) else {
                            return Err(SessionError::UnsafeSetup);
                        };
                        let name = name.to_ascii_lowercase();
                        if !safe_expr(value)
                            || (!(name.starts_with("old_") || name.starts_with("saved_"))
                                && !matches!(
                                    value.as_ref(),
                                    Expr::SessionVariable {
                                        kind: SessionVariableKind::System
                                            | SessionVariableKind::SystemSession,
                                        ..
                                    }
                                ))
                        {
                            return Err(SessionError::UnsafeSetup);
                        }
                    }
                }
            }
            if skip && assignments.len() != 1 {
                return Err(SessionError::UnsafeSetup);
            }
            Ok(Some(if skip {
                SessionSetup::Skip
            } else {
                SessionSetup::Keep
            }))
        }
        _ => Err(SessionError::UnsafeSetup),
    }
}

fn connection_scope(scope: SystemVariableScope) -> bool {
    matches!(
        scope,
        SystemVariableScope::Implicit
            | SystemVariableScope::AtAt
            | SystemVariableScope::Keyword(
                SystemVariableScopeKind::Session | SystemVariableScopeKind::Local
            )
            | SystemVariableScope::AtAtScoped(
                SystemVariableScopeKind::Session | SystemVariableScopeKind::Local
            )
    )
}

fn safe_value(value: &SetVariableValue) -> bool {
    matches!(value, SetVariableValue::Expr { expr, .. } if safe_expr(expr))
}

fn safe_expr(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Literal { .. }
            | Expr::SessionVariable {
                kind: SessionVariableKind::User
                    | SessionVariableKind::System
                    | SessionVariableKind::SystemSession,
                ..
            }
    )
}

fn safe_charset_value(value: &SetVariableValue, source: &str, variable: &str) -> bool {
    let SetVariableValue::Expr { expr, .. } = value else {
        return false;
    };
    let Expr::Column { name, .. } = expr.as_ref() else {
        return false;
    };
    let [identifier] = name.0.as_slice() else {
        return false;
    };
    let span = identifier.meta.span;
    source
        .get(span.start() as usize..span.end() as usize)
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            match variable {
                "character_set_client" | "character_set_results" => {
                    matches!(name.as_str(), "utf8mb4" | "utf8mb3" | "utf8" | "latin1")
                }
                "collation_connection" => {
                    ["utf8mb4_", "utf8mb3_", "utf8_", "latin1_"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                }
                _ => false,
            }
        })
}

fn safe_sql_mode_literal(source: &str) -> bool {
    let value = source
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .or_else(|| {
            source
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        });
    let Some(value) = value else {
        return false;
    };
    value.is_empty()
        || value.split(',').all(|mode| {
            matches!(
                mode.to_ascii_uppercase().as_str(),
                "NO_AUTO_VALUE_ON_ZERO"
                    | "ONLY_FULL_GROUP_BY"
                    | "STRICT_TRANS_TABLES"
                    | "STRICT_ALL_TABLES"
                    | "ALLOW_INVALID_DATES"
                    | "TRADITIONAL"
                    | "NO_ZERO_IN_DATE"
                    | "NO_ZERO_DATE"
                    | "ERROR_FOR_DIVISION_BY_ZERO"
                    | "NO_ENGINE_SUBSTITUTION"
            )
        })
}
