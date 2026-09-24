use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ManagedResourceCommands, ManagedResourceUninstallOptions, ResourceAdapter, ResourceHttpClient,
    TargetPlatform, TrackName, TrackSelector, UreqResourceHttpClient,
};
use state::{Database, ManagedResourceDesiredState, PvPaths, StateError};

use crate::args::{ComposerUninstallArgs, ShimArgs};
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Streams};
use crate::progress::DownloadProgressRenderer;

const COMPOSER_TRACK: &str = "2";

pub(crate) fn install(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment)?;
    let jobs_lock = super::acquire_jobs_lock(&paths)?;
    let database = Database::open(&paths)?;
    let selector = php_selector(&database)?;
    let progress = DownloadProgressRenderer::new(&streams.err);
    let installed = with_resource_http_client(environment, |client| {
        commands.install_composer_with_php_pair_and_progress(selector, client, &progress)
    })?;
    drop(progress);
    drop(jobs_lock);
    let php_pair = installed.php_pair();
    let composer = installed.composer();
    super::write_php_pair_install_lines(php_pair, streams)?;
    super::write_revoked_latest_warning(composer, &mut streams.err)?;
    streams
        .out
        .success(Line::field("Installed Composer track ", composer.track()))?;
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
        commands.update_composer_with_progress(client, &progress)
    })?;
    drop(progress);
    drop(jobs_lock);
    let output = &mut streams.out;

    super::write_revoked_latest_warnings(updated.installs(), &mut streams.err)?;
    super::write_updated(output, updated.installs().len(), "Composer track(s)")?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    args: ComposerUninstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths, environment)?;
    let options = ManagedResourceUninstallOptions::new()
        .prune(args.prune)
        .force(args.force);
    let removal = commands.uninstall_composer(options)?;
    let output = &mut streams.out;

    output.success(Line::field(
        "Queued removal for Composer track ",
        removal.track(),
    ))?;
    super::request_system_reconciliation(&paths, streams)?;

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
    let env = composer_env_overlay(&paths, environment)?;

    super::php::shim_with_args_and_env(shim_args, env, environment)
}

fn php_selector(database: &Database) -> Result<TrackSelector, ExecuteError> {
    if let Some(track) = database.global_php_default_track()? {
        return Ok(TrackSelector::Track(TrackName::new(track)?));
    }

    Ok(TrackSelector::Latest)
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

fn composer_env_overlay(
    paths: &PvPaths,
    environment: &impl Environment,
) -> Result<Vec<(OsString, OsString)>, ExecuteError> {
    Ok(vec![
        (
            OsString::from("COMPOSER_HOME"),
            paths.composer().as_std_path().as_os_str().to_os_string(),
        ),
        (
            OsString::from("COMPOSER_CACHE_DIR"),
            paths
                .composer()
                .join("cache")
                .as_std_path()
                .as_os_str()
                .to_os_string(),
        ),
        (
            OsString::from("PATH"),
            composer_path_overlay(paths, environment)?,
        ),
    ])
}

fn composer_path_overlay(
    paths: &PvPaths,
    environment: &impl Environment,
) -> Result<OsString, ExecuteError> {
    let pv_bin = paths.bin().as_std_path().to_path_buf();
    let composer_bin = paths
        .composer()
        .join("vendor/bin")
        .as_std_path()
        .to_path_buf();
    let mut entries = vec![pv_bin.clone(), composer_bin.clone()];

    if let Some(path) = environment.var_os("PATH") {
        entries.extend(
            std::env::split_paths(&path).filter(|entry| entry != &pv_bin && entry != &composer_bin),
        );
    }

    join_paths(entries)
}

fn join_paths(entries: Vec<PathBuf>) -> Result<OsString, ExecuteError> {
    std::env::join_paths(entries)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error).into())
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
    if let Some(target_platform) = environment.target_platform() {
        return Ok(target_platform);
    }

    Ok(TargetPlatform::current()?)
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
