use std::collections::BTreeSet;

use squonk::ast::Resolver;
use squonk::ast::generated::Visit;
use squonk::ast::generated::visit::{walk_expr, walk_statement};
use squonk::ast::{
    CreateTableOptionKind, DropObjectKind, Expr, FunctionBody, FunctionOption, SessionStatement,
    SetVariableAssignment, Statement, SystemVariableScope, SystemVariableScopeKind,
    TableOptionValue,
};
use squonk::dialect::MySql;
use thiserror::Error;

use crate::table::{safe_table_engine, safe_table_option_name};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoredRoutinePolicyError {
    #[error("unsupported stored routine syntax")]
    UnsupportedSyntax,
    #[error("stored routine has an opaque or missing SQL body")]
    OpaqueBody,
    #[error("stored routine contains an unsupported or unsafe {kind} statement")]
    UnsafeStatement { kind: &'static str },
    #[error("stored routine calls a server-side file function")]
    ServerFileFunction,
    #[error("stored routine TRUNCATE table is not in a mapped target database")]
    UnmappedTruncate,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StatementPolicyError {
    #[error("unsupported MySQL syntax")]
    UnsupportedSyntax,
    #[error("statement can affect shared MySQL state or has an unsupported form")]
    UnsafeStatement,
    #[error("statement calls a server-side file function")]
    ServerFileFunction,
}

/// Positive allowlist for ordinary, complete dump statements. Routing and
/// session setup are checked separately before calling this function.
pub fn validate_statement_effects(source: &str) -> Result<(), StatementPolicyError> {
    let parsed = squonk::parse_with(source, squonk::ParseConfig::new(MySql))
        .map_err(|_| StatementPolicyError::UnsupportedSyntax)?;
    let [statement] = parsed.statements() else {
        return Err(StatementPolicyError::UnsupportedSyntax);
    };
    let allowed = match statement {
        Statement::AlterTable { .. }
        | Statement::CreateView { .. }
        | Statement::CreateIndex { .. }
        | Statement::DropIndex { .. }
        | Statement::Insert { .. }
        | Statement::Update { .. }
        | Statement::Delete { .. }
        | Statement::Transaction { .. }
        | Statement::LockTables { .. }
        | Statement::UnlockTables { .. }
        | Statement::DropEvent { .. }
        | Statement::DropRoutine { .. } => true,
        Statement::Drop { drop, .. } => {
            matches!(
                drop.object_kind,
                DropObjectKind::Table | DropObjectKind::View | DropObjectKind::Trigger
            ) && drop.behavior.is_none()
        }
        _ => false,
    };
    if !allowed {
        return Err(StatementPolicyError::UnsafeStatement);
    }
    let mut visitor = FileFunctionVisitor {
        resolver: parsed.resolver(),
        found: false,
    };
    visitor.visit_statement(statement);
    if visitor.found {
        Err(StatementPolicyError::ServerFileFunction)
    } else {
        Ok(())
    }
}

/// A trigger or event body is executable later; inspect its nested SQL before
/// accepting a root-owned definition. `TRUNCATE` remains unsupported here.
pub fn validate_stored_object_effects(source: &str) -> Result<(), StoredRoutinePolicyError> {
    validate_effects(source, None, None, true).map(|_| ())
}

/// Validate the complete AST of a normalized MySQL stored routine. The caller
/// must still remap and verify every database reference before execution.
pub fn validate_stored_routine_effects(analysis: &str) -> Result<(), StoredRoutinePolicyError> {
    validate_effects(analysis, None, None, false).map(|_| ())
}

/// Permit a routine's `TRUNCATE` only when every table resolves to a mapped
/// source database. Returns whether the routine contains such a statement so
/// the import plan can disclose it before execution.
pub fn validate_stored_routine_effects_for_targets(
    analysis: &str,
    active_database: Option<&str>,
    mapped_databases: &BTreeSet<String>,
) -> Result<bool, StoredRoutinePolicyError> {
    validate_effects(analysis, active_database, Some(mapped_databases), false)
}

fn validate_effects(
    analysis: &str,
    active_database: Option<&str>,
    mapped_databases: Option<&BTreeSet<String>>,
    stored_object: bool,
) -> Result<bool, StoredRoutinePolicyError> {
    let parsed = squonk::parse_with(analysis, squonk::ParseConfig::new(MySql))
        .map_err(|_| StoredRoutinePolicyError::UnsupportedSyntax)?;
    let [statement] = parsed.statements() else {
        return Err(StoredRoutinePolicyError::UnsupportedSyntax);
    };
    match statement {
        Statement::CreateFunction { create, .. } if !stored_object => {
            if create.body.is_none()
                || create
                    .options
                    .iter()
                    .any(|option| matches!(option, FunctionOption::As { .. }))
                || create
                    .body
                    .as_deref()
                    .is_some_and(|body| matches!(body, FunctionBody::Definition { .. }))
            {
                return Err(StoredRoutinePolicyError::OpaqueBody);
            }
        }
        Statement::CreateProcedure { .. } if !stored_object => {}
        Statement::CreateStoredTrigger { .. } | Statement::CreateEvent { .. } if stored_object => {}
        _ => return Err(StoredRoutinePolicyError::UnsupportedSyntax),
    }
    let mut visitor = PolicyVisitor {
        resolver: parsed.resolver(),
        depth: 0,
        error: None,
        active_database,
        mapped_databases,
        has_truncate: false,
    };
    visitor.visit_statement(statement);
    visitor.error.map_or(Ok(visitor.has_truncate), Err)
}

struct PolicyVisitor<'a> {
    resolver: &'a dyn Resolver,
    depth: usize,
    error: Option<StoredRoutinePolicyError>,
    active_database: Option<&'a str>,
    mapped_databases: Option<&'a BTreeSet<String>>,
    has_truncate: bool,
}

impl<'ast> Visit<'ast> for PolicyVisitor<'_> {
    fn visit_statement(&mut self, statement: &'ast Statement) {
        if self.error.is_some() {
            return;
        }
        let allowed = if self.depth == 0 {
            matches!(
                statement,
                Statement::CreateFunction { .. }
                    | Statement::CreateProcedure { .. }
                    | Statement::CreateStoredTrigger { .. }
                    | Statement::CreateEvent { .. }
            )
        } else {
            matches!(
                statement,
                Statement::Query { .. }
                    | Statement::Insert { .. }
                    | Statement::Update { .. }
                    | Statement::Delete { .. }
                    | Statement::Call { .. }
                    | Statement::Do { .. }
                    | Statement::DoExpressions { .. }
                    | Statement::Compound { .. }
                    | Statement::If { .. }
                    | Statement::Case { .. }
                    | Statement::Loop { .. }
                    | Statement::While { .. }
                    | Statement::Repeat { .. }
                    | Statement::Leave { .. }
                    | Statement::Iterate { .. }
                    | Statement::Return { .. }
                    | Statement::OpenCursor { .. }
                    | Statement::FetchCursor { .. }
                    | Statement::CloseCursor { .. }
                    | Statement::Signal { .. }
                    | Statement::Resignal { .. }
                    | Statement::GetDiagnostics { .. }
                    | Statement::CreateTable { .. }
                    | Statement::Drop { .. }
                    | Statement::Session { .. }
                    | Statement::Transaction { .. }
                    | Statement::Truncate { .. }
            )
        };
        if !allowed {
            self.error = Some(StoredRoutinePolicyError::UnsafeStatement {
                kind: statement_kind(statement),
            });
            return;
        }
        if let Statement::Drop { drop, .. } = statement
            && drop.object_kind != DropObjectKind::Table
        {
            self.error = Some(StoredRoutinePolicyError::UnsafeStatement { kind: "DROP" });
            return;
        }
        if let Statement::CreateTable { create, .. } = statement
            && create.options.iter().any(|option| {
                let CreateTableOptionKind::KeyValue { option, .. } = &option.kind else {
                    return true;
                };
                let Some(name) = self.resolver.try_resolve(option.name.sym) else {
                    return true;
                };
                if name.eq_ignore_ascii_case("ENGINE") {
                    let TableOptionValue::Word { word, .. } = &option.value else {
                        return true;
                    };
                    !self
                        .resolver
                        .try_resolve(word.sym)
                        .is_some_and(safe_table_engine)
                } else {
                    !safe_table_option_name(name) && !name.eq_ignore_ascii_case("COMMENT")
                }
            })
        {
            self.error = Some(StoredRoutinePolicyError::UnsafeStatement {
                kind: "CREATE TABLE option",
            });
            return;
        }
        if let Statement::Session { session, .. } = statement
            && !safe_routine_session(session)
        {
            self.error = Some(StoredRoutinePolicyError::UnsafeStatement { kind: "SET" });
            return;
        }
        if let Statement::Truncate {
            tables,
            restart_identity,
            behavior,
            ..
        } = statement
        {
            let Some(mapped_databases) = self.mapped_databases else {
                self.error = Some(StoredRoutinePolicyError::UnmappedTruncate);
                return;
            };
            if tables.is_empty() || restart_identity.is_some() || behavior.is_some() {
                self.error = Some(StoredRoutinePolicyError::UnsafeStatement { kind: "TRUNCATE" });
                return;
            }
            for table in tables {
                let database = match table.0.as_slice() {
                    [_] => self.active_database,
                    [database, _] => self.resolver.try_resolve(database.sym),
                    _ => None,
                };
                if !database.is_some_and(|database| mapped_databases.contains(database)) {
                    self.error = Some(StoredRoutinePolicyError::UnmappedTruncate);
                    return;
                }
            }
            self.has_truncate = true;
        }
        self.depth += 1;
        walk_statement(self, statement);
        self.depth -= 1;
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Expr::Function { call, .. } = expression
            && let Some(name) = call.name.0.last()
            && self
                .resolver
                .try_resolve(name.sym)
                .is_some_and(|name| name.eq_ignore_ascii_case("LOAD_FILE"))
        {
            self.error = Some(StoredRoutinePolicyError::ServerFileFunction);
            return;
        }
        walk_expr(self, expression);
    }
}

