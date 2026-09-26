use std::io;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use platform::CaFileState;
use resources::{
    ArtifactManifest, ArtifactManifestCache, ArtifactManifestSource, ResourceHttpClient,
    ResourceName, TargetPlatform, TrackName, TrackSelector, UreqResourceHttpClient,
};
use state::{Database, ManagedResourceDesiredState, PvPaths, StateError};

use super::{ca, daemon as daemon_command, dns, ports};
use crate::args::{SetupArgs, UninstallArgs};
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::helper_release::{HelperReleaseMetadata, metadata_path as helper_metadata_path};
use crate::output::{Line, Mark, Output, Streams};
use crate::progress::{DownloadProgressRenderer, step_spinner};
use crate::prompt;
use crate::shell::Shell;

const PV_ENV_START: &str = "# >>> PV ENV";
const PV_ENV_END: &str = "# <<< PV ENV";
#[cfg(unix)]
const SHIM_FILE_MODE: u32 = 0o700;

const DEFAULT_SETUP_RESOURCES: &[SetupResourceDefault] = &[
    SetupResourceDefault::concrete("caddy", "2"),
    SetupResourceDefault::manifest_default("frankenphp"),
    SetupResourceDefault::manifest_default("php"),
    SetupResourceDefault::manifest_default("mysql"),
    SetupResourceDefault::manifest_default("postgres"),
    SetupResourceDefault::manifest_default("redis"),
    SetupResourceDefault::manifest_default("mailpit"),
    SetupResourceDefault::manifest_default("rustfs"),
    SetupResourceDefault::concrete("composer", "2"),
];

#[derive(Clone, Copy, Debug)]
struct SetupResourceDefault {
    resource_name: &'static str,
    track: SetupResourceTrackDefault,
}

#[derive(Clone, Copy, Debug)]
enum SetupResourceTrackDefault {
    ManifestDefault,
    Concrete(&'static str),
}

#[derive(Clone, Debug)]
struct SetupResourcePlan {
    resource_name: ResourceName,
    track: TrackName,
}

#[derive(Clone, Debug, Default)]
struct SetupResourcePlans {
    plans: Vec<SetupResourcePlan>,
    failures: Vec<String>,
}

struct PrivilegedHelperCandidate {
    path: Utf8PathBuf,
    metadata: HelperReleaseMetadata,
}

impl SetupResourceDefault {
    const fn manifest_default(resource_name: &'static str) -> Self {
        Self {
            resource_name,
            track: SetupResourceTrackDefault::ManifestDefault,
        }
    }

