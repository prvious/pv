use std::io::Write;
use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::generate;

use crate::args::{Cli, CompletionsArgs};

pub(crate) fn run(args: CompletionsArgs, stdout: &mut impl Write) -> ExitCode {
    let mut command = Cli::command();
    generate(args.shell.completion_shell(), &mut command, "pv", stdout);

    ExitCode::SUCCESS
}
