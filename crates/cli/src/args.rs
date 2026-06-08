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

    #[command(name = "setup", about = "Configure PV system integrations")]
    Setup(SetupArgs),

    #[command(name = "uninstall", about = "Uninstall PV safely")]
    Uninstall(UninstallArgs),

    #[command(name = "daemon:enable", about = "Enable the PV login daemon")]
    DaemonEnable,

    #[command(name = "daemon:disable", about = "Disable the PV login daemon")]
    DaemonDisable,

    #[command(name = "daemon:restart", about = "Restart the PV login daemon")]
    DaemonRestart,

    #[command(name = "daemon:run", about = "Run the internal PV daemon", hide = true)]
    DaemonRun,

    #[command(
        name = "shim:php",
        about = "Run the internal PV PHP shim",
        hide = true,
        disable_help_flag = true,
        disable_version_flag = true,
        trailing_var_arg = true
    )]
    ShimPhp(ShimArgs),

    #[command(
        name = "shim:composer",
        about = "Run the internal PV Composer shim",
        hide = true,
        disable_help_flag = true,
        disable_version_flag = true,
        trailing_var_arg = true
    )]
    ShimComposer(ShimArgs),

    #[command(name = "dns:status", about = "Show PV .test resolver status")]
    DnsStatus,

    #[command(
        name = "dns:install",
        about = "Install or repair PV .test resolver config"
    )]
    DnsInstall,

    #[command(name = "dns:uninstall", about = "Remove PV .test resolver config")]
    DnsUninstall,

    #[command(name = "ports:status", about = "Show PV pf redirect status")]
    PortsStatus,

    #[command(name = "ports:install", about = "Install or repair PV pf redirects")]
    PortsInstall,

    #[command(name = "ports:uninstall", about = "Remove PV pf redirects")]
    PortsUninstall,

    #[command(name = "ca:status", about = "Show PV local CA trust status")]
    CaStatus,

    #[command(name = "ca:trust", about = "Trust PV local CA in the System keychain")]
    CaTrust,

    #[command(name = "ca:untrust", about = "Remove PV local CA trust")]
    CaUntrust,

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

    #[command(name = "php:use", about = "Set the PHP track")]
    PhpUse(PhpUseArgs),

    #[command(name = "php:install", about = "Install a PHP track")]
    PhpInstall(PhpInstallArgs),

    #[command(name = "php:update", about = "Update installed PHP tracks")]
    PhpUpdate,

    #[command(name = "php:uninstall", about = "Uninstall a PHP track")]
    PhpUninstall(PhpUninstallArgs),

    #[command(name = "php:list", about = "List installed PHP tracks")]
    PhpList,

    #[command(name = "composer:install", about = "Install Composer track 2")]
    ComposerInstall,

    #[command(name = "composer:update", about = "Update Composer track 2")]
    ComposerUpdate,

    #[command(name = "composer:uninstall", about = "Uninstall Composer")]
    ComposerUninstall(ComposerUninstallArgs),

    #[command(name = "mailpit:install", about = "Install a Mailpit track")]
    MailpitInstall(MailpitInstallArgs),

    #[command(name = "mailpit:update", about = "Update installed Mailpit tracks")]
    MailpitUpdate,

    #[command(name = "mailpit:uninstall", about = "Uninstall a Mailpit track")]
    MailpitUninstall(MailpitUninstallArgs),

    #[command(name = "mailpit:list", about = "List installed Mailpit tracks")]
    MailpitList,

    #[command(name = "mailpit:open", about = "Open the running Mailpit dashboard")]
    MailpitOpen,

    #[command(name = "mail:install", about = "Install a Mailpit track")]
    MailInstall(MailpitInstallArgs),

    #[command(name = "mail:update", about = "Update installed Mailpit tracks")]
    MailUpdate,

    #[command(name = "mail:uninstall", about = "Uninstall a Mailpit track")]
    MailUninstall(MailpitUninstallArgs),

    #[command(name = "mail:list", about = "List installed Mailpit tracks")]
    MailList,

    #[command(name = "mail:open", about = "Open the running Mailpit dashboard")]
    MailOpen,

    #[command(name = "redis:install", about = "Install a Redis track")]
    RedisInstall(RedisInstallArgs),

    #[command(name = "redis:update", about = "Update installed Redis tracks")]
    RedisUpdate,

    #[command(name = "redis:uninstall", about = "Uninstall a Redis track")]
    RedisUninstall(RedisUninstallArgs),

    #[command(name = "redis:list", about = "List installed Redis tracks")]
    RedisList,

    #[command(name = "rustfs:install", about = "Install a RustFS track")]
    RustfsInstall(RustfsInstallArgs),

    #[command(name = "rustfs:update", about = "Update installed RustFS tracks")]
    RustfsUpdate,

    #[command(name = "rustfs:uninstall", about = "Uninstall a RustFS track")]
    RustfsUninstall(RustfsUninstallArgs),

    #[command(name = "rustfs:list", about = "List installed RustFS tracks")]
    RustfsList,

    #[command(name = "rustfs:open", about = "Open the running RustFS console")]
    RustfsOpen,

    #[command(name = "s3:install", about = "Install a RustFS track (S3 alias)")]
    S3Install(RustfsInstallArgs),

    #[command(
        name = "s3:update",
        about = "Update installed RustFS tracks (S3 alias)"
    )]
    S3Update,

    #[command(name = "s3:uninstall", about = "Uninstall a RustFS track (S3 alias)")]
    S3Uninstall(RustfsUninstallArgs),

    #[command(name = "s3:list", about = "List installed RustFS tracks (S3 alias)")]
    S3List,

    #[command(name = "s3:open", about = "Open the running RustFS console (S3 alias)")]
    S3Open,

    #[command(name = "mysql:install", about = "Install a MySQL track")]
    MysqlInstall(MysqlInstallArgs),

    #[command(name = "mysql:update", about = "Update installed MySQL tracks")]
    MysqlUpdate,

    #[command(name = "mysql:uninstall", about = "Uninstall a MySQL track")]
    MysqlUninstall(MysqlUninstallArgs),

    #[command(name = "mysql:list", about = "List installed MySQL tracks")]
    MysqlList,
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
pub(crate) struct SetupArgs {
    #[arg(long, help = "Accept PV-owned setup confirmations")]
    pub(crate) yes: bool,

