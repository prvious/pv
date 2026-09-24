mod args;
mod commands;
mod environment;
mod error;
mod helper_release;
mod output;
mod progress;
mod prompt;
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
pub use output::{Output, Surface};
use output::{Presentation, Streams};
pub use prompt::{Answer, Choice, Prompt, PromptKind, Validator};

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
    let presentation = Presentation::detect(&args, environment);
    let mut clap_command = Cli::command().color(clap_color(presentation));
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

    environment.set_terminal_colors(presentation.stderr.color());
    let mut streams = Streams::new(stdout, stderr, presentation);
    let result = commands::execute(cli, environment, &mut streams);

    finish_execution(result, &mut streams.err)
}

fn finish_execution(
    result: Result<ExitCode, ExecuteError>,
    stderr: &mut Output<'_>,
) -> Result<ExitCode> {
    let message = match result {
        Ok(exit_code) => return Ok(exit_code),
        // The prompt has already drawn its cancelled state.
        Err(ExecuteError::User(CliError::PromptCancelled)) => return Ok(ExitCode::from(130)),
        Err(ExecuteError::Daemon(error)) => return Err(error.into()),
        Err(ExecuteError::State(error)) => return Err(error.into()),
        Err(error) => error.to_string(),
    };
    stderr.error(&message)?;

    Ok(ExitCode::FAILURE)
}

fn clap_color(presentation: Presentation) -> clap::ColorChoice {
    if !presentation.stderr.color() {
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

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::ExitCode;

    use super::finish_execution;
    use crate::error::ExecuteError;
    use crate::output::{Output, Surface};

    #[test]
    fn finish_execution_formats_io_errors() -> anyhow::Result<()> {
        let mut stderr = Vec::new();
        let exit_code = finish_execution(
            Err(ExecuteError::Io(io::Error::other("stdout closed"))),
            &mut Output::new(&mut stderr, Surface::plain()),
        )?;

        assert_eq!(exit_code, ExitCode::FAILURE);
        assert_eq!(String::from_utf8(stderr)?, "error: stdout closed\n");

        Ok(())
    }
}
