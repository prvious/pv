use std::ffi::OsString;
use std::io::{self, Write};

use anstyle::{AnsiColor, Effects, Style};
use serde::Serialize;
use textwrap::{Options, WordSeparator, WordSplitter};

use crate::environment::Environment;

mod table;

pub(crate) use table::Table;

const DEFAULT_WIDTH: usize = 80;
/// Wrapped text never gets narrower than this, even in a tiny terminal.
const MIN_TEXT_WIDTH: usize = 20;
/// One indent unit: the glyph column (`✓  `).
const INDENT: &str = "   ";
const GUTTER: &str = "│";

/// How one output stream is rendered.
///
/// A stream is *decorated* when it is a terminal: it may use glyphs, gutters,
/// headings, and tables. It uses color only when it is decorated and color is
/// not disabled by `NO_COLOR` or `--no-color`. A plain stream receives the
/// documented plain text only.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Surface {
    decorated: bool,
    color: bool,
    width: usize,
}

impl Surface {
    #[cfg(test)]
    pub(crate) fn plain() -> Self {
        Self {
            decorated: false,
            color: false,
            width: DEFAULT_WIDTH,
        }
    }

    #[cfg(test)]
    pub(crate) fn terminal(color: bool, width: usize) -> Self {
        Self {
            decorated: true,
            color,
            width,
        }
    }

    pub(crate) fn decorated(self) -> bool {
        self.decorated
    }

    pub(crate) fn color(self) -> bool {
        self.color
    }

    pub(crate) fn width(self) -> usize {
        self.width
    }
}

/// The presentation facts that change rendering, observed once at the CLI
/// boundary.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Presentation {
    pub(crate) stdout: Surface,
    pub(crate) stderr: Surface,
    /// Whether stdin and stderr are both terminals, so a prompt can own them.
    pub(crate) interactive: bool,
}

impl Presentation {
    pub(crate) fn detect(args: &[OsString], environment: &impl Environment) -> Self {
        let no_color = args.iter().any(|argument| argument == "--no-color")
            || environment.var_os("NO_COLOR").is_some();
        let stdout_is_terminal = environment.stdout_is_terminal();
        let stderr_is_terminal = environment.stderr_is_terminal();
        let width = if stdout_is_terminal || stderr_is_terminal {
            environment.terminal_width().filter(|width| *width > 0)
        } else {
            None
        }
        .unwrap_or(DEFAULT_WIDTH);
        let surface = |terminal: bool| Surface {
            decorated: terminal,
            color: terminal && !no_color,
            width,
        };

        Self {
            stdout: surface(stdout_is_terminal),
            stderr: surface(stderr_is_terminal),
            interactive: environment.stdin_is_terminal() && stderr_is_terminal,
        }
    }

    #[cfg(test)]
    pub(crate) fn plain() -> Self {
        Self {
            stdout: Surface::plain(),
            stderr: Surface::plain(),
            interactive: false,
        }
    }
}

/// The semantic role of a piece of text, mapped to one ANSI style.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Tone {
    Plain,
    /// Ports, versions, paths, hostnames, job ids, and commands.
    Value,
    /// Labels and explanations.
    Dim,
    Strong,
    Success,
    Warning,
    Failure,
    Accent,
}

impl Tone {
    fn style(self) -> Style {
        match self {
            Self::Plain => Style::new(),
            Self::Value => Style::new().fg_color(Some(AnsiColor::Cyan.into())),
            Self::Dim => Style::new().effects(Effects::DIMMED),
            Self::Strong => Style::new().effects(Effects::BOLD),
            Self::Success => Style::new().fg_color(Some(AnsiColor::Green.into())),
            Self::Warning => Style::new().fg_color(Some(AnsiColor::Yellow.into())),
            Self::Failure => Style::new().fg_color(Some(AnsiColor::Red.into())),
            Self::Accent => Style::new().fg_color(Some(AnsiColor::Magenta.into())),
        }
    }

    /// The same role as a `console` style, for libraries that draw with
    /// `console` on stderr, such as the prompt theme.
    pub(crate) fn console(self) -> console::Style {
        let style = console::Style::new().for_stderr();
        match self {
            Self::Plain => style,
            Self::Value => style.cyan(),
            Self::Dim => style.dim(),
            Self::Strong => style.bold(),
            Self::Success => style.green(),
            Self::Warning => style.yellow(),
            Self::Failure => style.red(),
            Self::Accent => style.magenta(),
        }
    }

