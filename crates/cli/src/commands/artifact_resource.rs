#![expect(
    dead_code,
    reason = "adapter command modules reuse this private helper in follow-up PRs"
)]

use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ManagedResourceCommands, ManagedResourceUninstallOptions, ResourceHttpClient, ResourceName,
    TargetPlatform, TrackName, TrackSelector, UreqResourceHttpClient,
};
use state::{PvPaths, StateError};

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

const DEFAULT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";
const RECONCILE_KIND: &str = "reconcile";
const SYSTEM_SCOPE: &str = "system";

pub(crate) struct ArtifactResourceCommandSpec {
    pub resource_name: &'static str,
    pub display_name: &'static str,
    pub adapter: fn() -> resources::Result<resources::RuntimeArtifactAdapter>,
}

pub(crate) fn install(
    spec: ArtifactResourceCommandSpec,
    track: Option<&str>,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = match track {
        Some(track) => TrackSelector::parse(track)?,
        None => TrackSelector::Latest,
    };
    let adapter = (spec.adapter)()?;
    let commands = resource_commands(&paths, environment);
    let installed = with_resource_http_client(environment, |client| {
        commands.install(&adapter, selector, client)
    })?;
    let mut output = Output::new(stdout, OutputMode::plain());

    super::write_revoked_latest_warning(&installed, &mut output)?;
    output.line(&format!(
        "Installed {} track {}",
        spec.display_name,
        installed.track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(
    spec: ArtifactResourceCommandSpec,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let adapter = (spec.adapter)()?;
    let commands = resource_commands(&paths, environment);
    let updated =
        with_resource_http_client(environment, |client| commands.update(&adapter, client))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    super::write_revoked_latest_warnings(updated.installs(), &mut output)?;
    output.line(&format!(
        "Updated {} {} track(s)",
        updated.installs().len(),
        spec.display_name
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    spec: ArtifactResourceCommandSpec,
    track: &str,
    prune: bool,
    force: bool,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let resource_name = ResourceName::new(spec.resource_name)?;
    let track = TrackName::new(track)?;
    let options = ManagedResourceUninstallOptions::new()
        .prune(prune)
        .force(force);
    let commands = resource_commands(&paths, environment);
    let removal = commands.uninstall(&resource_name, &track, options)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Queued removal for {} track {}",
        spec.display_name,
        removal.track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    spec: ArtifactResourceCommandSpec,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let resource_name = ResourceName::new(spec.resource_name)?;
    let commands = resource_commands(&paths, environment);
    let tracks = commands.list(Some(&resource_name))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    if tracks.is_empty() {
        output.line(&format!("No {} tracks installed", spec.display_name))?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Track  Projects  Version  Path")?;
    for track in tracks {
        output.line(&format!(
            "{}  {}  {}  {}",
            track.track(),
            track.usage_count(),
            track.installed_version(),
            track.current_artifact_path()
        ))?;
    }

    Ok(ExitCode::SUCCESS)
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn resource_commands(paths: &PvPaths, environment: &impl Environment) -> ManagedResourceCommands {
    ManagedResourceCommands::new(
        paths.clone(),
        environment
            .artifact_manifest_url()
            .unwrap_or_else(|| DEFAULT_MANIFEST_URL.to_string()),
        target_platform(environment),
    )
}

fn target_platform(environment: &impl Environment) -> TargetPlatform {
    environment
        .target_platform()
        .unwrap_or_else(current_target_platform)
}

fn current_target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
}

fn with_resource_http_client<T>(
    environment: &impl Environment,
    operation: impl FnOnce(&dyn ResourceHttpClient) -> Result<T, resources::ManagedResourceCommandError>,
) -> Result<T, ExecuteError> {
    if let Some(client) = environment.resource_http_client() {
        return Ok(operation(client)?);
    }

    let client = UreqResourceHttpClient::default();
    Ok(operation(&client)?)
}

fn request_system_reconciliation(
    paths: &PvPaths,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    match daemon::submit_job_blocking(paths.clone(), RECONCILE_KIND, SYSTEM_SCOPE) {
        Ok(job) => output.line(&format!("System reconciliation requested: {}", job.id))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => {
            write_daemon_unavailable_warning(output)?
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn write_daemon_unavailable_warning(
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    output.line(
        "warning: PV daemon is not running; reconciliation will run after `pv setup` starts it",
    )?;

    Ok(())
}

fn daemon_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}
