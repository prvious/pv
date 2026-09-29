use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::generate;

use crate::args::{Cli, CompletionsArgs};
use crate::output::Streams;

pub(crate) fn run(args: CompletionsArgs, streams: &mut Streams<'_>) -> ExitCode {
    let mut command = Cli::command();
    generate(
        args.shell.completion_shell(),
        &mut command,
        "pv",
        streams.out.writer(),
    );

    ExitCode::SUCCESS
}