    pub(crate) fn paint(self, text: &str, color: bool) -> String {
        if !color || self == Self::Plain || text.is_empty() {
            return text.to_string();
        }
        let style = self.style();

        format!("{style}{text}{style:#}")
    }
}

/// A line of text made of toned spans, such as a message with a cyan value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Line {
    spans: Vec<Span>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Span {
    Text(Tone, String),
    ProseValue(String),
    /// A glyph shown only when decorated, such as a table status cell's `✗`.
    Mark(Mark),
}

impl Line {
    /// A label followed by an indivisible value. The label carries its own
    /// separator, as in `Line::field("path: ", path)`.
    pub(crate) fn field(label: &str, value: impl std::fmt::Display) -> Self {
        Self::from(label).value(value.to_string())
    }

    pub(crate) fn text(self, text: impl Into<String>) -> Self {
        self.toned(Tone::Plain, text)
    }

    /// A value such as a path or URL whose spaces must stay together.
    pub(crate) fn value(self, text: impl Into<String>) -> Self {
        self.toned(Tone::Value, text)
    }

    /// A value such as a summary sentence that may wrap at spaces.
    pub(crate) fn prose_value(mut self, text: impl Into<String>) -> Self {
        self.spans.push(Span::ProseValue(text.into()));
        self
    }

    /// Wraps diagnostic prose while keeping paths in their own spans.
    pub(crate) fn prose_with_paths(mut self, text: &str) -> Self {
        let mut remaining = text;
        while let Some(start) = remaining.match_indices('/').find_map(|(index, _)| {
            let prefix = &remaining[..index];
            match prefix.chars().next_back() {
                None => Some(index),
                Some(character) if character.is_whitespace() || "`\"".contains(character) => {
                    Some(index)
                }
                Some(':' | '/') => None,
                Some(_) => prefix.rfind(": ").map(|separator| separator + 2),
            }
        }) {
            let path = &remaining[start..];
            let end = if let Some(quote @ ('`' | '"')) = remaining[..start].chars().next_back() {
                path.find(quote).unwrap_or(path.len())
            } else {
                let next_failure = path.match_indices(", ").find_map(|(index, _)| {
                    let (label, _) = path[index + 2..].split_once(": ")?;
                    (!label.is_empty()
                        && label.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || b"._-".contains(&byte)
                        }))
                    .then_some(index)
                });
                let clause_end = [
                    next_failure,
                    path.find("; Gateway runtime"),
                    path.find('\n'),
                ]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(path.len());
                let clause = &path[..clause_end];
                if let Some(index) = clause.find(" and /") {
                    index
                } else if remaining[..start].ends_with("filesystem error at ")
                    && let Some(index) = clause.rfind(": ")
                {
                    index
                } else if remaining[..start].ends_with(" and ")
                    && let Some(prefix) = clause.strip_suffix(" exist")
                {
                    prefix.len()
                } else {
                    clause_end
                }
            };
            self = self.prose_value(&remaining[..start]).value(&path[..end]);
            remaining = &path[end..];
        }
        self.prose_value(remaining)
    }

    pub(crate) fn toned(mut self, tone: Tone, text: impl Into<String>) -> Self {
        self.spans.push(Span::Text(tone, text.into()));
        self
    }

    /// A status word in its mark's tone, led by the mark's glyph when
    /// decorated. Plain output omits the glyph, so plain words stay unchanged.
    pub(crate) fn marked(mark: Mark, text: impl Into<String>) -> Self {
        Self {
            spans: vec![Span::Mark(mark), Span::Text(mark.tone(), text.into())],
        }
    }

    pub(crate) fn plain(&self) -> String {
        self.spans
            .iter()
            .filter_map(|span| match span {
                Span::Text(_tone, text) => Some(text.as_str()),
                Span::ProseValue(text) => Some(text.as_str()),
                Span::Mark(_mark) => None,
            })
            .collect()
    }

    /// Paints every span, using `base` for spans without their own tone.
    fn paint(&self, base: Tone, color: bool) -> String {
        self.paint_with_value_space(base, color, None)
    }

    fn paint_with_value_space(&self, base: Tone, color: bool, space: Option<char>) -> String {
        self.spans
            .iter()
            .map(|span| match span {
                Span::Text(tone, text) => {
                    let is_value = *tone == Tone::Value;
                    let tone = if *tone == Tone::Plain { base } else { *tone };
                    if let Some(space) = space
                        && is_value
                    {
                        tone.paint(&text.replace(' ', &space.to_string()), color)
                    } else {
                        tone.paint(text, color)
                    }
                }
                Span::ProseValue(text) => Tone::Value.paint(text, color),
                Span::Mark(mark) => format!("{} ", mark.paint(color)),
            })
            .collect()
    }

    fn has_spaced_value(&self) -> bool {
        self.spans
            .iter()
            .any(|span| matches!(span, Span::Text(Tone::Value, text) if text.contains(' ')))
    }
}

