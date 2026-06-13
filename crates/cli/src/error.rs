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

    #[error(
        "PHP track `{track}` has {usage_count} active Project/global default selection(s); use --force to remove it anyway"
    )]
    PhpTrackInUse { track: String, usage_count: i64 },

    #[error("PHP track {track} is not installed.\nRun `pv php:install {track}` to install it.")]
    MissingPhpTrack { track: String },

    #[error("Composer track 2 is not installed.\nRun `pv composer:install` to install it.")]
    MissingComposer,

    #[error("log line count must be zero or greater")]
    InvalidLogLineCount,

    #[error("multiple {resource} tracks are installed; pass --track with one of: {tracks}")]
    AmbiguousLogResourceTrack { resource: String, tracks: String },

    #[error("no installed {resource} tracks were found; pass --track explicitly")]
    MissingLogResourceTrack { resource: String },

    #[error("PV update is already in progress; update lock is held at {path}")]
    UpdateInProgress { path: String },

    #[error("PV application update requires an installed active release: {message}")]
    AppUpdateInvalidActiveRelease { message: String },

    #[error(
        "PV application update requires active release {current_version}, found {active_version}"
    )]
    AppUpdateActiveReleaseMismatch {
        active_version: String,
        current_version: String,
    },

    #[error(
        "PV application update requires the PV LaunchAgent at {path}; run `pv setup` or `pv daemon:enable`"
    )]
    AppUpdateLaunchAgentMissing { path: String },

    #[error("PV LaunchAgent file is not PV-owned; leaving it unchanged at {path}")]
    AppUpdateLaunchAgentConflict { path: String },

    #[error("PV LaunchAgent file is unreadable; leaving it unchanged: {message}")]
    AppUpdateLaunchAgentUnreadable { message: String },

    #[error("PV app download size mismatch for `{url}`: expected {expected}, got {actual}")]
    AppUpdateSizeMismatch {
        url: String,
        expected: u64,
        actual: u64,
    },

    #[error("PV app download checksum mismatch for `{url}`: expected {expected}, got {actual}")]
    AppUpdateChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },

    #[error("{message}")]
    AppUpdatePostActivationFailed { message: String },

    #[error("PV application update failed ({original}); rollback failed: {rollback}")]
    AppUpdateRollbackFailed { original: String, rollback: String },

    #[error(
        "PV application update failed ({original}); previous release was restored, but daemon restart after rollback failed: {rollback}. Run `pv daemon:restart` or `pv setup`."
    )]
    AppUpdateRollbackDaemonFailed { original: String, rollback: String },

    #[error("pv update --check requires the PV daemon; run `pv daemon:restart` or `pv setup`")]
    UpdateCheckDaemonUnavailable,

    #[error("pv update --check failed: {message}")]
    UpdateCheckFailed { message: String },

    #[error(
        "PV application update succeeded, but Managed Resource update continuation failed to start: {message}. Run `pv update` again to update Managed Resources."
    )]
    ManagedResourceUpdateContinuationFailed { message: String },

    #[error(
        "pv update requires the PV daemon for Managed Resource updates; run `pv daemon:restart` or `pv setup`"
    )]
    ManagedResourceUpdateDaemonUnavailable,

    #[error("pv update Managed Resource phase failed: {message}")]
    ManagedResourceUpdateFailed { message: String },
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

    #[error(transparent)]
    SelfUpdate(#[from] self_update::AppUpdateManifestError),
}
