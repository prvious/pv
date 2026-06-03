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

    #[command(name = "dns:status", about = "Show PV .test resolver status")]
    DnsStatus,

    #[command(name = "dns:install", about = "Prepare PV .test resolver config")]
    DnsInstall,

    #[command(name = "dns:uninstall", about = "Remove PV .test resolver config")]
    DnsUninstall,

    #[command(name = "link", about = "Link a Project")]
    Link(LinkArgs),

    #[command(name = "unlink", about = "Unlink a Project")]
    Unlink(UnlinkArgs),

    #[command(name = "open", about = "Open a linked Project")]
    Open(OpenArgs),

    #[command(
        name = "project:env",
        about = "Print generated Project environment values"
    )]
    ProjectEnv(ProjectEnvArgs),

    #[command(name = "list", about = "List linked Projects")]
    List,

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

#[derive(Debug, clap::Args)]
pub(crate) struct LinkArgs {
    #[arg(value_name = "path", help = "Project path to link")]
    pub(crate) path: Option<String>,

    #[arg(long, value_name = "hostname", help = "Primary .test hostname")]
    pub(crate) hostname: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct UnlinkArgs {
    #[arg(value_name = "hostname", help = "Project hostname to unlink")]
    pub(crate) hostname: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct OpenArgs {
    #[arg(value_name = "hostname", help = "Project hostname to open")]
    pub(crate) hostname: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ProjectEnvArgs {
    #[arg(long, help = "Print generated Project environment values as JSON")]
    pub(crate) json: bool,

    #[arg(value_name = "hostname", help = "Project hostname to render env for")]
    pub(crate) hostname: Option<String>,
}
