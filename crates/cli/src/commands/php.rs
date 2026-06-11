use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ArtifactManifestCache, ManagedResourceCommands, ManagedResourceUninstallOptions,
    ResourceAdapter, ResourceHttpClient, ResourceName, TargetPlatform, TrackName, TrackSelector,
    UreqResourceHttpClient,
};
use serde::Serialize;
use state::{Database, ManagedResourceDesiredState, ProjectRecord, PvPaths, StateError};

use crate::args::{ListArgs, PhpInstallArgs, PhpUninstallArgs, PhpUseArgs, ShimArgs};
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

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
    request_system_reconciliation(&paths, &mut output)?;

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

    super::write_revoked_latest_warnings(updated.installs(), &mut output)?;
    output.line(&format!(
        "Updated {} PHP runtime artifact(s)",
        updated.installs().len()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

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
        let default_track = effective_global_php_default_track(&paths, &database)?;
        let usage_count =
            active_php_selection_usage_count(&database, default_track.as_deref(), &track)?;
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
    args: ListArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let php = ResourceName::new("php")?;
    let commands = resource_commands(&paths, environment);
    let tracks = commands.list(Some(&php))?;

    if tracks.is_empty() {
        if args.json {
            serde_json::to_writer(&mut *stdout, &PhpListOutput { tracks: Vec::new() })?;
            writeln!(stdout)?;

            return Ok(ExitCode::SUCCESS);
        }

        let mut output = Output::new(stdout, OutputMode::plain());
        output.line("No PHP tracks installed")?;
        return Ok(ExitCode::SUCCESS);
    }

    let default_track = effective_global_php_default_track(&paths, &database)?;
    let project_counts = php_project_selection_counts(&database, default_track.as_deref())?;

    if args.json {
        let tracks = tracks
            .iter()
            .map(|track| {
                let track_name = track.track().as_str();
                PhpListTrack {
                    track: track_name.to_string(),
                    default: default_track.as_deref() == Some(track_name),
                    projects: project_counts.get(track_name).copied().unwrap_or(0),
                    version: track.installed_version().as_str().to_string(),
                    path: track.current_artifact_path().to_string(),
                }
            })
            .collect::<Vec<_>>();
        serde_json::to_writer(&mut *stdout, &PhpListOutput { tracks })?;
        writeln!(stdout)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut output = Output::new(stdout, OutputMode::plain());
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

#[derive(Serialize)]
struct PhpListOutput {
    tracks: Vec<PhpListTrack>,
}

#[derive(Serialize)]
struct PhpListTrack {
    track: String,
    default: bool,
    projects: i64,
    version: String,
    path: String,
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
    shim_with_args_and_env(args, Vec::new(), environment)
}

pub(crate) fn shim_with_args_and_env(
    args: Vec<String>,
    mut env: Vec<(OsString, OsString)>,
    environment: &impl Environment,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let track = resolve_php_track_for_shim(&paths, &database, environment)?;
    let installed = installed_php(&database, &track)?;
    env.extend(php_env_overlay(&installed.release));

    environment
        .exec_with_env(installed.executable.as_std_path(), &args, &env)
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

fn effective_global_php_default_track(
    paths: &PvPaths,
    database: &Database,
) -> Result<Option<String>, ExecuteError> {
    if let Some(track) = database.global_php_default_track()? {
        return Ok(Some(track));
    }
    if !paths.downloads().join("manifest.json").exists() {
        return Ok(None);
    }

    let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
    let php = ResourceName::new("php")?;
    let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

    Ok(Some(track.as_str().to_string()))
}

struct InstalledPhp {
    release: Utf8PathBuf,
    executable: Utf8PathBuf,
}

fn installed_php(database: &Database, track: &str) -> Result<InstalledPhp, ExecuteError> {
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
    let executable = adapter.executable_path(&release);

    Ok(InstalledPhp {
        release,
        executable,
    })
}

fn php_env_overlay(release: &Utf8Path) -> Vec<(OsString, OsString)> {
    vec![
        (
            OsString::from("PHPRC"),
            release.join("etc").as_std_path().as_os_str().to_os_string(),
        ),
        (
            OsString::from("PHP_INI_SCAN_DIR"),
            release
                .join("etc/conf.d")
                .as_std_path()
                .as_os_str()
                .to_os_string(),
        ),
    ]
}

fn write_install_lines(
    installed: &resources::PhpPairInstall,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    super::write_revoked_latest_warning(installed.php(), output)?;
    super::write_revoked_latest_warning(installed.frankenphp(), output)?;
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
        artifact_manifest_url(environment),
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
    default_track: Option<&str>,
    track: &TrackName,
) -> Result<i64, ExecuteError> {
    let mut usage_count = 0_i64;
    for project in database.projects()? {
        let project_track = project.desired_php_track.as_deref().or(default_track);
        if project_track == Some(track.as_str()) {
            usage_count += 1;
        }
    }
    if default_track == Some(track.as_str()) {
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
