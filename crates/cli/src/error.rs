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
}

#[derive(Debug, Error)]
pub(crate) enum ExecuteError {
    #[error(transparent)]
    User(#[from] CliError),

    #[error(transparent)]
    Io(#[from] io::Error),
}
