mod args;
mod commands;
mod environment;
mod error;
mod output;
mod shell;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use anyhow::Result;
use args::Cli;
use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches};
pub use environment::{Environment, ProcessEnvironment};
pub use error::CliError;
use error::ExecuteError;
pub use output::{Output, OutputMode};

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
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => {
            let status_code = error.exit_code();
            write_clap_error(error, stdout, stderr)?;
            return Ok(exit_code(status_code));
        }
    };

    finish_execution(
        commands::execute(cli, environment, stdout),
        output_mode,
        stderr,
    )
}

fn finish_execution(
    result: Result<ExitCode, ExecuteError>,
    output_mode: OutputMode,
    stderr: &mut impl Write,
) -> Result<ExitCode> {
    match result {
        Ok(exit_code) => Ok(exit_code),
        Err(ExecuteError::User(error)) => {
            let mut output = Output::new(stderr, output_mode);
            output.error(&error.to_string())?;

            Ok(ExitCode::FAILURE)
        }
        Err(ExecuteError::Io(error)) => Err(error.into()),
        Err(ExecuteError::Daemon(error)) => Err(error.into()),
        Err(ExecuteError::State(error)) => Err(error.into()),
    }
}

fn clap_color(output_mode: OutputMode) -> clap::ColorChoice {
    if output_mode.no_color() {
        clap::ColorChoice::Never
    } else {
        clap::ColorChoice::Auto
    }
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
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        write!(stdout, "{error}")
    } else {
        write!(stderr, "{error}")
    }
}