impl From<&str> for Line {
    fn from(text: &str) -> Self {
        Self::default().text(text)
    }
}

impl From<String> for Line {
    fn from(text: String) -> Self {
        Self::default().text(text)
    }
}

impl From<&String> for Line {
    fn from(text: &String) -> Self {
        Self::default().text(text.as_str())
    }
}

/// The glyph that leads a status row. Every mark accompanies status text.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Mark {
    Success,
    Failure,
    Warning,
    /// A no-op, idle, or empty outcome.
    Idle,
    Running,
    /// An available update, distinct from a warning about a problem.
    UpdateAvailable,
    /// A completed flow step or answered prompt.
    Done,
    /// The flow step in progress.
    Active,
}

impl Mark {
    fn glyph(self) -> &'static str {
        match self {
            Self::Success => "✓",
            Self::Failure => "✗",
            Self::Warning => "⚠",
            Self::Idle => "○",
            Self::Running => "●",
            Self::UpdateAvailable => "↑",
            Self::Done => "◇",
            Self::Active => "◆",
        }
    }

    fn tone(self) -> Tone {
        match self {
            Self::Success | Self::Running | Self::Done => Tone::Success,
            Self::Failure => Tone::Failure,
            Self::Warning | Self::UpdateAvailable => Tone::Warning,
            Self::Idle => Tone::Dim,
            Self::Active => Tone::Accent,
        }
    }

    fn paint(self, color: bool) -> String {
        self.tone().paint(self.glyph(), color)
    }
}

/// A writer paired with how it is rendered.
///
/// Row methods write documented plain text on a plain surface and the
/// terminal-design treatment (glyph column, gutter, color, wrapping) on a
/// decorated one.
pub(crate) struct Output<'writer> {
    writer: &'writer mut dyn Write,
    surface: Surface,
    /// The open `┌ │ └` flow's title. Rows inside a flow carry the gutter;
    /// only a decorated surface opens one.
    flow: Option<String>,
    /// The prefix that aligns details and quotes under the latest row.
    continuation: String,
}

impl<'writer> Output<'writer> {
    pub(crate) fn new(writer: &'writer mut dyn Write, surface: Surface) -> Self {
        Self {
            writer,
            surface,
            flow: None,
            continuation: INDENT.to_string(),
        }
    }

    pub(crate) fn surface(&self) -> Surface {
        self.surface
    }

    /// Whether a flow is open, so a prompt shown now joins its gutter.
    pub(crate) fn in_flow(&self) -> bool {
        self.flow.is_some()
    }

    /// The underlying writer, for raw payloads that must not be rendered.
    pub(crate) fn writer(&mut self) -> &mut dyn Write {
        self.writer
    }

    /// Writes one compact JSON document and a newline, never decorated.
    pub(crate) fn json(&mut self, value: &impl Serialize) -> io::Result<()> {
        serde_json::to_writer(&mut *self.writer, value)?;
        writeln!(self.writer)
    }

    /// Writes one line as-is on both surfaces.
    pub(crate) fn line(&mut self, line: &str) -> io::Result<()> {
        writeln!(self.writer, "{line}")
    }

    pub(crate) fn success(&mut self, line: impl Into<Line>) -> io::Result<()> {
        self.status(Mark::Success, line)
    }

    pub(crate) fn failure(&mut self, line: impl Into<Line>) -> io::Result<()> {
        self.status(Mark::Failure, line)
    }

    /// A no-op outcome, such as something already being current.
    pub(crate) fn note(&mut self, line: impl Into<Line>) -> io::Result<()> {
        self.status(Mark::Idle, line)
    }

