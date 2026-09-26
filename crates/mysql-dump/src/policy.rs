use squonk::ast::Resolver;
use squonk::ast::generated::Visit;
use squonk::ast::generated::visit::{walk_expr, walk_statement};
use squonk::ast::{
    DropObjectKind, Expr, FunctionBody, FunctionOption, SessionStatement, SetVariableAssignment,
    Statement, SystemVariableScope, SystemVariableScopeKind,
};
use squonk::dialect::MySql;
use thiserror::Error;

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
}

/// Validate the complete AST of a normalized MySQL stored routine. The caller
/// must still remap and verify every database reference before execution.
pub fn validate_stored_routine_effects(analysis: &str) -> Result<(), StoredRoutinePolicyError> {
    let parsed = squonk::parse_with(analysis, squonk::ParseConfig::new(MySql))
        .map_err(|_| StoredRoutinePolicyError::UnsupportedSyntax)?;
    let [statement] = parsed.statements() else {
        return Err(StoredRoutinePolicyError::UnsupportedSyntax);
    };
    match statement {
        Statement::CreateFunction { create, .. } => {
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
        Statement::CreateProcedure { .. } => {}
        _ => return Err(StoredRoutinePolicyError::UnsupportedSyntax),
    }
    let mut visitor = PolicyVisitor {
        resolver: parsed.resolver(),
        depth: 0,
        error: None,
    };
    visitor.visit_statement(statement);
    visitor.error.map_or(Ok(()), Err)
}

struct PolicyVisitor<'a> {
    resolver: &'a dyn Resolver,
    depth: usize,
    error: Option<StoredRoutinePolicyError>,
}

impl<'ast> Visit<'ast> for PolicyVisitor<'_> {
    fn visit_statement(&mut self, statement: &'ast Statement) {
        if self.error.is_some() {
            return;
        }
        let allowed = if self.depth == 0 {
            matches!(
                statement,
                Statement::CreateFunction { .. } | Statement::CreateProcedure { .. }
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
        if let Statement::Session { session, .. } = statement
            && !safe_routine_session(session)
        {
            self.error = Some(StoredRoutinePolicyError::UnsafeStatement { kind: "SET" });
            return;
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
