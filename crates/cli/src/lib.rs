use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::generate;
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

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(name = "env", about = "Print shell integration code")]
    Env(EnvArgs),

    #[command(name = "completions", about = "Generate shell completions")]
    Completions(CompletionsArgs),

    #[command(name = "php:install", about = "Install a PHP track")]
    PhpInstall(PhpInstallArgs),
}

#[derive(Debug, clap::Args)]
struct EnvArgs {
    #[arg(long, value_enum, help = "Shell syntax to generate")]
    shell: Option<Shell>,
}

#[derive(Debug, clap::Args)]
struct CompletionsArgs {
    #[arg(value_enum, help = "Shell to generate completions for")]
    shell: Shell,
}

#[derive(Debug, clap::Args)]
struct PhpInstallArgs {
    #[arg(help = "PHP track to install")]
    track: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum Shell {
    Bash,
    Fish,
    Zsh,
}

impl Shell {
    fn detect(shell_path: &OsStr) -> Option<Self> {
        let file_name = Path::new(shell_path).file_name()?;
        let shell_name = file_name.to_string_lossy();

        match shell_name.as_ref() {
            "bash" => Some(Self::Bash),
            "fish" => Some(Self::Fish),
            "zsh" => Some(Self::Zsh),
            _ => None,
        }
    }

    fn env_script(self) -> &'static str {
        match self {
            Self::Bash | Self::Zsh => POSIX_ENV_SCRIPT,
            Self::Fish => FISH_ENV_SCRIPT,
        }
    }

    fn completion_shell(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Fish => clap_complete::Shell::Fish,
            Self::Zsh => clap_complete::Shell::Zsh,
        }
    }
}

const POSIX_ENV_SCRIPT: &str = r#"pv_prepend_path() {
  case ":$PATH:" in
    *":$1:"*) ;;
    *) PATH="$1${PATH:+:$PATH}" ;;
  esac
}

export COMPOSER_HOME="$HOME/.pv/composer"
export COMPOSER_CACHE_DIR="$HOME/.pv/composer/cache"
pv_prepend_path "$HOME/.pv/composer/vendor/bin"
pv_prepend_path "$HOME/.pv/bin"
export PATH
unset -f pv_prepend_path
"#;

const FISH_ENV_SCRIPT: &str = r#"set -gx COMPOSER_HOME "$HOME/.pv/composer"
set -gx COMPOSER_CACHE_DIR "$HOME/.pv/composer/cache"
contains -- "$HOME/.pv/composer/vendor/bin" $PATH; or set -gx PATH "$HOME/.pv/composer/vendor/bin" $PATH
contains -- "$HOME/.pv/bin" $PATH; or set -gx PATH "$HOME/.pv/bin" $PATH
"#;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("could not detect the current shell; pass --shell zsh, --shell bash, or --shell fish")]
    MissingShell,

    #[error(
        "detected unsupported shell `{shell}`; pass --shell zsh, --shell bash, or --shell fish"
    )]
    UnsupportedDetectedShell { shell: String },

    #[error("{command} is routed, but Managed Resource installs start after PV-023")]
    DeferredCommand { command: &'static str },
}

#[derive(Debug, Error)]
enum ExecuteError {
    #[error(transparent)]
    User(#[from] CliError),

    #[error(transparent)]
    Io(#[from] io::Error),
}

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
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => {
            let status_code = error.exit_code();
            write_clap_error(error, stdout, stderr)?;
            return Ok(exit_code(status_code));
        }
    };

    match execute(cli, environment, stdout) {
        Ok(exit_code) => Ok(exit_code),
        Err(ExecuteError::User(error)) => {
            let mut output = Output::new(stderr, output_mode);
            output.error(&error.to_string())?;

            Ok(ExitCode::FAILURE)
        }
        Err(ExecuteError::Io(error)) => Err(error.into()),
    }
}

fn execute(
    cli: Cli,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    match cli.command {
        Command::Env(args) => {
            let shell = match args.shell {
                Some(shell) => shell,
                None => detect_shell(environment)?,
            };
            let mut output = Output::new(
                stdout,
                OutputMode {
                    no_color: cli.no_color,
                },
            );
            output.line(shell.env_script())?;

            Ok(ExitCode::SUCCESS)
        }
        Command::Completions(args) => {
            let mut command = Cli::command();
            generate(args.shell.completion_shell(), &mut command, "pv", stdout);

            Ok(ExitCode::SUCCESS)
        }
        Command::PhpInstall(args) => {
            let _track = args.track;

            Err(CliError::DeferredCommand {
                command: "php:install",
            }
            .into())
        }
    }
}

fn detect_shell(environment: &impl Environment) -> Result<Shell, CliError> {
    let shell_path = environment.var_os("SHELL").ok_or(CliError::MissingShell)?;
    Shell::detect(&shell_path).ok_or_else(|| CliError::UnsupportedDetectedShell {
        shell: shell_path.to_string_lossy().into_owned(),
    })
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
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        write!(stdout, "{error}")
    } else {
        write!(stderr, "{error}")
    }
}
