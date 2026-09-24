use std::io;

use comfy_table::presets::NOTHING;
use comfy_table::{ContentArrangement, Table as ComfyTable};
use textwrap::core::display_width;

use super::{INDENT, Line, Output, Tone};

/// Space between report columns.
const COLUMN_GAP: u16 = 2;

/// A report table: one header row and one row of cells per record, each
/// optionally followed by detail lines about that record.
pub(crate) struct Table {
    headers: Vec<String>,
    rows: Vec<Row>,
}

struct Row {
    cells: Vec<Line>,
    details: Vec<Line>,
}

impl Table {
    pub(crate) fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(ToString::to_string).collect(),
            rows: Vec::new(),
        }
    }

    pub(crate) fn row(&mut self, cells: Vec<Line>) {
        self.rows.push(Row {
            cells,
            details: Vec::new(),
        });
    }

    /// Adds a detail line under the most recent row.
    pub(crate) fn detail(&mut self, line: impl Into<Line>) {
        if let Some(row) = self.rows.last_mut() {
            row.details.push(line.into());
        }
    }
}

impl Output<'_> {
    /// Writes a report table.
    ///
    /// A plain surface gets the header and rows joined by two spaces. A
    /// decorated surface gets aligned borderless columns, or, when those do
    /// not fit the width, one stacked block of `label value` lines per record
    /// so no field is dropped and nothing scrolls horizontally.
    pub(crate) fn table(&mut self, table: &Table) -> io::Result<()> {
        self.continuation = INDENT.to_string();
        if !self.surface.decorated {
            self.line(&table.headers.join("  "))?;
            for row in &table.rows {
                let cells = row.cells.iter().map(Line::plain).collect::<Vec<_>>();
                self.line(&cells.join("  "))?;
                self.row_details(row)?;
            }

            return Ok(());
        }

        let color = self.surface.color;
        let mut columns = ComfyTable::new();
        columns
            .load_style(NOTHING)
            .set_content_arrangement(ContentArrangement::Disabled)
            .set_header(
                table
                    .headers
                    .iter()
                    .map(|header| Tone::Dim.paint(&header.to_uppercase(), color)),
            );
        for row in &table.rows {
            columns.add_row(
                row.cells
                    .iter()
                    .map(|cell| cell_text(cell, Tone::Plain, color)),
            );
        }
        for column in columns.column_iter_mut() {
            column.set_padding((0, COLUMN_GAP));
        }
        let lines = columns
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>();
        if lines
            .iter()
            .any(|line| display_width(line) > self.surface.width)
        {
            return self.stacked(table);
        }

        let mut lines = lines.into_iter();
        if let Some(header) = lines.next() {
            self.line(&header)?;
        }
        for (row, line) in table.rows.iter().zip(lines) {
            self.line(&line)?;
            self.row_details(row)?;
        }

        Ok(())
    }

    fn stacked(&mut self, table: &Table) -> io::Result<()> {
        let color = self.surface.color;
        let label_width = table
            .headers
            .iter()
            .skip(1)
            .map(|header| display_width(header))
            .max()
            .unwrap_or(0);
        for (index, row) in table.rows.iter().enumerate() {
            if index > 0 {
                writeln!(self.writer)?;
            }
            let mut cells = row.cells.iter();
            if let Some(title) = cells.next() {
                self.line(&cell_text(title, Tone::Strong, color))?;
            }
            // Values are never wrapped: they are paths, hostnames, and ids.
            for (header, cell) in table.headers.iter().skip(1).zip(cells) {
                let label = format!("{:label_width$}", header.to_lowercase());
                writeln!(
                    self.writer,
                    "{INDENT}{}  {}",
                    Tone::Dim.paint(&label, color),
                    cell_text(cell, Tone::Plain, color)
                )?;
            }
            self.row_details(row)?;
        }

        Ok(())
    }

    fn row_details(&mut self, row: &Row) -> io::Result<()> {
        for detail in &row.details {
            self.detail(detail.clone())?;
        }

        Ok(())
    }
}

