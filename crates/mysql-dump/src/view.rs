use std::ops::{ControlFlow, Range};

use sqlparser::ast::{
    Expr, GranteeName, Ident, ObjectName, ObjectNamePart, Select, Statement, Visit, Visitor,
};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
use thiserror::Error;

use crate::reference::{DatabaseReference, StatementReferences};
use crate::span::SourceLines;

const MAX_VIEW_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ViewError {
    #[error("view statement is too large to inspect")]
    TooLarge,
    #[error("unsupported CREATE VIEW syntax: {message}")]
    UnsupportedSyntax { message: String },
    #[error("expected exactly one CREATE VIEW statement")]
    WrongStatement,
    #[error("unsupported CREATE VIEW form or expression")]
    UnsupportedForm,
    #[error("view database identifier has no verified source span")]
    InvalidSpan,
    #[error("view definer has no verified source span")]
    InvalidDefinerSpan,
}

/// Inspect a MySQL CREATE VIEW when the primary parser cannot parse its query.
/// Only source-verified qualifiers and the explicit definer are returned.
pub fn view_statement_references(source: &str) -> Result<StatementReferences, ViewError> {
    if source.len() > MAX_VIEW_BYTES {
        return Err(ViewError::TooLarge);
    }
    let analysis = expose_versioned_lines(source)?;
    let parsed = Parser::parse_sql(&MySqlDialect {}, &analysis).map_err(|error| {
        ViewError::UnsupportedSyntax {
            message: error.to_string(),
        }
    })?;
    let [Statement::CreateView(view)] = parsed.as_slice() else {
        return Err(ViewError::WrongStatement);
    };
    if view.or_alter
        || view.materialized
        || view.secure
        || view.temporary
        || view.to.is_some()
        || !view.cluster_by.is_empty()
        || view.comment.is_some()
        || view.with_no_schema_binding
        || view.copy_grants
        || !view.query.locks.is_empty()
        || view.query.settings.is_some()
        || view.query.format_clause.is_some()
        || !view.query.pipe_operators.is_empty()
    {
        return Err(ViewError::UnsupportedForm);
    }
    let lines = SourceLines::new(source);
    let mut collector = ViewCollector {
        lines: &lines,
        references: StatementReferences::default(),
        error: None,
    };
    collector.add_name(&view.name);
    if let Some(definer) = view
        .params
        .as_ref()
        .and_then(|params| params.definer.as_ref())
    {
        collector
            .references
            .definers
            .push(definer_span(source, &lines, definer)?);
    }
    if let ControlFlow::Break(()) = view.query.visit(&mut collector) {
        return Err(collector.error.unwrap_or(ViewError::UnsupportedForm));
    }
    if let Some(error) = collector.error {
        return Err(error);
    }
    collector
        .references
        .databases
        .sort_by_key(|reference| reference.span.start);
    collector
        .references
        .databases
        .dedup_by(|left, right| left.span == right.span);
    Ok(collector.references)
}

struct ViewCollector<'a, 'b> {
    lines: &'a SourceLines<'b>,
    references: StatementReferences,
    error: Option<ViewError>,
}

impl ViewCollector<'_, '_> {
    fn add_identifier(&mut self, identifier: &Ident) {
        let Some(span) = self.lines.span(identifier) else {
            self.error = Some(ViewError::InvalidSpan);
            return;
        };
        self.references.databases.push(DatabaseReference {
            name: identifier.value.clone(),
            span,
        });
    }

    fn add_name(&mut self, name: &ObjectName) {
        match name.0.as_slice() {
            [ObjectNamePart::Identifier(_)] => {}
            [
                ObjectNamePart::Identifier(database),
                ObjectNamePart::Identifier(_),
            ] => {
                self.add_identifier(database);
            }
            _ => self.error = Some(ViewError::UnsupportedForm),
        }
    }
}

