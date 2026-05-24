use std::ffi::OsString;
use std::io::Write;

use crate::environment::Environment;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OutputMode {
    no_color: bool,
}

impl OutputMode {
    pub fn plain() -> Self {
        Self { no_color: true }
    }

    pub fn no_color(self) -> bool {
        self.no_color
    }

    pub(crate) fn from_inputs(args: &[OsString], environment: &impl Environment) -> Self {
        Self {
            no_color: args_have_no_color(args) || environment.var_os("NO_COLOR").is_some(),
        }
    }

    pub(crate) fn from_no_color(no_color: bool) -> Self {
        Self { no_color }
    }

    fn error_label(self) -> &'static str {
        "error"
    }
}

pub struct Output<'writer, Writer> {
    writer: &'writer mut Writer,
    mode: OutputMode,
}

impl<'writer, Writer> Output<'writer, Writer>
where
    Writer: Write,
{
    pub fn new(writer: &'writer mut Writer, mode: OutputMode) -> Self {
        Self { writer, mode }
    }

    pub fn line(&mut self, line: &str) -> std::io::Result<()> {
        writeln!(self.writer, "{line}")
    }

    pub fn error(&mut self, message: &str) -> std::io::Result<()> {
        writeln!(self.writer, "{}: {message}", self.mode.error_label())
    }
}

fn args_have_no_color(args: &[OsString]) -> bool {
    args.iter().any(|argument| argument == "--no-color")
}