    /// A status row: the glyph column followed by the status text.
    pub(crate) fn status(&mut self, mark: Mark, line: impl Into<Line>) -> io::Result<()> {
        let line = line.into();
        if !self.surface.decorated {
            return self.line(&line.plain());
        }
        let glyph = mark.paint(self.surface.color);
        let body_tone = if mark == Mark::Idle {
            Tone::Dim
        } else {
            Tone::Plain
        };
        let (first, rest) = if self.flow.is_some() {
            let gutter = self.gutter_prefix();
            (format!("{gutter}  {glyph} "), format!("{gutter}    "))
        } else {
            (format!("{glyph}  "), INDENT.to_string())
        };
        self.write_wrapped_line(&first, &rest, "", &line, body_tone)?;
        self.continuation = rest;

        Ok(())
    }

    /// A secondary line under the latest row.
    pub(crate) fn detail(&mut self, line: impl Into<Line>) -> io::Result<()> {
        let line = line.into();
        if !self.surface.decorated {
            return self.line(&format!("  {}", line.plain()));
        }
        if let [Span::Text(Tone::Value, value)] = line.spans.as_slice() {
            return writeln!(
                self.writer,
                "{}{}",
                self.continuation,
                Tone::Value.paint(value, self.surface.color)
            );
        }
        let prefix = self.continuation.clone();
        self.write_wrapped_line(&prefix, &prefix, "", &line, Tone::Dim)
    }

    /// A consequence of the latest row, such as the reconciliation job it
    /// queued. Plain output keeps the line unindented.
    pub(crate) fn follow_up(&mut self, line: impl Into<Line>) -> io::Result<()> {
        let line = line.into();
        if !self.surface.decorated {
            return self.line(&line.plain());
        }
        self.arrow_row(&line)
    }

    /// A runnable next step, such as a repair command.
    pub(crate) fn hint(&mut self, label: &str, command: &str) -> io::Result<()> {
        if !self.surface.decorated {
            return self.line(&format!("  {label}: {command}"));
        }
        self.arrow_row(&Line::from(label).text("  ").value(command))
    }

    /// A dim `↳ …` row under the latest row, keeping value spans colored.
    fn arrow_row(&mut self, line: &Line) -> io::Result<()> {
        let color = self.surface.color;
        let leading = format!("{} ", Tone::Dim.paint("↳", color));
        let prefix = self.continuation.clone();
        self.write_wrapped_line(&prefix, &format!("{prefix}  "), &leading, line, Tone::Dim)
    }

    /// Verbatim content such as a config preview: unchanged on a plain
    /// surface, aligned under the latest row on a decorated one, and never
    /// wrapped or recolored.
    pub(crate) fn quote(&mut self, text: &str) -> io::Result<()> {
        if !self.surface.decorated {
            return self.line(text);
        }
        writeln!(self.writer, "{}{text}", self.continuation)
    }

    /// A warning, normally written to stderr.
    pub(crate) fn warning(&mut self, message: &str) -> io::Result<()> {
        if !self.surface.decorated {
            return writeln!(self.writer, "warning: {message}");
        }
        self.labelled(Mark::Warning, "warning:", Tone::Plain, message)
    }

    /// An error, normally written to stderr. Lines after the first are cause
    /// and repair details.
    pub(crate) fn error(&mut self, message: &str) -> io::Result<()> {
        let mut lines = message.lines();
        if !self.surface.decorated {
            writeln!(self.writer, "error: {}", lines.next().unwrap_or_default())?;
            for cause in lines {
                writeln!(self.writer, "  {cause}")?;
            }
            return Ok(());
        }
        self.labelled(
            Mark::Failure,
            "error:",
            Tone::Strong,
            lines.next().unwrap_or_default(),
        )?;
        let color = self.surface.color;
        for cause in lines {
            self.write_wrapped(INDENT, INDENT, &Tone::Dim.paint(cause, color))?;
        }

        Ok(())
    }

    /// A decorated `glyph  label summary` row, colored by the mark.
    fn labelled(&mut self, mark: Mark, label: &str, tone: Tone, summary: &str) -> io::Result<()> {
        let color = self.surface.color;
        let body = format!(
            "{} {}",
            mark.tone().paint(label, color),
            tone.paint(summary, color)
        );
        self.write_wrapped(&format!("{}  ", mark.paint(color)), INDENT, &body)
    }

    /// A report heading: the `pv` badge and the command, then a rule. A plain
    /// surface gets `plain_title` when the command documents one.
    pub(crate) fn heading(&mut self, command: &str, plain_title: Option<&str>) -> io::Result<()> {
        if !self.surface.decorated {
            return match plain_title {
                Some(title) => self.line(title),
                None => Ok(()),
            };
        }
        self.badge(command)?;
        let rule = "─".repeat(self.surface.width);
        writeln!(
            self.writer,
            "{}",
            Tone::Dim.paint(&rule, self.surface.color)
        )
    }