    const fn concrete(resource_name: &'static str, track: &'static str) -> Self {
        Self {
            resource_name,
            track: SetupResourceTrackDefault::Concrete(track),
        }
    }
}

pub(crate) fn setup(
    args: SetupArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    state::fs::ensure_layout(&paths)?;

    streams
        .out
        .flow_start("setup", "PV setup", Some("macOS integration"))?;
    streams.out.flow_step(
        Mark::Done,
        Line::field("Ensured PV state layout: ", paths.root()),
    )?;
    install_command_shims(environment, &paths)?;
    let default_resource_plan = refresh_setup_artifact_manifest(environment, &paths, streams)?;

    let helper_candidate = privileged_helper_candidate(environment, &paths)?;
    let helper_installation_required =
        privileged_helper_installation_required(environment, &helper_candidate)?;
    if args.non_interactive && helper_installation_required {
        return Err(CliError::SetupHelperRequiresAuthentication.into());
    }
    if helper_installation_required
        && !args.yes
        && !prompt::confirm_or(
            environment,
            streams,
            CliError::HelperConfirmationRequired,
            "Install or replace the PV privileged helper?\nmacOS will request administrator authentication.",
            true,
        )?
    {
        return Ok(ExitCode::FAILURE);
    }

    let _helper_lifecycle_lock = state::HelperLifecycleLock::acquire(&paths)?;
    if ensure_privileged_helper(environment, &paths, helper_installation_required, streams)?
        != ExitCode::SUCCESS
    {
        return Ok(ExitCode::FAILURE);
    }

    if configure_shell_integration(&args, environment, &paths, streams)? != ExitCode::SUCCESS {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("DNS resolver setup", streams, |streams| {
        dns::install_config_only(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("port redirect setup", streams, |streams| {
        ports::install(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("CA trust setup", streams, |streams| {
        ca::trust(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    record_default_resource_desired_state(&paths, &default_resource_plan.plans)?;
    if !run_required_step("daemon registration", streams, |streams| {
        daemon_command::enable_without_reconciliation(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }

    let mut progress = DownloadProgressRenderer::with_output(&mut streams.err);
    let completed =
        ::daemon::run_job_with_events_blocking(paths, "reconcile", "system", &mut progress)?;
    drop(progress);
    let output = &mut streams.out;

    output.flow_step(
        Mark::Done,
        Line::field("System reconciliation completed: ", &completed.summary),
    )?;
    if !default_resource_plan.failures.is_empty() {
        output.flow_end(
            Mark::Failure,
            "Default Managed Resource planning failed for some defaults:",
        )?;
        for failure in &default_resource_plan.failures {
            output.detail(format!("- {failure}"))?;
        }
        output.follow_up("PV setup completed core integrations; rerun `pv setup` after fixing the default Managed Resource manifest.")?;

        return Ok(ExitCode::FAILURE);
    }
    output.flow_end(Mark::Done, "PV setup complete")?;

    Ok(ExitCode::SUCCESS)
}

fn refresh_setup_artifact_manifest(
    environment: &impl Environment,
    paths: &PvPaths,
    streams: &mut Streams<'_>,
) -> Result<SetupResourcePlans, ExecuteError> {
    let cache = ArtifactManifestCache::new(paths.downloads());
    let manifest_url = artifact_manifest_url(environment);

    let refresh =
        with_resource_http_client(environment, |client| cache.refresh(&manifest_url, client))?;

    if let ArtifactManifestSource::Cached { reason } = refresh.source() {
        streams.err.warning(&format!(
            "artifact manifest refresh failed ({reason}); using cached manifest at {}",
            cache.path()
        ))?;
    }

    resolve_default_resource_plan(refresh.manifest(), target_platform(environment)?)
}

fn resolve_default_resource_plan(
    manifest: &ArtifactManifest,
    target_platform: TargetPlatform,
) -> Result<SetupResourcePlans, ExecuteError> {
    let php_resource = ResourceName::new("php")?;
    let php_default_track = manifest
        .resolve_track(&php_resource, TrackSelector::Latest)
        .cloned();
    let mut plans = SetupResourcePlans::default();

    for resource_default in DEFAULT_SETUP_RESOURCES {
        let resource_name = match ResourceName::new(resource_default.resource_name) {
            Ok(resource_name) => resource_name,
            Err(error) => {
                plans.failures.push(error.to_string());
                continue;
            }
        };
        let track_selector = match setup_resource_track_selector(
            resource_default,
            &resource_name,
            &php_default_track,
        ) {
            Ok(track_selector) => track_selector,
            Err(error) => {
                plans.failures.push(error);
                continue;
            }
        };
        let track = match manifest.resolve_track(&resource_name, track_selector) {
            Ok(track) => track.clone(),
            Err(error) => {
                plans.failures.push(error.to_string());
                continue;
            }
        };

        if let Err(error) = manifest.select_latest(&resource_name, &track, target_platform) {
            plans.failures.push(error.to_string());
            continue;
        }

        plans.plans.push(SetupResourcePlan {
            resource_name,
            track,
        });
    }

    Ok(plans)
}

fn setup_resource_track_selector(
    resource_default: &SetupResourceDefault,
    resource_name: &ResourceName,
    php_default_track: &resources::Result<TrackName>,
) -> Result<TrackSelector, String> {
    if resource_name.as_str() == "frankenphp" {
        return match php_default_track {
            Ok(track) => Ok(TrackSelector::Track(track.clone())),
            Err(error) => Err(format!(
                "could not resolve frankenphp default track from PHP default: {error}"
            )),
        };
    }

    match resource_default.track {
        SetupResourceTrackDefault::ManifestDefault => Ok(TrackSelector::Latest),
        SetupResourceTrackDefault::Concrete(track) => TrackName::new(track)
            .map(TrackSelector::Track)
            .map_err(|error| error.to_string()),
    }
}

fn with_resource_http_client<T>(
    environment: &impl Environment,
    operation: impl FnOnce(&dyn ResourceHttpClient) -> resources::Result<T>,
) -> resources::Result<T> {
    if let Some(client) = environment.resource_http_client() {
        return operation(client);
    }

    let client = UreqResourceHttpClient::default();

    operation(&client)
}

fn target_platform(environment: &impl Environment) -> Result<TargetPlatform, ExecuteError> {
    if let Some(target_platform) = environment.target_platform() {
        return Ok(target_platform);
    }

    Ok(TargetPlatform::current()?)
}

fn install_command_shims(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<(), ExecuteError> {
    let pv_executable = current_executable(environment)?;

    for shim in [
        CommandShim {
            name: "php",
            command: "shim:php",
        },
        CommandShim {
            name: "composer",
            command: "shim:composer",
        },
    ] {
        let path = paths.bin().join(shim.name);
        let content = format!(
            "#!/bin/sh\nexec {} {} \"$@\"\n",
            shell_quote(pv_executable.as_str()),
            shim.command
        );

        write_executable_file(&path, &content)?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct CommandShim {
    name: &'static str,
    command: &'static str,
}

pub(crate) fn uninstall(
    args: UninstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;

    if args.prune
        && !args.force
        && !prompt::confirm_or(
            environment,
            streams,
            CliError::PruneRequiresTerminal,
            "Permanently remove all PV-owned state under ~/.pv?",
            false,
        )?
    {
        return Ok(ExitCode::FAILURE);
    }

    let subtitle = if args.prune {
        "--prune"
    } else {
        "macOS integration"
    };
    streams
        .out
        .flow_start("uninstall", "PV uninstall", Some(subtitle))?;

    let _helper_lifecycle_lock = state::HelperLifecycleLock::acquire(&paths)?;
    if !run_required_step("daemon removal", streams, |streams| {
        daemon_command::disable(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("DNS resolver removal", streams, |streams| {
        dns::uninstall(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("port redirect removal", streams, |streams| {
        ports::uninstall(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("CA trust removal", streams, |streams| {
        untrust_ca_for_uninstall(environment, streams)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    // sudo asks for the password in this terminal, so the step runs live
    // and is titled once it finishes, like the captured steps.
    super::write_administrator_step(&mut streams.err, "Removing privileged helper".to_string())?;
    environment.remove_privileged_helper()?;
    if streams.out.surface().decorated() {
        streams
            .out
            .flow_step(Mark::Done, "privileged helper removal")?;
    }
    streams.out.success("Privileged helper removed")?;
    if remove_shell_integration(environment, &paths, streams)? != ExitCode::SUCCESS {
        return Ok(ExitCode::FAILURE);
    }

    run_required_step("PV state removal", streams, |streams| {
        if args.prune {
            prune_state(&paths, streams)?;
        } else {
            remove_default_state(&paths, streams)?;
        }

        Ok(ExitCode::SUCCESS)
    })?;

    streams.out.flow_end(Mark::Done, "PV uninstall complete")?;

    Ok(ExitCode::SUCCESS)
}

/// Runs one required setup or uninstall step. A terminal stderr shows a
/// spinner while it runs. Decorated stdout then places its rows under a title
/// marked with the outcome and closes a failed flow with the stop line. Plain
/// stdout streams rows as they happen and writes the stop line for a failing
/// exit code. A step that needs sudo or a prompt must not run here.
fn run_required_step(
    label: &str,
    streams: &mut Streams<'_>,
    command: impl FnOnce(&mut Streams<'_>) -> Result<ExitCode, ExecuteError>,
) -> Result<bool, ExecuteError> {
    let spinner = step_spinner(&streams.err, label);
    if !streams.out.surface().decorated() {
        let result = command(streams);
        spinner.finish_and_clear();
        if result? == ExitCode::SUCCESS {
            return Ok(true);
        }
        streams.out.line(&format!("PV stopped during {label}."))?;

        return Ok(false);
    }

    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = streams.capture(&mut out, &mut err, command);
    spinner.finish_and_clear();
    let succeeded = matches!(result, Ok(exit_code) if exit_code == ExitCode::SUCCESS);
    let mark = if succeeded { Mark::Done } else { Mark::Failure };
    // The step's own error outranks a failure to replay its rows.
    let replayed = streams
        .out
        .flow_step(mark, label)
        .and_then(|()| streams.out.writer().write_all(&out))
        .and_then(|()| streams.err.writer().write_all(&err));
    if succeeded {
        replayed?;
        return Ok(true);
    }
    let stopped = streams
        .out
        .flow_end(Mark::Failure, format!("PV stopped during {label}."));
    result?;
    replayed?;
    stopped?;

    Ok(false)
}

fn privileged_helper_installation_required(
    environment: &impl Environment,
    candidate: &PrivilegedHelperCandidate,
) -> Result<bool, ExecuteError> {
    match environment.privileged_helper_status() {
        Ok(status) => Ok(status.version != candidate.metadata.version()
            || status.protocol_version != candidate.metadata.protocol_version()),
        Err(error) if helper_status_error_is_repairable(&error) => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn helper_status_error_is_repairable(error: &platform::PlatformError) -> bool {
    matches!(
        error,
        platform::PlatformError::PrivilegedHelperUnavailable
            | platform::PlatformError::PrivilegedHelperProtocolMismatch { .. }
            | platform::PlatformError::PrivilegedHelperRejected { .. }
            | platform::PlatformError::PrivilegedHelperRemote { .. }
            | platform::PlatformError::PrivilegedHelperIo(_)
            | platform::PlatformError::PrivilegedHelperProtocol(_)
            | platform::PlatformError::SystemIntegration(_)
    )
}

fn ensure_privileged_helper(
    environment: &impl Environment,
    paths: &PvPaths,
    installation_required: bool,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let candidate = privileged_helper_candidate(environment, paths)?;
    let current_installation_required =
        privileged_helper_installation_required(environment, &candidate)?;
    if !current_installation_required {
        let status = environment.privileged_helper_status()?;
        streams.out.flow_step(
            Mark::Done,
            format!(
                "Privileged helper: current {} (protocol {})",
                status.version, status.protocol_version
            ),
        )?;

        return Ok(ExitCode::SUCCESS);
    }
    if !installation_required {
        return Err(platform::PlatformError::PrivilegedHelperInstallation(
            "privileged-helper state changed during setup; rerun `pv setup`".to_string(),
        )
        .into());
    }
    super::write_administrator_step(
        &mut streams.err,
        format!(
            "Installing privileged helper {} (protocol {})",
            candidate.metadata.version(),
            candidate.metadata.protocol_version()
        ),
    )?;
    let prepared_directory = paths.config().join("helper");
    let install_outcome = environment.install_privileged_helper(
        &candidate.path,
        &prepared_directory,
        candidate.metadata.sha256(),
        candidate.metadata.version(),
        candidate.metadata.protocol_version(),
    )?;
    let status = install_outcome.status();
    streams.out.flow_step(
        Mark::Done,
        format!(
            "Installed privileged helper {} (protocol {})",
            status.version, status.protocol_version,
        ),
    )?;
    if let Some(warning) = install_outcome.cleanup_warning() {
        streams.err.warning(warning)?;
    }

    Ok(ExitCode::SUCCESS)
}

fn privileged_helper_candidate(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<PrivilegedHelperCandidate, ExecuteError> {
    let layout = state::AppReleaseLayout::new(paths.clone());
    let path = if let Some(active_version) = layout.active_release()? {
        paths.app_release_helper(&active_version)
    } else {
        current_executable(environment)?.with_file_name("pv-helper")
    };
    let metadata_path = helper_metadata_path(&path);
    if !state::fs::path_entry_exists(&metadata_path)? {
        return Err(CliError::InvalidPrivilegedHelperReleaseMetadata {
            path: metadata_path.to_string(),
            reason: "release metadata is missing; reinstall PV before running `pv setup`"
                .to_string(),
        }
        .into());
    }
    let metadata = HelperReleaseMetadata::read(&path)?;

    Ok(PrivilegedHelperCandidate { path, metadata })
}

fn record_default_resource_desired_state(
    paths: &PvPaths,
    default_resource_plan: &[SetupResourcePlan],
) -> Result<(), ExecuteError> {
    let mut database = Database::open(paths)?;

    for planned in default_resource_plan {
        database.record_managed_resource_track_desired(
            planned.resource_name.as_str(),
            planned.track.as_str(),
            ManagedResourceDesiredState::Installed,
        )?;
    }

    Ok(())
}

fn configure_shell_integration(
    args: &SetupArgs,
    environment: &impl Environment,
    paths: &PvPaths,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let output = &mut streams.out;

    if args.no_path {
        output.flow_step(
            Mark::Idle,
            "Shell profile integration skipped by --no-path.",
        )?;
        write_manual_shell_integration(output, None)?;

        return Ok(ExitCode::SUCCESS);
    }

    let Some(shell_path) = environment.var_os("SHELL") else {
        output.flow_step(
            Mark::Idle,
            "Shell profile integration skipped because $SHELL is not set.",
        )?;
        write_manual_shell_integration(output, None)?;

        return Ok(ExitCode::SUCCESS);
    };
    let Some(shell) = Shell::detect(shell_path.as_os_str()) else {
        output.flow_step(
            Mark::Idle,
            format!(
                "Shell profile integration skipped for unsupported shell: {}",
                shell_path.to_string_lossy()
            ),
        )?;
        write_manual_shell_integration(output, None)?;

        return Ok(ExitCode::SUCCESS);
    };

    let profile_path = shell_profile_path(paths.home(), shell);
    let block = shell_profile_block(shell);
    let existing = read_user_file(&profile_path)?;
    let (next_content, action) = match existing.as_deref() {
        Some(content) => {
            let transform = remove_pv_env_block(content);
            if !transform.complete {
                output.flow_step(
                    Mark::Failure,
                    format!(
                        "Shell profile has an incomplete PV ENV block; leaving it unchanged: {profile_path}"
                    ),
                )?;

                return Ok(ExitCode::FAILURE);
            }

            let next = if transform.found {
                append_shell_block(&transform.content, &block)
            } else {
                append_shell_block(content, &block)
            };
            if next == content {
                output.flow_step(
                    Mark::Done,
                    Line::field("Shell profile integration already current: ", &profile_path),
                )?;

                return Ok(ExitCode::SUCCESS);
            }

            let action = if transform.found { "repair" } else { "update" };
            (next, action)
        }
        None => (block, "create"),
    };

    if args.non_interactive {
        return Err(CliError::ShellProfileChangeNonInteractive {
            action,
            path: profile_path.to_string(),
        }
        .into());
    }

    if !args.yes
        && !prompt::confirm_or(
            environment,
            streams,
            CliError::ShellProfileConfirmationRequired {
                action,
                path: profile_path.to_string(),
            },
            &format!("Update shell profile for PV ENV integration ({action})?\n{profile_path}"),
            true,
        )?
    {
        return Ok(ExitCode::FAILURE);
    }

    if existing.is_some() {
        let backup_path = backup_user_file(&profile_path)?;
        streams.out.flow_step(
            Mark::Done,
            Line::field("Backed up shell profile: ", backup_path),
        )?;
    }
    write_user_file(&profile_path, &next_content)?;

    let output = &mut streams.out;
    output.flow_step(
        Mark::Done,
        Line::field("Updated shell profile integration: ", &profile_path),
    )?;
    write_manual_shell_integration(output, Some(shell))?;

    Ok(ExitCode::SUCCESS)
}

fn remove_shell_integration(
    environment: &impl Environment,
    paths: &PvPaths,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let output = &mut streams.out;
    let Some(shell_path) = environment.var_os("SHELL") else {
        output.flow_step(
            Mark::Idle,
            "Shell profile integration not inspected because $SHELL is not set.",
        )?;

        return Ok(ExitCode::SUCCESS);
    };
    let Some(shell) = Shell::detect(shell_path.as_os_str()) else {
        output.flow_step(
            Mark::Idle,
            format!(
                "Shell profile integration not inspected for unsupported shell: {}",
                shell_path.to_string_lossy()
            ),
        )?;

        return Ok(ExitCode::SUCCESS);
    };

    let profile_path = shell_profile_path(paths.home(), shell);
    let Some(content) = read_user_file(&profile_path)? else {
        output.flow_step(
            Mark::Idle,
            Line::field("Shell profile already absent: ", &profile_path),
        )?;

        return Ok(ExitCode::SUCCESS);
    };
    let transform = remove_pv_env_block(&content);

    if !transform.complete {
        output.flow_step(
            Mark::Failure,
            format!(
                "Shell profile has an incomplete PV ENV block; leaving it unchanged: {profile_path}"
            ),
        )?;

        return Ok(ExitCode::FAILURE);
    }
    if !transform.found {
        output.flow_step(
            Mark::Idle,
            Line::field("Shell profile has no PV ENV block: ", &profile_path),
        )?;

        return Ok(ExitCode::SUCCESS);
    }

    let backup_path = backup_user_file(&profile_path)?;
    write_user_file(&profile_path, &transform.content)?;
    output.flow_step(
        Mark::Done,
        Line::field("Backed up shell profile: ", backup_path),
    )?;
    output.flow_step(
        Mark::Done,
        Line::field("Removed shell profile integration: ", &profile_path),
    )?;

    Ok(ExitCode::SUCCESS)
}

fn untrust_ca_for_uninstall(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());

    if matches!(local_state, CaFileState::Missing { .. }) {
        let fingerprints = trusted_pv_ca_fingerprints(environment)?;
        let output = &mut streams.out;
        if fingerprints.is_empty() {
            output
                .note("PV local CA files are absent; System keychain trust is already absent.")?;

            return Ok(ExitCode::SUCCESS);
        }

        for fingerprint in fingerprints {
            environment.untrust_system_ca(&fingerprint)?;
            output.success(Line::field(
                "Removed stale PV local CA trust from the System keychain: ",
                &fingerprint,
            ))?;
        }

        return Ok(ExitCode::SUCCESS);
    }

    ca::untrust(environment, streams)
}

fn remove_default_state(paths: &PvPaths, streams: &mut Streams<'_>) -> Result<(), ExecuteError> {
    state::fs::remove_daemon_socket(paths)?;

    let output = &mut streams.out;
    for (label, path) in [
        ("PV app binaries and shims", paths.bin()),
        ("runtime metadata", paths.run()),
        ("generated configs", paths.config()),
        ("download cache", paths.downloads()),
    ] {
        if delete_optional_dir(path)? {
            output.success(Line::field(&format!("Removed {label}: "), path))?;
        } else {
            output.note(Line::field(&format!("{label} already absent: "), path))?;
        }
    }
    output.note("Preserved logs, pv.db, certificates, Composer home/cache, and resources data.")?;

    Ok(())
}

fn trusted_pv_ca_fingerprints(environment: &impl Environment) -> Result<Vec<String>, ExecuteError> {
    struct EnvironmentTrustInspector<'environment, E> {
        environment: &'environment E,
    }

    impl<E: Environment> platform::SystemTrustInspector for EnvironmentTrustInspector<'_, E> {
        fn trusted_certificates(
            &self,
        ) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
            self.environment.trusted_ca_certificates()
        }
    }

    Ok(platform::trusted_pv_ca_fingerprints(
        &EnvironmentTrustInspector { environment },
    )?)
}

fn prune_state(paths: &PvPaths, streams: &mut Streams<'_>) -> Result<(), ExecuteError> {
    state::fs::remove_daemon_socket(paths)?;
    let output = &mut streams.out;

    if delete_optional_dir(paths.root())? {
        output.success(Line::field("Removed PV state: ", paths.root()))?;
    } else {
        output.note(Line::field("PV state already absent: ", paths.root()))?;
    }

    Ok(())
}

fn delete_optional_dir(path: &Utf8Path) -> Result<bool, ExecuteError> {
    match state::fs::delete_dir_all(path) {
        Ok(()) => Ok(true),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn shell_profile_path(home: &Utf8Path, shell: Shell) -> Utf8PathBuf {
    match shell {
        Shell::Bash => home.join(".bash_profile"),
        Shell::Fish => home.join(".config/fish/config.fish"),
        Shell::Zsh => home.join(".zprofile"),
    }
}

fn shell_profile_block(shell: Shell) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!(
            r#"{PV_ENV_START}
if [ -x "$HOME/.pv/bin/pv" ]; then
  eval "$("$HOME/.pv/bin/pv" env --shell {shell_name})"
fi
{PV_ENV_END}
"#,
            shell_name = shell_name(shell),
        ),
        Shell::Fish => format!(
            r#"{PV_ENV_START}
if test -x "$HOME/.pv/bin/pv"
  eval ("$HOME/.pv/bin/pv" env --shell {shell_name} | string collect)
end
{PV_ENV_END}
"#,
            shell_name = shell_name(shell),
        ),
    }
}

fn write_manual_shell_integration(output: &mut Output<'_>, shell: Option<Shell>) -> io::Result<()> {
    match shell {
        Some(shell) => output.follow_up(format!(
            "Open a new terminal, or run `pv env --shell {}` for current-session shell integration.",
            shell_name(shell)
        )),
        None => output.follow_up(
            "Run `pv env --shell zsh`, `pv env --shell bash`, or `pv env --shell fish` for manual shell integration.",
        ),
    }
}

#[derive(Debug)]
struct BlockTransform {
    content: String,
    found: bool,
    complete: bool,
}

fn remove_pv_env_block(content: &str) -> BlockTransform {
    let mut next = String::new();
    let mut found = false;
    let mut in_block = false;
    let mut complete = true;

    for line in content.lines() {
        match (line.trim(), in_block) {
            (PV_ENV_START, false) => {
                found = true;
                in_block = true;
            }
            (PV_ENV_START, true) => {
                complete = false;
            }
            (PV_ENV_END, true) => {
                in_block = false;
            }
            (_line, true) => {}
            (_line, false) => {
                next.push_str(line);
                next.push('\n');
            }
        }
    }

    if in_block {
        complete = false;
    }

    BlockTransform {
        content: next,
        found,
        complete,
    }
}

fn append_shell_block(content: &str, block: &str) -> String {
    if content.trim().is_empty() {
        return block.to_string();
    }

    let mut next = content.trim_end_matches('\n').to_string();
    next.push_str("\n\n");
    next.push_str(block);

    next
}

fn shell_name(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "bash",
        Shell::Fish => "fish",
        Shell::Zsh => "zsh",
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn current_executable(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.current_exe()?)
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn backup_user_file(path: &Utf8Path) -> Result<Utf8PathBuf, ExecuteError> {
    let backup_path = backup_path(path);

    copy_user_file(path, &backup_path)?;

    Ok(backup_path)
}

fn backup_path(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("profile");
    let timestamp = timestamp_suffix();

    path.with_file_name(format!("{file_name}.{timestamp}.pv.bak"))
}

fn timestamp_suffix() -> String {
    let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_error) => 0,
    };

    timestamp.to_string()
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI shell integration helper owns user profile reads"
)]
fn read_user_file(path: &Utf8Path) -> Result<Option<String>, ExecuteError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(path_io_error(path, error).into()),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI shell integration helper owns user profile writes"
)]
fn write_user_file(path: &Utf8Path, content: &str) -> Result<(), ExecuteError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| path_io_error(parent, source))?;
    }
    std::fs::write(path, content).map_err(|source| path_io_error(path, source).into())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI setup helper owns PV command shim writes"
)]
fn write_executable_file(path: &Utf8Path, content: &str) -> Result<(), ExecuteError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| path_io_error(parent, source))?;
    }

    std::fs::write(path, content).map_err(|source| path_io_error(path, source))?;
    set_command_shim_permissions(path)
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "CLI setup helper owns PV command shim permission updates"
)]
fn set_command_shim_permissions(path: &Utf8Path) -> Result<(), ExecuteError> {
    use std::os::unix::fs::PermissionsExt as _;

    let permissions = std::fs::Permissions::from_mode(SHIM_FILE_MODE);
    std::fs::set_permissions(path, permissions).map_err(|source| path_io_error(path, source).into())
}

#[cfg(not(unix))]
fn set_command_shim_permissions(path: &Utf8Path) -> Result<(), ExecuteError> {
    Err(path_io_error(
        path,
        io::Error::new(
            io::ErrorKind::Unsupported,
            "PV command shims require Unix permissions",
        ),
    )
    .into())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI shell integration helper owns user profile backups"
)]
fn copy_user_file(source: &Utf8Path, destination: &Utf8Path) -> Result<(), ExecuteError> {
    std::fs::copy(source, destination)
        .map(|_bytes| ())
        .map_err(|error| path_io_error(destination, error).into())
}

fn path_io_error(path: &Utf8Path, source: io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{path}: {source}"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}
