use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ArtifactManifestCache, ManagedResourceCommands, ManagedResourceUninstallOptions,
    ResourceAdapter, ResourceHttpClient, ResourceName, TargetPlatform, TrackName, TrackSelector,
    UreqResourceHttpClient,
};
use state::{Database, ManagedResourceDesiredState, PvPaths, StateError};

use crate::args::{ComposerUninstallArgs, ShimArgs};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

const DEFAULT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";
const COMPOSER_TRACK: &str = "2";
const RECONCILE_KIND: &str = "reconcile";
const SYSTEM_SCOPE: &str = "system";

pub(crate) fn install(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment);
    let database = Database::open(&paths)?;
    let php_track = resolved_global_php_track(&paths, &database)?;
    let php_track = TrackName::new(php_track)?;
    let php_pair = with_resource_http_client(environment, |client| {
        commands.install_php_pair(TrackSelector::Track(php_track), client)
    })?;
    let composer =
        with_resource_http_client(environment, |client| commands.install_composer(client))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!("Installed PHP track {}", php_pair.php().track()))?;
    output.line(&format!(
        "Installed FrankenPHP track {}",
        php_pair.frankenphp().track()
    ))?;
    output.line(&format!("Installed Composer track {}", composer.track()))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment);
    let updated =
        with_resource_http_client(environment, |client| commands.update_composer(client))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Updated {} Composer track(s)",
        updated.installs().len()
    ))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    args: ComposerUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment);
    let options = ManagedResourceUninstallOptions::new()
        .prune(args.prune)
        .force(args.force);
    let removal = commands.uninstall_composer(options)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Queued removal for Composer track {}",
        removal.track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn shim(
    args: ShimArgs,
    environment: &impl Environment,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let phar = installed_composer_phar(&database)?;
    let mut shim_args = Vec::with_capacity(args.args.len() + 1);
    shim_args.push(phar.to_string());
    shim_args.extend(args.args);

    super::php::shim_with_args(shim_args, environment)
}

fn resolved_global_php_track(paths: &PvPaths, database: &Database) -> Result<String, ExecuteError> {
    if let Some(track) = database.global_php_default_track()? {
        return Ok(track);
    }

    let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
    let php = ResourceName::new("php")?;

    Ok(manifest
        .resolve_track(&php, TrackSelector::Latest)?
        .as_str()
        .to_string())
}

fn installed_composer_phar(database: &Database) -> Result<Utf8PathBuf, ExecuteError> {
    let Some(record) = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| {
            record.resource_name == "composer"
                && record.track == COMPOSER_TRACK
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
    else {
        return Err(CliError::MissingComposer.into());
    };
    let release = record
        .current_artifact_path
        .ok_or(CliError::MissingComposer)?;
    let adapter = resources::composer_adapter()?;
    adapter.validate_installation(&release)?;

    Ok(adapter.executable_path(&release))
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