    /// A full report heading includes the version shown in the terminal designs.
    pub(crate) fn report_heading(
        &mut self,
        command: &str,
        plain_title: Option<&str>,
    ) -> io::Result<()> {
        if !self.surface.decorated {
            return self.heading(command, plain_title);
        }
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let label_width = 5 + command.chars().count();
        if label_width + version.len() + 1 > self.surface.width {
            return self.heading(command, plain_title);
        }
        let padding = " ".repeat(self.surface.width - label_width - version.len());
        let color = self.surface.color;
        let badge = if color {
            let style = Tone::Accent
                .style()
                .effects(Effects::INVERT | Effects::BOLD);
            format!("{style} pv {style:#}")
        } else {
            "[pv]".to_string()
        };
        writeln!(
            self.writer,
            "{badge} {}{padding}{}",
            Tone::Strong.paint(command, color),
            Tone::Dim.paint(&version, color),
        )?;
        writeln!(
            self.writer,
            "{}",
            Tone::Dim.paint(&"─".repeat(self.surface.width), color)
        )
    }

    /// A report section heading, such as `ROUTING`, after a blank line.
    /// Plain reports stay flat, so a plain surface writes nothing; a report
    /// whose plain form has its own section labels uses its own plain writer.
    pub(crate) fn section(&mut self, title: &str) -> io::Result<()> {
        if !self.surface.decorated {
            return Ok(());
        }
        let title = title.to_uppercase();
        let title = if self.surface.color {
            let style = Style::new().effects(Effects::DIMMED | Effects::BOLD);
            format!("{style}{title}{style:#}")
        } else {
            title
        };
        self.continuation = INDENT.to_string();
        writeln!(self.writer)?;
        writeln!(self.writer, "{title}")
    }

    /// Opens a multi-step flow: the badge, then `┌  title  ·  subtitle`.
    /// A plain surface gets only the title.
    pub(crate) fn flow_start(
        &mut self,
        command: &str,
        title: &str,
        subtitle: Option<&str>,
    ) -> io::Result<()> {
        if !self.surface.decorated {
            return self.line(title);
        }
        let color = self.surface.color;
        self.badge(command)?;
        writeln!(self.writer)?;
        let subtitle = subtitle
            .map(|subtitle| Tone::Dim.paint(&format!("  ·  {subtitle}"), color))
            .unwrap_or_default();
        writeln!(
            self.writer,
            "{}  {}{subtitle}",
            Tone::Dim.paint("┌", color),
            Tone::Strong.paint(title, color)
        )?;
        self.flow_resume(title);

        Ok(())
    }

    /// Continues a flow that an earlier process opened, such as the
    /// continuation `pv update` re-execs after activating a new release:
    /// later rows carry the gutter without a second opener.
    pub(crate) fn flow_resume(&mut self, title: &str) {
        if self.surface.decorated {
            self.flow = Some(title.to_string());
            self.continuation = format!("{}  ", self.gutter_prefix());
        }
    }

    /// Closes a flow that a failing command left open, before its error.
    /// Plain output has no flow to close.
    pub(crate) fn flow_stopped(&mut self) -> io::Result<()> {
        let Some(title) = &self.flow else {
            return Ok(());
        };
        let stopped = format!("{title} stopped");
        self.flow_end(Mark::Failure, stopped)
    }

    /// A flow step. Rows written after it render inside the gutter.
    pub(crate) fn flow_step(&mut self, mark: Mark, line: impl Into<Line>) -> io::Result<()> {
        let line = line.into();
        if !self.surface.decorated {
            return self.line(&line.plain());
        }
        let color = self.surface.color;
        writeln!(self.writer, "{}", self.gutter_prefix())?;
        let rest = format!("{}  ", self.gutter_prefix());
        self.write_wrapped_line(
            &format!("{}  ", mark.paint(color)),
            &rest,
            "",
            &line,
            Tone::Strong,
        )?;
        self.continuation = rest;

        Ok(())
    }

    /// Closes a flow with its outcome.
    pub(crate) fn flow_end(&mut self, mark: Mark, line: impl Into<Line>) -> io::Result<()> {
        let line = line.into();
        self.flow = None;
        self.continuation = INDENT.to_string();
        if !self.surface.decorated {
            return self.line(&line.plain());
        }
        let color = self.surface.color;
        writeln!(self.writer, "{}", Tone::Dim.paint(GUTTER, color))?;
        let outcome = if mark == Mark::Done {
            String::new()
        } else {
            format!("{} ", mark.paint(color))
        };
        self.write_wrapped_line(
            &format!("{}  ", Tone::Dim.paint("└", color)),
            INDENT,
            &outcome,
            &line,
            Tone::Strong,
        )
    }