/// A decorated cell on one line: comfy-table would split a multi-line cell
/// across rows and shift every detail after it.
fn cell_text(cell: &Line, base: Tone, color: bool) -> String {
    cell.paint(base, color).replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::super::tests::visible;
    use super::*;
    use crate::output::{Mark, Surface};

    fn projects() -> Table {
        let mut table = Table::new(&["Project", "Mode", "PHP", "Status", "Env", "Path"]);
        table.row(vec![
            Line::from("acme.test"),
            Line::from("served"),
            Line::from("8.5"),
            Line::marked(Mark::Running, "ok"),
            Line::from("rendered"),
            Line::default().value("/Users/me/Code/acme"),
        ]);
        table.row(vec![
            Line::from("invoicer"),
            Line::from("resource-only"),
            Line::from("default"),
            Line::marked(Mark::Idle, "unknown"),
            Line::default().toned(Tone::Warning, "warning"),
            Line::default().value("/Users/me/Code/clients/invoicer-platform"),
        ]);
        table.detail("env: warning: APP_URL already exists outside the PV block");
        table
    }

    fn render(table: &Table, surface: Surface) -> String {
        let mut bytes = Vec::new();
        let mut output = Output::new(&mut bytes, surface);
        if let Err(error) = output.table(table) {
            return format!("write failed: {error}");
        }

        visible(bytes)
    }

    #[test]
    fn plain_tables_join_cells_and_keep_status_words() {
        assert_snapshot!(render(&projects(), Surface::plain()), @"
        Project  Mode  PHP  Status  Env  Path
        acme.test  served  8.5  ok  rendered  /Users/me/Code/acme
        invoicer  resource-only  default  unknown  warning  /Users/me/Code/clients/invoicer-platform
          env: warning: APP_URL already exists outside the PV block
        ");
    }

    #[test]
    fn decorated_tables_align_columns_when_they_fit() {
        assert_snapshot!(render(&projects(), Surface::terminal(false, 120)), @"
        PROJECT    MODE           PHP      STATUS     ENV       PATH
        acme.test  served         8.5      ● ok       rendered  /Users/me/Code/acme
        invoicer   resource-only  default  ○ unknown  warning   /Users/me/Code/clients/invoicer-platform
           env: warning: APP_URL already exists outside the PV block
        ");
    }

    #[test]
    fn decorated_tables_stack_records_when_columns_do_not_fit() {
        assert_snapshot!(render(&projects(), Surface::terminal(false, 80)), @"
        acme.test
           mode    served
           php     8.5
           status  ● ok
           env     rendered
           path    /Users/me/Code/acme

        invoicer
           mode    resource-only
           php     default
           status  ○ unknown
           env     warning
           path    /Users/me/Code/clients/invoicer-platform
           env: warning: APP_URL already exists outside the PV block
        ");
        assert_snapshot!(render(&projects(), Surface::terminal(false, 50)), @"
        acme.test
           mode    served
           php     8.5
           status  ● ok
           env     rendered
           path    /Users/me/Code/acme

        invoicer
           mode    resource-only
           php     default
           status  ○ unknown
           env     warning
           path    /Users/me/Code/clients/invoicer-platform
           env: warning: APP_URL already exists outside
           the PV block
        ");
    }

    #[test]
    fn decorated_tables_keep_multi_line_cells_on_their_row() {
        let mut table = Table::new(&["ID", "Status", "Summary"]);
        table.row(vec![
            Line::from("job_1"),
            Line::marked(Mark::Failure, "failed"),
            Line::from("exit 1\nport 3306 in use"),
        ]);
        table.detail("scope: mysql:8.4");
        table.row(vec![
            Line::from("job_2"),
            Line::marked(Mark::Success, "succeeded"),
            Line::from("no changes"),
        ]);
        assert_snapshot!(render(&table, Surface::terminal(false, 80)), @"
        ID     STATUS       SUMMARY
        job_1  ✗ failed     exit 1 port 3306 in use
           scope: mysql:8.4
        job_2  ✓ succeeded  no changes
        ");
    }

    #[test]
    fn decorated_tables_measure_colored_cells_by_visible_width() {
        assert_snapshot!(render(&projects(), Surface::terminal(true, 120)), @"
        ␛[2mPROJECT␛[0m    ␛[2mMODE␛[0m           ␛[2mPHP␛[0m      ␛[2mSTATUS␛[0m     ␛[2mENV␛[0m       ␛[2mPATH␛[0m
        acme.test  served         8.5      ␛[32m●␛[0m ␛[32mok␛[0m       rendered  ␛[36m/Users/me/Code/acme␛[0m
        invoicer   resource-only  default  ␛[2m○␛[0m ␛[2munknown␛[0m  ␛[33mwarning␛[0m   ␛[36m/Users/me/Code/clients/invoicer-platform␛[0m
           ␛[2menv: warning: APP_URL already exists outside the PV block␛[0m
        ");
    }
}