struct FileFunctionVisitor<'a> {
    resolver: &'a dyn Resolver,
    found: bool,
}

impl<'ast> Visit<'ast> for FileFunctionVisitor<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if calls_load_file(expression, self.resolver) {
            self.found = true;
        } else {
            walk_expr(self, expression);
        }
    }
}

fn calls_load_file(expression: &Expr, resolver: &dyn Resolver) -> bool {
    matches!(expression, Expr::Function { call, .. }
        if call.name.0.last().and_then(|name| resolver.try_resolve(name.sym))
            .is_some_and(|name| name.eq_ignore_ascii_case("LOAD_FILE")))
}

fn statement_kind(statement: &Statement) -> &'static str {
    match statement {
        Statement::Prepare { .. }
        | Statement::PrepareFrom { .. }
        | Statement::Execute { .. }
        | Statement::ExecuteUsing { .. }
        | Statement::Deallocate { .. } => "dynamic SQL",
        Statement::Truncate { .. } => "TRUNCATE",
        Statement::AlterTable { .. } => "ALTER TABLE",
        Statement::CreateIndex { .. } => "CREATE INDEX",
        Statement::DropIndex { .. } => "DROP INDEX",
        Statement::Rename { .. } => "RENAME",
        Statement::LockTables { .. } => "LOCK TABLES",
        Statement::UnlockTables { .. } => "UNLOCK TABLES",
        Statement::Other { .. } => "unknown",
        _ => "server-scoped",
    }
}

fn safe_routine_session(session: &SessionStatement) -> bool {
    match session {
        SessionStatement::SetVariables { assignments, .. } => {
            assignments.iter().all(|assignment| match assignment {
                SetVariableAssignment::UserVariable { .. } => true,
                SetVariableAssignment::SystemVariable { scope, .. } => matches!(
                    scope,
                    SystemVariableScope::Implicit
                        | SystemVariableScope::AtAt
                        | SystemVariableScope::Keyword(
                            SystemVariableScopeKind::Session | SystemVariableScopeKind::Local
                        )
                        | SystemVariableScope::AtAtScoped(
                            SystemVariableScopeKind::Session | SystemVariableScopeKind::Local
                        )
                ),
            })
        }
        _ => false,
    }
}
