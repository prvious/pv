use std::io::Write;
use std::process::ExitCode;

use crate::args::EnvArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};
use crate::shell::Shell;

pub(crate) fn run(
    args: EnvArgs,
    no_color: bool,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let shell = match args.shell {
        Some(shell) => shell,
        None => detect_shell(environment)?,
    };
    let mut output = Output::new(stdout, OutputMode::from_no_color(no_color));
    output.line(shell.env_script())?;

    Ok(ExitCode::SUCCESS)
}

fn detect_shell(environment: &impl Environment) -> Result<Shell, CliError> {
    let shell_path = environment.var_os("SHELL").ok_or(CliError::MissingShell)?;
    Shell::detect(&shell_path).ok_or_else(|| CliError::UnsupportedDetectedShell {
        shell: shell_path.to_string_lossy().into_owned(),
    })
}
