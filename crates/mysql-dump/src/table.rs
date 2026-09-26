use std::ops::{ControlFlow, Range};

use sqlparser::ast::{
    ColumnOption, Expr, Ident, ObjectName, ObjectNamePart, Statement, TableConstraint, Visit,
    Visitor,
};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::Location;
use thiserror::Error;

use crate::reference::DatabaseReference;

const MAX_TABLE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum TableError {
    #[error("table statement is too large to inspect")]
    TooLarge,
    #[error("unsupported CREATE TABLE syntax: {message}")]
    UnsupportedSyntax { message: String },
    #[error("expected exactly one CREATE TABLE statement")]
    WrongStatement,
    #[error("CREATE TABLE form has references outside its table and foreign-key names")]
    UnsupportedForm,
    #[error("database identifier has no verified source span")]
    InvalidSpan,
}

/// Find database qualifiers in a CREATE TABLE definition that Squonk may not
/// parse. The original source must still pass import policy before execution.
pub fn table_database_references(source: &str) -> Result<Vec<DatabaseReference>, TableError> {
    if source.len() > MAX_TABLE_BYTES {
        return Err(TableError::TooLarge);
    }
    let parsed = Parser::parse_sql(&MySqlDialect {}, source).map_err(|error| {
        TableError::UnsupportedSyntax {
            message: error.to_string(),
        }
    })?;
    let [Statement::CreateTable(create)] = parsed.as_slice() else {
        return Err(TableError::WrongStatement);
    };
    if create.query.is_some()
        || create.like.is_some()
        || create.clone.is_some()
        || create.location.is_some()
        || create.external
    {
        return Err(TableError::UnsupportedForm);
    }
    let mut expression_check = ExpressionCheck;
    if let ControlFlow::Break(()) = parsed[0].visit(&mut expression_check) {
        return Err(TableError::UnsupportedForm);
    }
    let lines = SourceLines::new(source);
    let mut references = Vec::new();
    add_name(&mut references, &lines, &create.name)?;
    for constraint in &create.constraints {
        if let TableConstraint::ForeignKey(foreign_key) = constraint {
            add_name(&mut references, &lines, &foreign_key.foreign_table)?;
        }
    }
    for column in &create.columns {
        for option in &column.options {
            if let ColumnOption::ForeignKey(foreign_key) = &option.option {
                add_name(&mut references, &lines, &foreign_key.foreign_table)?;
            }
        }
    }
    references.sort_by_key(|reference| reference.span.start);
    references.dedup_by(|left, right| left.span == right.span);
    Ok(references)
}

struct ExpressionCheck;

impl Visitor for ExpressionCheck {
    type Break = ();

    fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
        match expression {
            Expr::CompoundIdentifier(_) => ControlFlow::Break(()),
            Expr::Function(function) if function.name.0.len() > 1 => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        }
    }
}

fn add_name(
    references: &mut Vec<DatabaseReference>,
    lines: &SourceLines<'_>,
    name: &ObjectName,
) -> Result<(), TableError> {
    let database = match name.0.as_slice() {
        [ObjectNamePart::Identifier(_)] => return Ok(()),
        [
            ObjectNamePart::Identifier(database),
            ObjectNamePart::Identifier(_),
        ] => database,
        _ => return Err(TableError::UnsupportedForm),
    };
    let span = lines.span(database)?;
    references.push(DatabaseReference {
        name: database.value.clone(),
        span,
    });
    Ok(())
}

struct SourceLines<'a> {
    source: &'a str,
    starts: Vec<usize>,
}

impl<'a> SourceLines<'a> {
    fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        Self { source, starts }
    }

    fn offset(&self, location: Location) -> Option<usize> {
        let line = usize::try_from(location.line.checked_sub(1)?).ok()?;
        let column = usize::try_from(location.column.checked_sub(1)?).ok()?;
        let start = *self.starts.get(line)?;
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());
        let source_line = self.source.get(start..end)?;
        let relative = source_line
            .char_indices()
            .nth(column)
            .map(|(offset, _)| offset)
            .or_else(|| (source_line.chars().count() == column).then_some(source_line.len()))?;
        start.checked_add(relative)
    }

    fn span(&self, identifier: &Ident) -> Result<Range<usize>, TableError> {
        let start = self
            .offset(identifier.span.start)
            .ok_or(TableError::InvalidSpan)?;
        let end = self
            .offset(identifier.span.end)
            .ok_or(TableError::InvalidSpan)?;
        let raw = self.source.get(start..end).ok_or(TableError::InvalidSpan)?;
        let decoded = match identifier.quote_style {
            None => raw.to_owned(),
            Some('`') => raw
                .strip_prefix('`')
                .and_then(|value| value.strip_suffix('`'))
                .ok_or(TableError::InvalidSpan)?
                .replace("``", "`"),
            _ => return Err(TableError::InvalidSpan),
        };
        if start >= end || decoded != identifier.value {
            return Err(TableError::InvalidSpan);
        }
        Ok(start..end)
    }
}
