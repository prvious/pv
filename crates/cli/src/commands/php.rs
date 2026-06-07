use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ArtifactManifestCache, ManagedResourceCommands, ManagedResourceUninstallOptions,
    ResourceAdapter, ResourceHttpClient, ResourceName, TargetPlatform, TrackName, TrackSelector,
    UreqResourceHttpClient,
};
use state::{Database, ManagedResourceDesiredState, ProjectRecord, PvPaths, StateError};

use crate::args::{PhpInstallArgs, PhpUninstallArgs, PhpUseArgs, ShimArgs};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

const DEFAULT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";
const RECONCILE_KIND: &str = "reconcile";
const SYSTEM_SCOPE: &str = "system";

pub(crate) fn use_track(
    args: PhpUseArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let requested_track = args.track;
    let selector = TrackSelector::parse(requested_track.as_str())?;
    let commands = resource_commands(&paths, environment);
    let mut output = Output::new(stdout, OutputMode::plain());

    if args.global {
        let installed = with_resource_http_client(environment, |client| {
            commands.install_php_pair(selector, client)
        })?;
        let track = installed.php().track().as_str().to_string();
        let mut database = Database::open(&paths)?;
        database.record_global_php_default_track(&track)?;

        output.line(&format!("Set global PHP track to {track}"))?;
        write_install_lines(&installed, &mut output)?;
        request_system_reconciliation(&paths, &mut output)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut database = Database::open(&paths)?;
    let project = resolve_current_project(&database, environment)?;
    config::ProjectConfigFile::read_from_root(&project.path)?;
    let installed = with_resource_http_client(environment, |client| {
        commands.install_php_pair(selector, client)
    })?;
    let track = installed.php().track().as_str().to_string();
    let config_file = config::write_project_php_track(&project.path, &requested_track)?;
    let project = database.replace_project_desired_php_track(&project.id, Some(&track))?;

    output.line(&format!(
        "Set {} PHP track to {track}",
        project.primary_hostname
    ))?;
    output.line(&format!("Updated Project config: {}", config_file.path))?;
    write_install_lines(&installed, &mut output)?;
    request_project_reconciliation(&paths, &project, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    args: PhpInstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = match args.track {
        Some(track) => TrackSelector::parse(track)?,
        None => TrackSelector::Latest,
    };
    let commands = resource_commands(&paths, environment);
    let installed = with_resource_http_client(environment, |client| {
        commands.install_php_pair(selector, client)
    })?;
    let mut output = Output::new(stdout, OutputMode::plain());

    write_install_lines(&installed, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment);
    let updated =
        with_resource_http_client(environment, |client| commands.update_php_pairs(client))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Updated {} PHP runtime artifact(s)",
        updated.installs().len()
    ))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    args: PhpUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let track = TrackName::new(args.track)?;
    if !args.force {
        let database = Database::open(&paths)?;
        let usage_count = active_php_selection_usage_count(&database, &track)?;
        if usage_count > 0 {
            return Err(CliError::PhpTrackInUse {
                track: track.as_str().to_string(),
                usage_count,
            }
            .into());
        }
    }

    let options = ManagedResourceUninstallOptions::new()
        .prune(args.prune)
        .force(args.force);
    let commands = resource_commands(&paths, environment);
    let removal = commands.uninstall_php_pair(&track, options)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Queued removal for PHP track {}",
        removal.php().track()
    ))?;
    output.line(&format!(
        "Queued removal for FrankenPHP track {}",
        removal.frankenphp().track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let default_track = database.global_php_default_track()?;
    let project_counts = php_project_selection_counts(&database, default_track.as_deref())?;
    let php = ResourceName::new("php")?;
    let commands = resource_commands(&paths, environment);
    let tracks = commands.list(Some(&php))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    if tracks.is_empty() {
        output.line("No PHP tracks installed")?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Track  Default  Projects  Version  Path")?;
    for track in tracks {
        let default_marker = if default_track.as_deref() == Some(track.track().as_str()) {
            "yes"
        } else {
            "no"
        };
        let project_count = if let Some(count) = project_counts.get(track.track().as_str()) {
            *count
        } else {
            0
        };
        output.line(&format!(
            "{}  {}  {}  {}  {}",
            track.track(),
            default_marker,
            project_count,
            track.installed_version(),
            track.current_artifact_path()
        ))?;
    }

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn shim(
    args: ShimArgs,
    environment: &impl Environment,
) -> Result<ExitCode, ExecuteError> {
    shim_with_args(args.args, environment)
}

pub(crate) fn shim_with_args(
    args: Vec<String>,
    environment: &impl Environment,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let track = resolve_php_track_for_shim(&paths, &database, environment)?;
    let executable = installed_php_executable(&database, &track)?;

    environment
        .exec(executable.as_std_path(), &args)
        .map_err(ExecuteError::from)
}

fn resolve_php_track_for_shim(
    paths: &PvPaths,
    database: &Database,
    environment: &impl Environment,
) -> Result<String, ExecuteError> {
    let current_dir = current_dir(environment)?;
    if let Some(project) = database.nearest_project_for_path(&current_dir)?
        && let Some(track) = project.desired_php_track
    {
        return Ok(track);
    }

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

fn installed_php_executable(database: &Database, track: &str) -> Result<Utf8PathBuf, ExecuteError> {
    let Some(record) = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| {
            record.resource_name == "php"
                && record.track == track
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
    else {
        return Err(CliError::MissingPhpTrack {
            track: track.to_string(),
        }
        .into());
    };
    let release = record
        .current_artifact_path
        .ok_or_else(|| CliError::MissingPhpTrack {
            track: track.to_string(),
        })?;
    let adapter = resources::php_adapter()?;
    adapter.validate_installation(&release)?;

    Ok(adapter.executable_path(&release))
}

fn write_install_lines(
    installed: &resources::PhpPairInstall,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    output.line(&format!("Installed PHP track {}", installed.php().track()))?;
    output.line(&format!(
        "Installed FrankenPHP track {}",
        installed.frankenphp().track()
    ))?;

    Ok(())
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn current_dir(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.current_dir()?)
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn resolve_current_project(
    database: &Database,
    environment: &impl Environment,
) -> Result<ProjectRecord, ExecuteError> {
    let current_dir = current_dir(environment)?;

    database
        .nearest_project_for_path(&current_dir)?
        .ok_or_else(|| CliError::ProjectNotResolved.into())
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

fn active_php_selection_usage_count(
    database: &Database,
    track: &TrackName,
) -> Result<i64, ExecuteError> {
    let mut usage_count = 0_i64;
    for project in database.projects()? {
        if project.desired_php_track.as_deref() == Some(track.as_str()) {
            usage_count += 1;
        }
    }
    if database.global_php_default_track()?.as_deref() == Some(track.as_str()) {
        usage_count += 1;
    }

    Ok(usage_count)
}

fn php_project_selection_counts(
    database: &Database,
    default_track: Option<&str>,
) -> Result<HashMap<String, i64>, ExecuteError> {
    let mut counts = HashMap::new();
    for project in database.projects()? {
        let track = if let Some(track) = project.desired_php_track.as_deref() {
            Some(track)
        } else {
            default_track
        };

        if let Some(track) = track {
            let count = counts.entry(track.to_string()).or_insert(0);
            *count += 1;
        }
    }

    Ok(counts)
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

fn request_project_reconciliation(
    paths: &PvPaths,
    project: &ProjectRecord,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let scope = format!("project:{}", project.id);
    match daemon::submit_job_blocking(paths.clone(), RECONCILE_KIND, &scope) {
        Ok(job) => output.line(&format!(
            "Queued reconciliation {} for {}",
            job.id, project.primary_hostname
        ))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => {
            write_daemon_unavailable_warning(output)?
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
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
