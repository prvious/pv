use std::io;

use thiserror::Error;

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

    #[error("{command} is routed, but LaunchAgent lifecycle management starts after PV-055")]
    DeferredDaemonLifecycle { command: &'static str },

    #[error("path is not valid UTF-8: {path:?}")]
    NonUtf8Path { path: std::path::PathBuf },

    #[error("could not resolve a linked Project; pass a hostname")]
    ProjectNotResolved,

    #[error("invalid Project picker selection `{selection}`; enter a number from 1 to {count}")]
    InvalidProjectSelection { selection: String, count: usize },
}

#[derive(Debug, Error)]
pub(crate) enum ExecuteError {
    #[error(transparent)]
    User(#[from] CliError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Daemon(#[from] daemon::DaemonError),

    #[error(transparent)]
    Config(#[from] config::ConfigError),

    #[error(transparent)]
    State(#[from] state::StateError),
}
