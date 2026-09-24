use std::ffi::OsString;
use std::io::Write;

use serde::Serialize;

use crate::environment::Environment;

const DEFAULT_WIDTH: usize = 80;

/// How one output stream is rendered.
///
/// A stream is *decorated* when it is a terminal: it may use glyphs, gutters,
/// headings, and tables. It uses color only when it is decorated and color is
/// not disabled by `NO_COLOR` or `--no-color`. A plain stream receives the
/// documented plain text only.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Surface {
    decorated: bool,
    color: bool,
    width: usize,
}

impl Surface {
    pub fn plain() -> Self {
        Self {
            decorated: false,
            color: false,
            width: DEFAULT_WIDTH,
        }
    }

    pub(crate) fn decorated(self) -> bool {
        self.decorated
    }

    pub(crate) fn color(self) -> bool {
        self.color
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

    #[cfg(test)]
    pub(crate) fn interactive() -> Self {
        Self {
            interactive: true,
            ..Self::plain()
        }
    }
}

/// A writer paired with how it is rendered.
pub struct Output<'writer> {
    writer: &'writer mut dyn Write,
    surface: Surface,
}

impl<'writer> Output<'writer> {
    pub fn new(writer: &'writer mut dyn Write, surface: Surface) -> Self {
        Self { writer, surface }
    }

    pub(crate) fn surface(&self) -> Surface {
        self.surface
    }

    /// The underlying writer, for raw payloads that must not be rendered.
    pub(crate) fn writer(&mut self) -> &mut dyn Write {
        self.writer
    }

    /// Writes one compact JSON document and a newline, never decorated.
    pub(crate) fn json(&mut self, value: &impl Serialize) -> std::io::Result<()> {
        serde_json::to_writer(&mut *self.writer, value)?;
        writeln!(self.writer)
    }

    pub fn line(&mut self, line: &str) -> std::io::Result<()> {
        writeln!(self.writer, "{line}")
    }

    pub fn error(&mut self, message: &str) -> std::io::Result<()> {
        writeln!(self.writer, "error: {message}")
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
}