impl Visitor for ViewCollector<'_, '_> {
    type Break = ();

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        if select.into.is_some() {
            self.error = Some(ViewError::UnsupportedForm);
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn pre_visit_relation(&mut self, name: &ObjectName) -> ControlFlow<Self::Break> {
        self.add_name(name);
        if self.error.is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
        match expression {
            Expr::CompoundIdentifier(parts) if parts.len() == 3 => {
                self.add_identifier(&parts[0]);
            }
            Expr::Function(function)
                if function
                    .name
                    .0
                    .last()
                    .and_then(ObjectNamePart::as_ident)
                    .is_some_and(|name| name.value.eq_ignore_ascii_case("LOAD_FILE")) =>
            {
                self.error = Some(ViewError::UnsupportedForm);
            }
            Expr::Function(function) if function.name.0.len() > 1 => {
                self.add_name(&function.name);
            }
            _ => {}
        }
        if self.error.is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

fn definer_span(
    source: &str,
    lines: &SourceLines<'_>,
    definer: &GranteeName,
) -> Result<Range<usize>, ViewError> {
    let GranteeName::UserHost { user, host } = definer else {
        return Err(ViewError::UnsupportedForm);
    };
    let user_span = lines.span(user).ok_or(ViewError::InvalidDefinerSpan)?;
    let host_span = lines.span(host).ok_or(ViewError::InvalidDefinerSpan)?;
    if source.get(user_span.end..host_span.start).map(str::trim) != Some("@") {
        return Err(ViewError::InvalidDefinerSpan);
    }
    let prefix = source
        .get(..user_span.start)
        .ok_or(ViewError::InvalidDefinerSpan)?;
    let start = prefix
        .as_bytes()
        .windows(7)
        .rposition(|bytes| bytes.eq_ignore_ascii_case(b"DEFINER"))
        .ok_or(ViewError::InvalidDefinerSpan)?;
    if source.get(start + 7..user_span.start).map(str::trim) != Some("=") {
        return Err(ViewError::InvalidDefinerSpan);
    }
    Ok(start..host_span.end)
}

/// `mysqldump` puts view options on lines such as `/*!50013 DEFINER=... */`.
/// Expose only complete versioned-comment lines, preserving every byte offset.
fn expose_versioned_lines(source: &str) -> Result<String, ViewError> {
    let mut analysis = source.as_bytes().to_vec();
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let line_bytes = line.as_bytes();
        let leading = line_bytes
            .iter()
            .take_while(|byte| **byte == b' ' || **byte == b'\t')
            .count();
        let remaining = &line_bytes[leading..];
        if let Some(relative_opening) = remaining.windows(3).position(|bytes| bytes == b"/*!") {
            let opening = leading + relative_opening;
            let prefix = line_bytes[leading..opening].trim_ascii();
            if !prefix.is_empty() && !prefix.eq_ignore_ascii_case(b"CREATE") {
                return Err(ViewError::UnsupportedForm);
            }
            let digit_count = line_bytes[opening + 3..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let opener_end = opening + 3 + digit_count;
            let mut closing_end = line_bytes
                .iter()
                .rposition(|byte| !byte.is_ascii_whitespace())
                .map(|index| index + 1)
                .ok_or(ViewError::UnsupportedForm)?;
            if line_bytes.get(closing_end - 1) == Some(&b';') {
                closing_end -= 1;
                while closing_end > 0 && line_bytes[closing_end - 1].is_ascii_whitespace() {
                    closing_end -= 1;
                }
            }
            if !(5..=6).contains(&digit_count)
                || !line_bytes
                    .get(opener_end)
                    .is_some_and(u8::is_ascii_whitespace)
                || closing_end < opener_end + 3
                || line_bytes.get(closing_end - 2..closing_end) != Some(b"*/")
            {
                return Err(ViewError::UnsupportedForm);
            }
            analysis[line_start + opening..line_start + opener_end].fill(b' ');
            analysis[line_start + closing_end - 2..line_start + closing_end].fill(b' ');
        }
        line_start += line_bytes.len();
    }
    String::from_utf8(analysis).map_err(|_| ViewError::UnsupportedForm)
}