    fn badge(&mut self, command: &str) -> io::Result<()> {
        let color = self.surface.color;
        let badge = if color {
            let style = Tone::Accent
                .style()
                .effects(Effects::INVERT | Effects::BOLD);
            format!("{style} pv {style:#}")
        } else {
            "[pv]".to_string()
        };
        writeln!(
            self.writer,
            "{badge} {}",
            Tone::Strong.paint(command, color)
        )
    }

    fn gutter_prefix(&self) -> String {
        Tone::Dim.paint(GUTTER, self.surface.color)
    }

    /// Writes `body` wrapped at word boundaries to the surface width, with
    /// `first` before the first line and `rest` before later ones. Words,
    /// including paths, URLs, and identifiers, are never split; a word longer
    /// than the line overflows instead.
    fn write_wrapped(&mut self, first: &str, rest: &str, body: &str) -> io::Result<()> {
        self.write_wrapped_with_space(first, rest, body, None)
    }

    fn write_wrapped_line(
        &mut self,
        first: &str,
        rest: &str,
        leading: &str,
        line: &Line,
        base: Tone,
    ) -> io::Result<()> {
        if !line.has_spaced_value() {
            return self.write_wrapped(
                first,
                rest,
                &format!("{leading}{}", line.paint(base, self.surface.color)),
            );
        }
        let plain = line.plain();
        let Some(space) = ('\u{e000}'..='\u{f8ff}').find(|space| !plain.contains(*space)) else {
            return writeln!(
                self.writer,
                "{first}{leading}{}",
                line.paint(base, self.surface.color)
            );
        };
        let body = format!(
            "{leading}{}",
            line.paint_with_value_space(base, self.surface.color, Some(space))
        );
        self.write_wrapped_with_space(first, rest, &body, Some(space))
    }

    fn write_wrapped_with_space(
        &mut self,
        first: &str,
        rest: &str,
        body: &str,
        space: Option<char>,
    ) -> io::Result<()> {
        if body.is_empty() {
            return writeln!(self.writer, "{}", first.trim_end());
        }
        let width = self
            .surface
            .width
            .max(textwrap::core::display_width(rest) + MIN_TEXT_WIDTH);
        let options = Options::new(width)
            .initial_indent(first)
            .subsequent_indent(rest)
            .break_words(false)
            .word_separator(WordSeparator::AsciiSpace)
            .word_splitter(WordSplitter::NoHyphenation);

        let wrapped = textwrap::fill(body, options);
        if let Some(space) = space {
            writeln!(self.writer, "{}", wrapped.replace(space, " "))
        } else {
            writeln!(self.writer, "{wrapped}")
        }
    }
}

/// The command's stdout and stderr, and whether it may prompt.
pub(crate) struct Streams<'writer> {
    pub(crate) out: Output<'writer>,
    pub(crate) err: Output<'writer>,
    pub(crate) interactive: bool,
}

impl<'writer> Streams<'writer> {
    pub(crate) fn new(
        stdout: &'writer mut dyn Write,
        stderr: &'writer mut dyn Write,
        presentation: Presentation,
    ) -> Self {
        Self {
            out: Output::new(stdout, presentation.stdout),
            err: Output::new(stderr, presentation.stderr),
            interactive: presentation.interactive,
        }
    }

