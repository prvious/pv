use std::collections::HashMap;
use std::ffi::OsString;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ArtifactManifestCache, ConcreteTrackName, ManagedResourceCommands,
    ManagedResourceUninstallOptions, ResourceAdapter, ResourceHttpClient, ResourceName,
    TargetPlatform, TrackName, TrackSelector, UreqResourceHttpClient,
};
use serde::Serialize;
use state::{Database, ManagedResourceDesiredState, ProjectRecord, PvPaths, StateError};

use crate::args::{ListArgs, PhpInstallArgs, PhpUninstallArgs, PhpUseArgs, ShimArgs};
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Streams, Table};
use crate::progress::DownloadProgressRenderer;
use crate::prompt;

pub(crate) fn use_track(
    args: PhpUseArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let requested_track = args.track;
    let selector = TrackSelector::parse(requested_track.as_str())?;
    let commands = resource_commands(&paths, environment)?;
    let jobs_lock = super::acquire_jobs_lock(&paths)?;
    let progress = DownloadProgressRenderer::new(&streams.err);

    if args.global {
        let installed = with_resource_http_client(environment, |client| {
            commands.install_php_pair_with_progress(selector, client, &progress)
        })?;
        drop(progress);
        let output = &mut streams.out;
        let track = installed.php().track().as_str().to_string();
        let mut database = Database::open(&paths)?;
        database.record_global_php_default_track(&track)?;
        drop(jobs_lock);

        output.success(Line::field("Set global PHP track to ", track))?;
        super::write_php_pair_install_lines(&installed, streams)?;
        super::request_system_reconciliation(&paths, streams)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut database = Database::open(&paths)?;
    let project = resolve_current_project(&database, environment)?;
    config::ProjectConfigFile::read_from_root(&project.path)?;
    let installed = with_resource_http_client(environment, |client| {
        commands.install_php_pair_with_progress(selector, client, &progress)
    })?;
    drop(progress);
    let output = &mut streams.out;
    let track = installed.php().track().as_str().to_string();
    let config_file = config::write_project_php_track(&project.path, &requested_track)?;
    let project = database.replace_project_desired_php_track(&project.id, Some(&track))?;
    drop(jobs_lock);

    output.success(Line::field(
        &format!(
            "Set {} PHP track to ",
            super::project_display_name(&project)
        ),
        track,
    ))?;
    output.detail(Line::field("Updated Project config: ", config_file.path))?;
    super::write_php_pair_install_lines(&installed, streams)?;
    super::request_project_reconciliation(&paths, &project, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    args: PhpInstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = match args.track {
        Some(track) => TrackSelector::parse(track)?,
        None => TrackSelector::Latest,
    };
    let commands = resource_commands(&paths, environment)?;
    let jobs_lock = super::acquire_jobs_lock(&paths)?;
    let progress = DownloadProgressRenderer::new(&streams.err);
    let installed = with_resource_http_client(environment, |client| {
        commands.install_php_pair_with_progress(selector, client, &progress)
    })?;
    drop(progress);
    drop(jobs_lock);

    super::write_php_pair_install_lines(&installed, streams)?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment)?;
    let jobs_lock = super::acquire_jobs_lock(&paths)?;
    let progress = DownloadProgressRenderer::new(&streams.err);
    let updated = with_resource_http_client(environment, |client| {
        commands.update_php_pairs_with_progress(client, &progress)
    })?;
    drop(progress);
    drop(jobs_lock);
    let output = &mut streams.out;

    super::write_revoked_latest_warnings(updated.installs(), &mut streams.err)?;
    super::write_updated(output, updated.installs().len(), "PHP runtime artifact(s)")?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    args: PhpUninstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let track = TrackName::new(args.track)?;
    let commands = resource_commands(&paths, environment)?;
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

    if args.prune && !args.force {
        let refusal = CliError::ResourcePruneRequiresTerminal {
            display_name: "PHP",
            resource_name: "php",
            track: track.to_string(),
        };
        let message = format!("Prune PV-owned data for PHP/FrankenPHP track {track}?");
        if !prompt::confirm_or(environment, streams, refusal, &message, false)? {
            streams.out.note("Prune cancelled.")?;
            return Ok(ExitCode::SUCCESS);
        }
    }

    let options = ManagedResourceUninstallOptions::new()
        .prune(args.prune)
        .force(args.force);
    let removal = commands.uninstall_php_pair(&track, options)?;
    let output = &mut streams.out;

    output.success(Line::field(
        "Queued removal for PHP track ",
        removal.php().track(),
    ))?;
    output.success(Line::field(
        "Queued removal for FrankenPHP track ",
        removal.frankenphp().track(),
    ))?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    args: ListArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let php = ResourceName::new("php")?;
    let commands = resource_commands(&paths, environment)?;
    let database = Database::open(&paths)?;
    let tracks = commands.list(Some(&php))?;

    if tracks.is_empty() {
        if args.json {
            streams.out.json(&PhpListOutput { tracks: Vec::new() })?;

            return Ok(ExitCode::SUCCESS);
        }

        streams.out.note("No PHP tracks installed")?;
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
        streams.out.json(&PhpListOutput { tracks })?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut table = Table::new(&["Track", "Default", "Projects", "Version", "Path"]);
    for track in tracks {
        let default = if default_track.as_deref() == Some(track.track().as_str()) {
            Line::marked(Mark::Running, "yes")
        } else {
            Line::marked(Mark::Idle, "no")
        };
        let project_count = project_counts
            .get(track.track().as_str())
            .copied()
            .unwrap_or(0);
        table.row(vec![
            Line::from(track.track().as_str()),
            default,
            Line::from(project_count.to_string()),
            Line::default().value(track.installed_version().as_str()),
            Line::from(track.current_artifact_path().as_str()),
        ]);
    }
    streams.out.table(&table)?;

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
    let runtime = resolve_php_runtime_for_shim(&paths, &database, environment)?;
    let installed = installed_php(&database, &runtime.track)?;
    resources::ensure_php_track_defaults(&paths, &runtime.track)?;
    let loaded_modules = resources::resolve_persisted_php_extension_modules(
        installed.release_path(),
        &runtime.loaded_extensions,
    )?;
    env.extend(resources::php_runtime_exec_environment(
        &paths,
        &runtime.track,
        &runtime.runtime_key,
        installed.release_path(),
        &loaded_modules,
    )?);
    let executable = installed.executable()?;

    environment
        .exec_with_env(executable.as_std_path(), &args, &env)
        .map_err(ExecuteError::from)
}

struct PhpShimRuntime {
    track: String,
    runtime_key: String,
    loaded_extensions: Vec<String>,
}

fn resolve_php_runtime_for_shim(
    paths: &PvPaths,
    database: &Database,
    environment: &impl Environment,
) -> Result<PhpShimRuntime, ExecuteError> {
    let current_dir = current_dir(environment)?;
    if let Some(project) = database.nearest_project_for_path(&current_dir)? {
        let config_file = match config::ProjectConfigFile::read_from_root(&project.path) {
            Ok(config_file) => config_file,
            Err(error) => {
                if let Some(runtime) = persisted_project_php_runtime_for_shim(&project)? {
                    return Ok(runtime);
                }

                return Err(error.into());
            }
        };
        if let Some(track) = project.php_runtime.track.clone() {
            let php = config_file.config.php.as_ref();
            let requested_extensions = php
                .map(|php| php.requested_extensions().to_vec())
                .unwrap_or_default();
            let config_track = if let Some(php) = php {
                match php.version_selector() {
                    Some(selector) if selector != "latest" && selector != track => Some(
                        resolve_project_config_php_track_for_shim(paths, database, php)?,
                    ),
                    None if database.global_php_default_track()?.is_some()
                        || paths.downloads().join("manifest.json").exists() =>
                    {
                        let resolved_track =
                            resolve_project_config_php_track_for_shim(paths, database, php)?;
                        if resolved_track == track {
                            None
                        } else {
                            Some(resolved_track)
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };
            let current_track = config_track.as_deref().unwrap_or(&track).to_string();
            if requested_extensions.is_empty() {
                return Ok(PhpShimRuntime {
                    runtime_key: current_track.clone(),
                    track: current_track,
                    loaded_extensions: Vec::new(),
                });
            }
            if config_track.is_none()
                && project.php_runtime.requested_extensions == requested_extensions
            {
                let runtime_key =
                    state::php_runtime_key(&track, &project.php_runtime.loaded_extensions)?;

                return Ok(PhpShimRuntime {
                    track,
                    runtime_key,
                    loaded_extensions: project.php_runtime.loaded_extensions,
                });
            }
            if let Some(php) = php {
                return resolve_project_config_php_runtime_for_shim(database, &current_track, php);
            }
        } else if let Some(php) = config_file.config.php.as_ref() {
            let track = resolve_project_config_php_track_for_shim(paths, database, php)?;

            return resolve_project_config_php_runtime_for_shim(database, &track, php);
        }
    }

    let track = match database.global_php_default_track()? {
        Some(track) => track,
        None => {
            let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
            let php = ResourceName::new("php")?;

            manifest
                .resolve_track(&php, TrackSelector::Latest)?
                .as_str()
                .to_string()
        }
    };

    Ok(PhpShimRuntime {
        runtime_key: track.clone(),
        track,
        loaded_extensions: Vec::new(),
    })
}

fn persisted_project_php_runtime_for_shim(
    project: &ProjectRecord,
) -> Result<Option<PhpShimRuntime>, ExecuteError> {
    let Some(track) = project.php_runtime.track.clone() else {
        return Ok(None);
    };
    let loaded_extensions = project.php_runtime.loaded_extensions.clone();
    let runtime_key = state::php_runtime_key(&track, &loaded_extensions)?;

    Ok(Some(PhpShimRuntime {
        track,
        runtime_key,
        loaded_extensions,
    }))
}

fn resolve_project_config_php_track_for_shim(
    paths: &PvPaths,
    database: &Database,
    php: &config::PhpConfig,
) -> Result<String, ExecuteError> {
    let selector = php
        .version_selector()
        .map(TrackSelector::parse)
        .transpose()?;
    let track = match selector {
        Some(TrackSelector::Latest) => {
            let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
            let php = ResourceName::new("php")?;

            manifest
                .resolve_track(&php, TrackSelector::Latest)?
                .as_str()
                .to_string()
        }
        Some(TrackSelector::Track(track)) => track.as_str().to_string(),
        None => match database.global_php_default_track()? {
            Some(track) => track,
            None => {
                let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
                let php = ResourceName::new("php")?;

                manifest
                    .resolve_track(&php, TrackSelector::Latest)?
                    .as_str()
                    .to_string()
            }
        },
    };
    let track = ConcreteTrackName::new(track)?;

    Ok(track.as_str().to_string())
}

fn resolve_project_config_php_runtime_for_shim(
    database: &Database,
    track: &str,
    php: &config::PhpConfig,
) -> Result<PhpShimRuntime, ExecuteError> {
    let requested_extensions = php.requested_extensions().to_vec();
    if requested_extensions.is_empty() {
        return Ok(PhpShimRuntime {
            runtime_key: track.to_string(),
            track: track.to_string(),
            loaded_extensions: Vec::new(),
        });
    }

    let installed = installed_php(database, track)?;
    let resolution =
        resources::resolve_php_extension_request(installed.release_path(), &requested_extensions)?;
    let loaded_extensions = resolution
        .loaded
        .iter()
        .map(|module| module.name.clone())
        .collect::<Vec<_>>();
    let runtime_key = state::php_runtime_key(track, &loaded_extensions)?;

    Ok(PhpShimRuntime {
        track: track.to_string(),
        runtime_key,
        loaded_extensions,
    })
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
}

impl InstalledPhp {
    fn executable(&self) -> Result<Utf8PathBuf, ExecuteError> {
        let adapter = resources::php_adapter()?;

        Ok(adapter.executable_path(&self.release))
    }

    fn release_path(&self) -> &Utf8Path {
        &self.release
    }
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

    Ok(InstalledPhp { release })
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

fn resource_commands(
    paths: &PvPaths,
    environment: &impl Environment,
) -> Result<ManagedResourceCommands, ExecuteError> {
    Ok(ManagedResourceCommands::new(
        paths.clone(),
        artifact_manifest_url(environment),
        target_platform(environment)?,
    ))
}

fn target_platform(environment: &impl Environment) -> Result<TargetPlatform, ExecuteError> {
    Ok(environment.resolve_target_platform()?)
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;

    use resources::{ResourcesError, TargetPlatform};

    use super::target_platform;
    use crate::environment::Environment;
    use crate::error::ExecuteError;

    struct UnsupportedPlatformEnvironment;

    impl Environment for UnsupportedPlatformEnvironment {
        fn var_os(&self, _key: &str) -> Option<OsString> {
            None
        }

        fn home_dir(&self) -> Option<PathBuf> {
            None
        }

        fn current_dir(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::new())
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::new())
        }

        fn stdin_is_terminal(&self) -> bool {
            false
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }

        fn resolve_target_platform(&self) -> resources::Result<TargetPlatform> {
            Err(ResourcesError::UnsupportedPlatform {
                platform: "linux-aarch64".to_string(),
            })
        }
    }

    #[test]
    fn target_platform_preserves_unsupported_platform_error() {
        let result = target_platform(&UnsupportedPlatformEnvironment);

        assert!(matches!(
            result,
            Err(ExecuteError::Resources(ResourcesError::UnsupportedPlatform { platform }))
                if platform == "linux-aarch64"
        ));
    }
}
