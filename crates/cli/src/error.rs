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

    #[error("path is not valid UTF-8: {path:?}")]
    NonUtf8Path { path: std::path::PathBuf },

    #[error("could not resolve a linked Project; pass a hostname")]
    ProjectNotResolved,

    #[error("invalid Project picker selection `{selection}`; enter a number from 1 to {count}")]
    InvalidProjectSelection { selection: String, count: usize },

    #[error(
        "artifact manifest cache is unavailable at `{path}`; setup cannot plan default Managed Resources"
    )]
    MissingSetupArtifactManifest { path: String },

    #[error("active pf redirects do not match the prepared PV port redirect config")]
    PfRedirectsInactive,
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

    #[error(transparent)]
    Platform(#[from] platform::PlatformError),

    #[error(transparent)]
    Resources(#[from] resources::ResourcesError),

    #[error(transparent)]
    ManagedResourceCommand(#[from] resources::ManagedResourceCommandError),
}