    /// Runs `step` with its stdout and stderr captured into `out` and `err`,
    /// so the caller can title the rows once the outcome is known. Captured
    /// stdout keeps its open flow. A captured step cannot prompt: its prompt
    /// would show above rows that are still held back, so prompts refuse.
    pub(crate) fn capture<T>(
        &mut self,
        out: &mut Vec<u8>,
        err: &mut Vec<u8>,
        step: impl FnOnce(&mut Streams<'_>) -> T,
    ) -> T {
        let mut captured = Streams {
            out: Output::new(out, self.out.surface),
            err: Output::new(err, self.err.surface),
            interactive: false,
        };
        if let Some(title) = &self.out.flow {
            captured.out.flow_resume(title);
        }

        step(&mut captured)
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    /// Renders escape sequences visibly so color snapshots stay readable.
    pub(crate) fn visible(bytes: Vec<u8>) -> String {
        String::from_utf8_lossy(&bytes).replace('\u{1b}', "␛")
    }

    fn render(surface: Surface, write: impl FnOnce(&mut Output<'_>) -> io::Result<()>) -> String {
        let mut bytes = Vec::new();
        let mut output = Output::new(&mut bytes, surface);
        if let Err(error) = write(&mut output) {
            return format!("write failed: {error}");
        }

        visible(bytes)
    }

    fn specimen(output: &mut Output<'_>) -> io::Result<()> {
        output.status(
            Mark::Done,
            Line::from("Wrote Project config: ")
                .value("/Users/me/Code/acme-store/with/a/long/path"),
        )?;
        output.detail("Laravel: Detected composer.json, artisan, and Laravel project files")?;
        output.status(Mark::Warning, "Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.")?;
        output.status(
            Mark::Idle,
            "No framework-specific Project signals detected.",
        )?;
        output.quote("  APP_URL: ${project_url}")
    }

    fn flow(output: &mut Output<'_>) -> io::Result<()> {
        output.flow_start("init", "pv init", Some("acme"))?;
        output.flow_step(Mark::Done, Line::from("Detected Project signals:"))?;
        output.detail("Laravel: Detected composer.json, artisan, and Laravel project files")?;
        output.flow_step(Mark::Done, "Project config preview:")?;
        output.quote("php: '8.4'")?;
        output.status(
            Mark::Failure,
            "Loopback TCP port 80 already has a listener.",
        )?;
        output.detail("Stop the conflicting service, then run `pv ports:install` again.")?;
        output.flow_end(Mark::Failure, "pv init cancelled; no files changed.")
    }

    #[test]
    fn plain_rows_keep_documented_text() {
        assert_snapshot!(render(Surface::plain(), specimen), @"
        Wrote Project config: /Users/me/Code/acme-store/with/a/long/path
          Laravel: Detected composer.json, artisan, and Laravel project files
        Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.
        No framework-specific Project signals detected.
          APP_URL: ${project_url}
        ");
    }

    #[test]
    fn decorated_rows_without_color_keep_glyphs_and_words() {
        assert_snapshot!(render(Surface::terminal(false, 80), specimen), @"
        ◇  Wrote Project config: /Users/me/Code/acme-store/with/a/long/path
           Laravel: Detected composer.json, artisan, and Laravel project files
        ⚠  Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and
           VITE_DEV_SERVER_KEY.
        ○  No framework-specific Project signals detected.
             APP_URL: ${project_url}
        ");
    }

    #[test]
    fn decorated_rows_wrap_at_sixty_columns_without_splitting_values() {
        assert_snapshot!(render(Surface::terminal(false, 60), specimen), @"
        ◇  Wrote Project config:
           /Users/me/Code/acme-store/with/a/long/path
           Laravel: Detected composer.json, artisan, and Laravel
           project files
        ⚠  Vite HTTPS: configure the app's Vite config to read
           VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.
        ○  No framework-specific Project signals detected.
             APP_URL: ${project_url}
        ");
    }

    #[test]
    fn decorated_rows_keep_whitespace_bearing_values_together() {
        let write = |output: &mut Output<'_>| {
            let path = "/Users/me/My Project/pv.yml";
            output.status(Mark::Done, Line::field("Wrote Project config: ", path))?;
            output.flow_start("init", "pv init", None)?;
            output.flow_step(Mark::Done, Line::field("Detected config: ", path))?;
            output.detail(Line::field(
                "path: ",
                "/Users/me/My\u{a0}Project/\u{2007}Source Files",
            ))?;
            output.detail(Line::from("summary: ").prose_value(
                "Project env reconciled for 1 of 2 Projects; failures: gateway unavailable",
            ))?;
            output.flow_end(Mark::Done, Line::field("Wrote Project config: ", path))
        };

        assert_snapshot!(render(Surface::terminal(false, 40), write), @"
        ◇  Wrote Project config:
           /Users/me/My Project/pv.yml
        [pv] init

        ┌  pv init
        │
        ◇  Detected config:
        │  /Users/me/My Project/pv.yml
        │  path:
        │  /Users/me/My Project/ Source Files
        │  summary: Project env reconciled for
        │  1 of 2 Projects; failures: gateway
        │  unavailable
        │
        └  Wrote Project config:
           /Users/me/My Project/pv.yml
        ");
        assert_snapshot!(render(Surface::terminal(true, 40), write), @"
        ␛[32m◇␛[0m  Wrote Project config:
           ␛[36m/Users/me/My Project/pv.yml␛[0m
        ␛[1m␛[7m␛[35m pv ␛[0m ␛[1minit␛[0m

        ␛[2m┌␛[0m  ␛[1mpv init␛[0m
        ␛[2m│␛[0m
        ␛[32m◇␛[0m  ␛[1mDetected config:
        ␛[2m│␛[0m  ␛[0m␛[36m/Users/me/My Project/pv.yml␛[0m
        ␛[2m│␛[0m  ␛[2mpath:
        ␛[2m│␛[0m  ␛[0m␛[36m/Users/me/My Project/ Source Files␛[0m
        ␛[2m│␛[0m  ␛[2msummary: ␛[0m␛[36mProject env reconciled for
        ␛[2m│␛[0m  1 of 2 Projects; failures: gateway
        ␛[2m│␛[0m  unavailable␛[0m
        ␛[2m│␛[0m
        ␛[2m└␛[0m  ␛[1mWrote Project config:
           ␛[0m␛[36m/Users/me/My Project/pv.yml␛[0m
        ");
    }

    #[test]
    fn decorated_rows_use_wide_terminals() {
        assert_snapshot!(render(Surface::terminal(false, 120), specimen), @"
        ◇  Wrote Project config: /Users/me/Code/acme-store/with/a/long/path
           Laravel: Detected composer.json, artisan, and Laravel project files
        ⚠  Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.
        ○  No framework-specific Project signals detected.
             APP_URL: ${project_url}
        ");
    }

    #[test]
    fn decorated_rows_color_semantic_roles() {
        assert_snapshot!(render(Surface::terminal(true, 80), specimen), @"
        ␛[32m◇␛[0m  Wrote Project config: ␛[36m/Users/me/Code/acme-store/with/a/long/path␛[0m
           ␛[2mLaravel: Detected composer.json, artisan, and Laravel project files␛[0m
        ␛[33m⚠␛[0m  Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and
           VITE_DEV_SERVER_KEY.
        ␛[2m○␛[0m  ␛[2mNo framework-specific Project signals detected.␛[0m
             APP_URL: ${project_url}
        ");
    }

    #[test]
    fn flows_render_gutter_steps_and_failed_ending() {
        assert_snapshot!(render(Surface::terminal(false, 80), flow), @"
        [pv] init

        ┌  pv init  ·  acme
        │
        ◇  Detected Project signals:
        │  Laravel: Detected composer.json, artisan, and Laravel project files
        │
        ◇  Project config preview:
        │  php: '8.4'
        │  ✗ Loopback TCP port 80 already has a listener.
        │    Stop the conflicting service, then run `pv ports:install` again.
        │
        └  ✗ pv init cancelled; no files changed.
        ");
        assert_snapshot!(render(Surface::plain(), flow), @"
        pv init
        Detected Project signals:
          Laravel: Detected composer.json, artisan, and Laravel project files
        Project config preview:
        php: '8.4'
        Loopback TCP port 80 already has a listener.
          Stop the conflicting service, then run `pv ports:install` again.
        pv init cancelled; no files changed.
        ");
    }

    #[test]
    fn errors_and_warnings_keep_their_label_and_dim_details() {
        let write = |output: &mut Output<'_>| {
            output.warning(
                "PV daemon is not running; reconciliation will run after `pv setup` starts it",
            )?;
            output.error("PHP track 8.3 is not installed.\nRun `pv php:install 8.3` to install it.")
        };

        assert_snapshot!(render(Surface::plain(), write), @"
        warning: PV daemon is not running; reconciliation will run after `pv setup` starts it
        error: PHP track 8.3 is not installed.
          Run `pv php:install 8.3` to install it.
        ");
        assert_snapshot!(render(Surface::terminal(false, 60), write), @"
        ⚠  warning: PV daemon is not running; reconciliation will
           run after `pv setup` starts it
        ✗  error: PHP track 8.3 is not installed.
           Run `pv php:install 8.3` to install it.
        ");
        assert_snapshot!(render(Surface::terminal(true, 60), write), @"
        ␛[33m⚠␛[0m  ␛[33mwarning:␛[0m PV daemon is not running; reconciliation will
           run after `pv setup` starts it
        ␛[31m✗␛[0m  ␛[31merror:␛[0m ␛[1mPHP track 8.3 is not installed.␛[0m
           ␛[2mRun `pv php:install 8.3` to install it.␛[0m
        ");
    }
}
