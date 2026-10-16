use std::process::ExitCode;

use clap_complete::generate;

use crate::args::{Cli, CompletionsArgs};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::Streams;

pub(crate) fn run(
    args: CompletionsArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let mut command = Cli::command_for(environment.platform_target()?);
    generate(
        args.shell.completion_shell(),
        &mut command,
        "pv",
        streams.out.writer(),
    );

    Ok(ExitCode::SUCCESS)
}
