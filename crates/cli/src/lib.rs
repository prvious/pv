use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "pv",
    version,
    about = "Laravel-first local desired-state control plane",
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(long, global = true, help = "Disable colored output")]
    no_color: bool,
}

#[derive(Debug, Error)]
pub enum CliError {}

pub trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;
}

#[derive(Debug, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        process_var_os(key)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV environment helper owns direct process environment reads"
)]
fn process_var_os(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

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

    fn from_inputs(args: &[OsString], environment: &impl Environment) -> Self {
        Self {
            no_color: args_have_no_color(args) || environment.var_os("NO_COLOR").is_some(),
        }
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

pub fn run<I, Argument>(
    args: I,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode>
where
    I: IntoIterator<Item = Argument>,
    Argument: Into<OsString>,
{
    run_with_environment(args, &ProcessEnvironment, stdout, stderr)
}

pub fn run_with_environment<I, Argument>(
    args: I,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode>
where
    I: IntoIterator<Item = Argument>,
    Argument: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let output_mode = OutputMode::from_inputs(&args, environment);
    let mut clap_command = Cli::command().color(clap_color(output_mode));
    let matches = match clap_command.try_get_matches_from_mut(&args) {
        Ok(matches) => matches,
        Err(error) => {
            let status_code = error.exit_code();
            write_clap_error(error, stdout, stderr)?;
            return Ok(exit_code(status_code));
        }
    };
    let _cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => {
            let status_code = error.exit_code();
            write_clap_error(error, stdout, stderr)?;
            return Ok(exit_code(status_code));
        }
    };

    Ok(ExitCode::SUCCESS)
}

fn clap_color(output_mode: OutputMode) -> clap::ColorChoice {
    if output_mode.no_color {
        clap::ColorChoice::Never
    } else {
        clap::ColorChoice::Auto
    }
}

fn args_have_no_color(args: &[OsString]) -> bool {
    args.iter().any(|argument| argument == "--no-color")
}

fn exit_code(code: i32) -> ExitCode {
    if code == 0 {
        return ExitCode::SUCCESS;
    }

    match u8::try_from(code) {
        Ok(code) => ExitCode::from(code),
        Err(_) => ExitCode::FAILURE,
    }
}

fn write_clap_error(
    error: clap::Error,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> std::io::Result<()> {
    if matches!(
        error.kind(),
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
    ) {
        write!(stdout, "{error}")
    } else {
        write!(stderr, "{error}")
    }
}
