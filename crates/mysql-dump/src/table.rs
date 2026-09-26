use std::ops::ControlFlow;

use sqlparser::ast::{
    ColumnOption, CreateTableOptions, Expr, ObjectName, ObjectNamePart, SqlOption, Statement,
    TableConstraint, Visit, Visitor,
};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
use thiserror::Error;

use crate::reference::DatabaseReference;
use crate::span::SourceLines;

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
    if source.contains("/*!") {
        return Err(TableError::UnsupportedForm);
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
    let options = match &create.table_options {
        CreateTableOptions::None => &[][..],
        CreateTableOptions::Plain(options) => options.as_slice(),
        _ => return Err(TableError::UnsupportedForm),
    };
    for option in options {
        let safe = match option {
            SqlOption::Comment(_) => true,
            SqlOption::KeyValue { key, .. } => safe_table_option_name(&key.value),
            SqlOption::NamedParenthesizedList(engine) => {
                engine.key.value.eq_ignore_ascii_case("ENGINE")
                    && engine.values.is_empty()
                    && engine
                        .name
                        .as_ref()
                        .is_some_and(|name| safe_table_engine(&name.value))
            }
            _ => false,
        };
        if !safe {
            return Err(TableError::UnsupportedForm);
        }
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

pub(crate) fn safe_table_option_name(name: &str) -> bool {
    [
        "AUTO_INCREMENT",
        "AVG_ROW_LENGTH",
        "CHARACTER SET",
        "CHARSET",
        "CHECKSUM",
        "COLLATE",
        "COMPRESSION",
        "DEFAULT CHARACTER SET",
        "DEFAULT CHARSET",
        "DEFAULT COLLATE",
        "DELAY_KEY_WRITE",
        "KEY_BLOCK_SIZE",
        "MAX_ROWS",
        "MIN_ROWS",
        "PACK_KEYS",
        "ROW_FORMAT",
        "STATS_AUTO_RECALC",
        "STATS_PERSISTENT",
        "STATS_SAMPLE_PAGES",
    ]
    .iter()
    .any(|allowed| name.eq_ignore_ascii_case(allowed))
}

pub(crate) fn safe_table_engine(name: &str) -> bool {
    ["INNODB", "MYISAM", "MEMORY", "CSV", "ARCHIVE"]
        .iter()
        .any(|allowed| name.eq_ignore_ascii_case(allowed))
}

struct ExpressionCheck;

impl Visitor for ExpressionCheck {
    type Break = ();

    fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
        match expression {
            Expr::CompoundIdentifier(_) => ControlFlow::Break(()),
            Expr::Function(function)
                if function.name.0.len() > 1
                    || function
                        .name
                        .0
                        .first()
                        .and_then(ObjectNamePart::as_ident)
                        .is_some_and(|name| name.value.eq_ignore_ascii_case("LOAD_FILE")) =>
            {
                ControlFlow::Break(())
            }
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
    let span = lines.span(database).ok_or(TableError::InvalidSpan)?;
    references.push(DatabaseReference {
        name: database.value.clone(),
        span,
    });
    Ok(())
}