    #[arg(long, help = "Disable interactive prompts")]
    pub(crate) non_interactive: bool,

    #[arg(long, help = "Skip shell profile integration")]
    pub(crate) no_path: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct UninstallArgs {
    #[arg(long, help = "Remove PV-owned state under ~/.pv")]
    pub(crate) prune: bool,

    #[arg(long, help = "Skip prune confirmation")]
    pub(crate) force: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ShimArgs {
    #[arg(
        value_name = "args",
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    pub(crate) args: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PhpInstallArgs {
    #[arg(value_name = "track", help = "PHP track to install")]
    pub(crate) track: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PhpUseArgs {
    #[arg(value_name = "track", help = "PHP track to use")]
    pub(crate) track: String,

    #[arg(short = 'g', long, help = "Set the global PHP default")]
    pub(crate) global: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PhpUninstallArgs {
    #[arg(value_name = "track", help = "PHP track to uninstall")]
    pub(crate) track: String,

    #[arg(long, help = "Remove PV-owned runtime data for the track")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove the track even if Projects use it")]
    pub(crate) force: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct ComposerUninstallArgs {
    #[arg(long, help = "Remove PV-owned Composer home/cache")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove Composer even if in use")]
    pub(crate) force: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct MailpitInstallArgs {
    #[arg(value_name = "track", help = "Mailpit track to install")]
    pub(crate) track: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct MailpitUninstallArgs {
    #[arg(value_name = "track", help = "Mailpit track to uninstall")]
    pub(crate) track: String,

    #[arg(long, help = "Remove PV-owned runtime data for the track")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove the track even if Projects use it")]
    pub(crate) force: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RedisInstallArgs {
    #[arg(value_name = "track", help = "Redis track to install")]
    pub(crate) track: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RedisUninstallArgs {
    #[arg(value_name = "track", help = "Redis track to uninstall")]
    pub(crate) track: String,

    #[arg(long, help = "Remove PV-owned runtime data for the track")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove the track even if Projects use it")]
    pub(crate) force: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RustfsInstallArgs {
    #[arg(value_name = "track", help = "RustFS track to install")]
    pub(crate) track: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct RustfsUninstallArgs {
    #[arg(value_name = "track", help = "RustFS track to uninstall")]
    pub(crate) track: String,

    #[arg(long, help = "Remove PV-owned runtime data for the track")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove the track even if Projects use it")]
    pub(crate) force: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct MysqlInstallArgs {
    #[arg(value_name = "track", help = "MySQL track to install")]
    pub(crate) track: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct MysqlUninstallArgs {
    #[arg(value_name = "track", help = "MySQL track to uninstall")]
    pub(crate) track: String,

    #[arg(long, help = "Remove PV-owned runtime data for the track")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove the track even if Projects use it")]
    pub(crate) force: bool,
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
