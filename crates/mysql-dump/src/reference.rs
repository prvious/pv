use std::ops::Range;

use squonk::ast::Resolver;
use squonk::ast::generated::Visit;
use squonk::ast::generated::visit::walk_expr;
use squonk::ast::{Definer, Expr, Ident, ObjectName, Statement};
use squonk::dialect::MySql;
use thiserror::Error;

const MAX_ANALYSIS_BYTES: usize = 8 * 1024 * 1024;

/// Database qualifier found in a parsed SQL object reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseReference {
    pub name: String,
    pub span: Range<usize>,
}

/// Identifier references and explicit source accounts in one SQL statement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatementReferences {
    pub databases: Vec<DatabaseReference>,
    pub definers: Vec<Range<usize>>,
}

#[derive(Debug, Error)]
pub enum ReferenceError {
    #[error("statement is too large for AST analysis")]
    TooLarge,
    #[error("unsupported MySQL syntax: {message}")]
    UnsupportedSyntax { message: String },
    #[error("expected exactly one SQL statement")]
    MultipleStatements,
    #[error("database qualifier has no verified source span")]
    InvalidSpan,
}

/// Find database qualifiers in one AST-parseable statement. A caller may pass
/// a same-length analysis normalization, then apply these spans to the original
/// source only after verifying the original identifier bytes. This does not
/// authorize the statement for execution.
pub fn statement_references(source: &str) -> Result<StatementReferences, ReferenceError> {
    if source.len() > MAX_ANALYSIS_BYTES {
        return Err(ReferenceError::TooLarge);
    }
    let parsed = squonk::parse_with(source, squonk::ParseConfig::new(MySql)).map_err(|error| {
        ReferenceError::UnsupportedSyntax {
            message: error.to_string(),
        }
    })?;
    let [statement] = parsed.statements() else {
        return Err(ReferenceError::MultipleStatements);
    };
    let mut collector = ReferenceCollector {
        source,
        resolver: parsed.resolver(),
        references: StatementReferences::default(),
        invalid_span: false,
    };
    if !matches!(statement, Statement::Session { .. }) {
        collector.visit_statement(statement);
    }
    if collector.invalid_span {
        return Err(ReferenceError::InvalidSpan);
    }
    collector
        .references
        .databases
        .sort_by_key(|reference| reference.span.start);
    collector
        .references
        .databases
        .dedup_by(|left, right| left.span == right.span);
    collector.references.definers.sort_by_key(|span| span.start);
    collector.references.definers.dedup();
    Ok(collector.references)
}

pub fn database_references(source: &str) -> Result<Vec<DatabaseReference>, ReferenceError> {
    Ok(statement_references(source)?.databases)
}

struct ReferenceCollector<'a> {
    source: &'a str,
    resolver: &'a dyn Resolver,
    references: StatementReferences,
    invalid_span: bool,
}

impl ReferenceCollector<'_> {
    fn add(&mut self, identifier: &Ident) {
        let span = identifier.meta.span.start() as usize..identifier.meta.span.end() as usize;
        let Some(name) = self.resolver.try_resolve(identifier.sym) else {
            self.invalid_span = true;
            return;
        };
        if span.start >= span.end || self.source.get(span.clone()).is_none() {
            self.invalid_span = true;
            return;
        }
        self.references.databases.push(DatabaseReference {
            name: name.to_owned(),
            span,
        });
    }
}

impl<'ast> Visit<'ast> for ReferenceCollector<'_> {
    fn visit_definer(&mut self, definer: &'ast Definer) {
        if let Definer::Account { meta, .. } = definer {
            let span = meta.span.start() as usize..meta.span.end() as usize;
            if span.start >= span.end || self.source.get(span.clone()).is_none() {
                self.invalid_span = true;
            } else {
                self.references.definers.push(span);
            }
        }
    }

    fn visit_object_name(&mut self, name: &'ast ObjectName) {
        if name.0.len() >= 2
            && let Some(database) = name.0.first()
        {
            self.add(database);
        }
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Expr::Column { name, .. } = expression {
            // `alias.column` is a two-part column reference, not a database.
            // Three-part names identify database.table.column in MySQL.
            if name.0.len() == 3
                && let Some(database) = name.0.first()
            {
                self.add(database);
            }
        } else {
            walk_expr(self, expression);
        }
    }
}
