use clap::{Parser, Subcommand};

use crate::shell::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "pv",
    version,
    about = "Laravel-first local desired-state control plane",
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    #[arg(long, global = true, help = "Disable colored output")]
    pub(crate) no_color: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    #[command(name = "env", about = "Print shell integration code")]
    Env(EnvArgs),

    #[command(name = "completions", about = "Generate shell completions")]
    Completions(CompletionsArgs),

    #[command(name = "daemon:enable", about = "Enable the PV login daemon")]
    DaemonEnable,

    #[command(name = "daemon:disable", about = "Disable the PV login daemon")]
    DaemonDisable,

    #[command(name = "daemon:restart", about = "Restart the PV login daemon")]
    DaemonRestart,

    #[command(name = "daemon:run", about = "Run the internal PV daemon", hide = true)]
    DaemonRun,

    #[command(name = "php:install", about = "Install a PHP track")]
    PhpInstall(PhpInstallArgs),
}

#[derive(Debug, clap::Args)]
pub(crate) struct EnvArgs {
    #[arg(long, value_enum, help = "Shell syntax to generate")]
    pub(crate) shell: Option<Shell>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct CompletionsArgs {
    #[arg(value_enum, help = "Shell to generate completions for")]
    pub(crate) shell: Shell,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PhpInstallArgs {
    #[arg(value_name = "version", help = "PHP track to install")]
    pub(crate) track: Option<String>,
}
