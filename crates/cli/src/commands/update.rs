use std::io;
use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};
use platform::{LaunchAgentConfig, LaunchAgentFileState};
use protocol::{
    ManagedResourceUpdateCheckTrack, ManagedResourceUpdateStatus as ResourceUpdateStatus,
};
use resources::{ResourceHttpClient, UreqResourceHttpClient};
use self_update::{
    AppUpdateAsset, AppUpdateManifest, AppUpdatePlatform, AppUpdateVersion,
    PrivilegedHelperUpdateAsset,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use state::{PvPaths, StateError};

use crate::args::UpdateArgs;
use crate::environment::{Environment, app_update_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::helper_release::{HelperReleaseMetadata, metadata_path as helper_metadata_path};
use crate::output::{Output, OutputMode};
use crate::progress::DownloadProgressRenderer;

static APP_DOWNLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);
const MANAGED_RESOURCE_UPDATE_CONTINUATION: &str = "internal:update-managed-resources";

#[expect(
    clippy::disallowed_types,
    reason = "PV app update downloader owns the command-scoped temporary file handle"
)]
type AppDownloadFile = std::fs::File;

pub(crate) fn run(
    args: UpdateArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    if !args.check {
        return run_update(environment, stdout, stderr);
    }

    let paths = pv_paths(environment)?;
    daemon::health_blocking(paths.clone()).map_err(update_check_daemon_error)?;
    let app = app_update_status(environment)?;
    let managed_resources = daemon::managed_resource_update_check_blocking(paths)
        .map_err(update_check_daemon_error)?
        .managed_resources;
    let check = UpdateCheckOutput {
        app,
        managed_resources,
    };

    if args.json {
        serde_json::to_writer(&mut *stdout, &check)?;
        writeln!(stdout)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut output = Output::new(stdout, OutputMode::plain());
    check.write_plain(&mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn run_managed_resource_continuation(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let layout = state::AppReleaseLayout::new(paths.clone());
    let current_version = AppUpdateVersion::current()?;
    validate_active_release(&layout, &current_version)?;

    run_managed_resource_update_phase(paths, environment, stdout)
}

fn run_update(
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let outcome = run_app_update_phase(environment, stdout, stderr)?;

    match outcome {
        AppUpdateOutcome::Current { paths } => {
            run_managed_resource_update_phase(paths, environment, stdout)
        }
        AppUpdateOutcome::Updated { paths } => {
            let active_pv_binary = paths.active_pv_binary();

            reexec_managed_resource_update(environment, &active_pv_binary)
        }
    }
}

fn run_managed_resource_update_phase(
    paths: PvPaths,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let mut progress = DownloadProgressRenderer::new(environment.stdout_is_terminal());
    let job = daemon::run_job_with_events_blocking(paths, "update", "system", &mut progress)
        .map_err(managed_resource_update_daemon_error)?;
    drop(progress);
    let mut output = Output::new(stdout, OutputMode::plain());
    write_managed_resource_update_summary(&mut output, &job.summary)?;

    Ok(ExitCode::SUCCESS)
}

fn reexec_managed_resource_update(
    environment: &impl Environment,
    active_pv_binary: &Utf8Path,
) -> Result<ExitCode, ExecuteError> {
    let args = vec![MANAGED_RESOURCE_UPDATE_CONTINUATION.to_string()];
    environment
        .exec(active_pv_binary.as_std_path(), &args)
        .map_err(|error| CliError::ManagedResourceUpdateContinuationFailed {
            message: error.to_string(),
        })
        .map_err(ExecuteError::from)
}

fn write_managed_resource_update_summary(
    output: &mut Output<'_, impl Write>,
    summary: &str,
) -> Result<(), ExecuteError> {
    if let Some((updated, reconciled)) = summary.split_once("; reconciled: ") {
        output.line(&format!("Managed Resources: {updated}"))?;
        output.line(&format!("Managed Resources reconciled: {reconciled}"))?;

        return Ok(());
    }

    output.line(&format!("Managed Resources: {summary}"))?;

    Ok(())
}

fn validate_active_release(
    layout: &state::AppReleaseLayout,
    current_version: &AppUpdateVersion,
) -> Result<String, ExecuteError> {
    let active_version =
        layout
            .active_release()
            .map_err(|error| CliError::AppUpdateInvalidActiveRelease {
                message: error.to_string(),
            })?;
    let Some(active_version) = active_version else {
        return Err(CliError::AppUpdateInvalidActiveRelease {
            message: "active PV binary symlink is missing".to_string(),
        }
        .into());
    };

    if active_version != current_version.as_str() {
        return Err(CliError::AppUpdateActiveReleaseMismatch {
            active_version,
            current_version: current_version.to_string(),
        }
        .into());
    }

    Ok(active_version)
}

fn normalize_launch_agent(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<Utf8PathBuf, ExecuteError> {
    let expected = launch_agent_config(paths);
    let path = launch_agent_path(environment)?;
    match platform::inspect_launch_agent_file(&path, Some(&expected)) {
        LaunchAgentFileState::Current { path, .. } => Ok(path),
        LaunchAgentFileState::Stale { .. } => {
            platform::write_launch_agent_file(&path, &expected)?;

            Ok(path)
        }
        LaunchAgentFileState::Missing { path } => Err(CliError::AppUpdateLaunchAgentMissing {
            path: path.to_string(),
        }
        .into()),
        LaunchAgentFileState::Conflict { path } => Err(CliError::AppUpdateLaunchAgentConflict {
            path: path.to_string(),
        }
        .into()),
        LaunchAgentFileState::Unreadable { message, .. } => {
            Err(CliError::AppUpdateLaunchAgentUnreadable { message }.into())
        }
    }
}

fn launch_agent_config(paths: &PvPaths) -> LaunchAgentConfig {
    LaunchAgentConfig::new(
        paths.active_pv_binary(),
        paths.launchd_stdout_log(),
        paths.launchd_stderr_log(),
    )
}

fn launch_agent_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    utf8_path(environment.launch_agent_path())
}

fn fetch_app_update_manifest(
    environment: &impl Environment,
) -> Result<AppUpdateManifest, ExecuteError> {
    let url = app_update_manifest_url(environment);
    let json = with_resource_http_client(environment, |client| client.get_text(&url))?;

    Ok(AppUpdateManifest::parse(&json)?)
}

fn download_app_asset(
    environment: &impl Environment,
    paths: &PvPaths,
    version: &str,
    asset: &AppUpdateAsset,
    progress: &DownloadProgressRenderer,
    stderr: &mut impl Write,
) -> Result<Utf8PathBuf, ExecuteError> {
    download_verified_asset(
        environment,
        paths,
        &format!("app-{version}"),
        version,
        asset.url(),
        asset.sha256().as_str(),
        asset.size(),
        progress,
        stderr,
    )
}

fn download_helper_asset(
    environment: &impl Environment,
    paths: &PvPaths,
    asset: &PrivilegedHelperUpdateAsset,
    progress: &DownloadProgressRenderer,
    stderr: &mut impl Write,
) -> Result<Utf8PathBuf, ExecuteError> {
    download_verified_asset(
        environment,
        paths,
        &format!("helper-{}", asset.version()),
        &format!("helper {}", asset.version()),
        asset.url(),
        asset.sha256().as_str(),
        asset.size(),
        progress,
        stderr,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "download verification keeps the selected immutable asset fields explicit"
)]
fn download_verified_asset(
    environment: &impl Environment,
    paths: &PvPaths,
    temporary_name: &str,
    progress_label: &str,
    url: &str,
    expected_sha256: &str,
    expected_size: u64,
    progress: &DownloadProgressRenderer,
    stderr: &mut impl Write,
) -> Result<Utf8PathBuf, ExecuteError> {
    state::fs::ensure_user_dir(paths.downloads())?;
    let path = temporary_app_download_path(paths, temporary_name);
    let file = create_download_file(&path)?;
    let mut writer = AppDownloadProgressWriter::new(
        CountingSha256Writer::new(file),
        progress,
        progress_label,
        expected_size,
    );
    let download_result =
        with_resource_http_client(environment, |client| client.download(url, &mut writer));
    if let Err(error) = download_result {
        write_download_cleanup_warning(stderr, remove_download(&path).err())?;
        return Err(error);
    }

    let stats = writer.finish();
    progress.update_app_progress(progress_label, stats.size, expected_size);
    if stats.size != expected_size {
        write_download_cleanup_warning(stderr, remove_download(&path).err())?;
        return Err(CliError::AppUpdateSizeMismatch {
            url: url.to_string(),
            expected: expected_size,
            actual: stats.size,
        }
        .into());
    }
    if stats.sha256 != expected_sha256 {
        write_download_cleanup_warning(stderr, remove_download(&path).err())?;
        return Err(CliError::AppUpdateChecksumMismatch {
            url: url.to_string(),
            expected: expected_sha256.to_string(),
            actual: stats.sha256,
        }
        .into());
    }

    Ok(path)
}

fn temporary_app_download_path(paths: &PvPaths, name: &str) -> Utf8PathBuf {
    let process_id = std::process::id();
    let counter = APP_DOWNLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);

    paths
        .downloads()
        .join(format!("pv-{name}-{process_id}-{counter}.tmp"))
}

fn remove_download(path: &Utf8Path) -> Result<(), ExecuteError> {
    state::fs::remove_file_if_exists(path)?;

    Ok(())
}

fn restart_daemon_without_reconciliation(
    environment: &impl Environment,
    paths: &PvPaths,
    launch_agent_path: &Utf8Path,
    health_check: DaemonHealthCheck,
) -> Result<(), ExecuteError> {
    bootout_launch_agent_if_loaded(environment)?;
    environment.bootstrap_launch_agent(launch_agent_path)?;
    clear_daemon_startup_failure_marker(paths)?;
    environment.kickstart_launch_agent()?;
    wait_until_daemon_started(paths.clone(), health_check)?;

    Ok(())
}

fn create_download_file(path: &Utf8Path) -> Result<AppDownloadFile, StateError> {
    state::fs::create_new_file(path)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DaemonHealthCheck {
    RequireCompatibleProtocol,
    AcceptProtocolMismatch,
}

fn wait_until_daemon_started(
    paths: PvPaths,
    health_check: DaemonHealthCheck,
) -> Result<(), ExecuteError> {
    match health_check {
        DaemonHealthCheck::RequireCompatibleProtocol => daemon::wait_until_healthy_blocking(paths)?,
        DaemonHealthCheck::AcceptProtocolMismatch => {
            daemon::wait_until_healthy_allowing_protocol_mismatch_blocking(paths)?;
        }
    }

    Ok(())
}

fn clear_daemon_startup_failure_marker(paths: &PvPaths) -> Result<(), ExecuteError> {
    state::fs::remove_file_if_exists(&paths.daemon_startup_error())?;

    Ok(())
}

fn bootout_launch_agent_if_loaded(environment: &impl Environment) -> Result<(), ExecuteError> {
    match environment.bootout_launch_agent() {
        Ok(()) => Ok(()),
        Err(error) if launch_agent_is_already_unloaded(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn launch_agent_is_already_unloaded(error: &platform::PlatformError) -> bool {
    match error {
        platform::PlatformError::LaunchAgent(message) => {
            let message = message.to_ascii_lowercase();
            message.contains("already unloaded")
                || message.contains("not loaded")
                || message.contains("not running")
                || message.contains("no such process")
        }
        platform::PlatformError::LaunchAgentCommandStatus { .. } => false,
        _ => false,
    }
}

fn update_state_error(error: StateError) -> ExecuteError {
    match error {
        StateError::CoordinationLockHeld { path } => CliError::CoordinationLockHeld {
            path: path.to_string(),
        }
        .into(),
        error => error.into(),
    }
}

struct DownloadStats {
    sha256: String,
    size: u64,
}

struct CountingSha256Writer {
    inner: AppDownloadFile,
    hasher: Sha256,
    size: u64,
}

impl CountingSha256Writer {
    fn new(inner: AppDownloadFile) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            size: 0,
        }
    }

    fn finish(self) -> DownloadStats {
        DownloadStats {
            sha256: sha256_digest_hex(self.hasher.finalize()),
            size: self.size,
        }
    }
}

impl Write for CountingSha256Writer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        let written_size =
            u64::try_from(written).map_err(|_| io::Error::other("download size overflow"))?;
        self.size = self
            .size
            .checked_add(written_size)
            .ok_or_else(|| io::Error::other("download size overflow"))?;
        self.hasher.update(&buffer[..written]);

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct AppDownloadProgressWriter<'progress> {
    inner: CountingSha256Writer,
    progress: &'progress DownloadProgressRenderer,
    version: &'progress str,
    total_size: u64,
}

impl<'progress> AppDownloadProgressWriter<'progress> {
    fn new(
        inner: CountingSha256Writer,
        progress: &'progress DownloadProgressRenderer,
        version: &'progress str,
        total_size: u64,
    ) -> Self {
        progress.update_app_progress(version, 0, total_size);

        Self {
            inner,
            progress,
            version,
            total_size,
        }
    }

    fn finish(self) -> DownloadStats {
        self.inner.finish()
    }
}

impl Write for AppDownloadProgressWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.progress
            .update_app_progress(self.version, self.inner.size, self.total_size);

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn sha256_digest_hex(digest: impl IntoIterator<Item = u8>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);

    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }

    hex
}

enum AppUpdateOutcome {
    Current { paths: PvPaths },
    Updated { paths: PvPaths },
}

fn run_app_update_phase(
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<AppUpdateOutcome, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("PV update")?;

    let _update_lock = state::UpdateLock::acquire(&paths).map_err(update_state_error)?;
    state::fs::ensure_layout(&paths)?;

    let layout = state::AppReleaseLayout::new(paths.clone());
    let current_version = AppUpdateVersion::current()?;
    let previous_version = validate_active_release(&layout, &current_version)?;
    let launch_agent_path = normalize_launch_agent(environment, &paths)?;

    let manifest = fetch_app_update_manifest(environment)?;
    let platform = environment
        .app_update_platform()
        .map(Ok)
        .unwrap_or_else(AppUpdatePlatform::current)?;
    let asset = manifest.select_platform(platform)?;
    let _helper_lifecycle_lock = state::HelperLifecycleLock::acquire(&paths)?;
    let installed_helper = installed_helper_state(environment.privileged_helper_status())?;
    let plan = app_update_plan(
        &current_version,
        manifest.version(),
        &installed_helper,
        asset.helper(),
    )?;
    let app_update_required = plan.app;
    let helper_update_required = matches!(plan.helper, HelperUpdatePlan::Update);

    if !app_update_required && !helper_update_required {
        output.line(&format!("PV application: current {current_version}"))?;
        match &installed_helper {
            InstalledHelperState::Ready(status) if manifest.version() < &current_version => {
                output.line(&format!(
                    "Privileged helper: retained {} (protocol {}); app manifest {} is older than current PV {}",
                    status.version,
                    status.protocol_version,
                    manifest.version(),
                    current_version
                ))?
            }
            InstalledHelperState::Ready(status) => output.line(&format!(
                "Privileged helper: current {} (protocol {})",
                status.version, status.protocol_version
            ))?,
            InstalledHelperState::Missing => output.line(
                "Privileged helper: unavailable; the older app manifest has no applicable repair",
            )?,
            InstalledHelperState::ProtocolMismatch => output.line(
                "Privileged helper: protocol mismatch; the older app manifest has no applicable repair",
            )?,
        }

        return Ok(AppUpdateOutcome::Current { paths });
    }

    let helper_rollback = if helper_update_required {
        Some(prepare_helper_rollback(
            &paths,
            &current_version,
            &installed_helper,
        )?)
    } else {
        None
    };
    let progress = DownloadProgressRenderer::new(environment.stdout_is_terminal());
    if app_update_required {
        let downloaded = download_app_asset(
            environment,
            &paths,
            manifest.version().as_str(),
            asset,
            &progress,
            stderr,
        )?;
        let install_result =
            layout.install_release_binary(manifest.version().as_str(), &downloaded);
        let cleanup_result = remove_download(&downloaded);
        match (install_result, cleanup_result) {
            (Ok(_install), Ok(())) => {}
            (Ok(_install), Err(cleanup_error)) => return Err(cleanup_error),
            (Err(install_error), cleanup_result) => {
                write_download_cleanup_warning(stderr, cleanup_result.err())?;
                return Err(install_error.into());
            }
        }
    }

    let target_release_version = if app_update_required {
        manifest.version().as_str()
    } else {
        current_version.as_str()
    };
    let helper_metadata = if helper_update_required {
        Some(HelperReleaseMetadata::new(
            asset.helper().version().as_str(),
            asset.helper().protocol_version(),
            asset.helper().sha256().as_str(),
        )?)
    } else {
        None
    };
    let (helper_candidate, helper_only_download) = if helper_update_required {
        let downloaded_helper =
            download_helper_asset(environment, &paths, asset.helper(), &progress, stderr)?;
        if app_update_required {
            let install_result = layout
                .install_release_helper(target_release_version, &downloaded_helper)
                .map_err(ExecuteError::from)
                .and_then(|helper_candidate| {
                    if let Some(metadata) = &helper_metadata {
                        metadata.write(&helper_candidate)?;
                    }

                    Ok(helper_candidate)
                });
            let cleanup_result = remove_download(&downloaded_helper);
            match (install_result, cleanup_result) {
                (Ok(helper_candidate), Ok(())) => (helper_candidate, None),
                (Ok(_helper_candidate), Err(cleanup_error)) => return Err(cleanup_error),
                (Err(install_error), cleanup_result) => {
                    write_download_cleanup_warning(stderr, cleanup_result.err())?;
                    write_cleanup_warning(
                        stderr,
                        layout.remove_release(target_release_version).err(),
                    )?;
                    return Err(install_error);
                }
            }
        } else {
            (downloaded_helper.clone(), Some(downloaded_helper))
        }
    } else {
        let current_helper = paths.app_release_helper(current_version.as_str());
        let install_result = HelperReleaseMetadata::read(&current_helper).and_then(|metadata| {
            let helper_candidate = layout
                .install_release_helper(target_release_version, &current_helper)
                .map_err(ExecuteError::from)?;
            metadata.write(&helper_candidate)?;

            Ok(helper_candidate)
        });
        match install_result {
            Ok(helper_candidate) => (helper_candidate, None),
            Err(error) => {
                write_cleanup_warning(stderr, layout.remove_release(target_release_version).err())?;
                return Err(error);
            }
        }
    };
    drop(progress);

    if let Some(helper_rollback) = &helper_rollback
        && let Err(error) = materialize_helper_rollback(helper_rollback)
    {
        let download_cleanup_error = helper_only_download
            .as_ref()
            .and_then(|download| remove_download(download).err());
        let helper_cleanup_error = cleanup_helper_rollback(Some(helper_rollback)).err();
        let failed_release_cleanup_error = if app_update_required {
            layout.remove_release(target_release_version).err()
        } else {
            None
        };
        write_download_cleanup_warning(stderr, download_cleanup_error)?;
        write_helper_rollback_cleanup_warning(stderr, helper_cleanup_error)?;
        write_cleanup_warning(stderr, failed_release_cleanup_error)?;

        return Err(error.into());
    }

    let mut helper_install_cleanup_warning = None;
    if helper_update_required {
        let prepared_directory = paths.config().join("helper");
        let install_result = environment.install_privileged_helper(
            &helper_candidate,
            &prepared_directory,
            asset.helper().sha256().as_str(),
            asset.helper().version().as_str(),
            asset.helper().protocol_version(),
        );
        let install_outcome = match install_result {
            Ok(install_outcome) => install_outcome,
            Err(error) => {
                let original = error.to_string();
                let download_cleanup_error = helper_only_download
                    .as_ref()
                    .and_then(|download| remove_download(download).err());
                let failed_release_cleanup_error = if app_update_required {
                    layout.remove_release(target_release_version).err()
                } else {
                    None
                };
                let (helper_restore_error, helper_restore_cleanup_warning) = match helper_rollback
                    .as_ref()
                    .map(|helper_rollback| restore_helper(environment, helper_rollback))
                {
                    Some(Ok(cleanup_warning)) => (None, cleanup_warning),
                    Some(Err(error)) => (Some(error), None),
                    None => (None, None),
                };
                let helper_cleanup_error = if helper_restore_error.is_none() {
                    cleanup_helper_rollback(helper_rollback.as_ref()).err()
                } else {
                    None
                };
                let warning_result = (|| {
                    write_download_cleanup_warning(stderr, download_cleanup_error)?;
                    write_helper_rollback_cleanup_warning(stderr, helper_cleanup_error)?;
                    write_privileged_helper_cleanup_warning(
                        stderr,
                        helper_restore_cleanup_warning.as_deref(),
                    )?;
                    write_cleanup_warning(stderr, failed_release_cleanup_error)?;

                    Ok::<(), ExecuteError>(())
                })();
                if let Some(rollback_error) = helper_restore_error
                    && let Some(helper_rollback) = &helper_rollback
                {
                    let diagnostic = warning_result
                        .err()
                        .map(|error| {
                            format!("; additionally failed to report cleanup warnings: {error}")
                        })
                        .unwrap_or_default();
                    return Err(CliError::AppUpdateRollbackFailed {
                        original,
                        rollback: format!(
                            "{rollback_error}; retained helper rollback candidate at {}{diagnostic}",
                            helper_rollback.release_candidate,
                        ),
                    }
                    .into());
                }
                warning_result?;

                return Err(error.into());
            }
        };
        helper_install_cleanup_warning = install_outcome.cleanup_warning().map(str::to_string);
        if let Some(download) = &helper_only_download {
            let promotion_result = layout
                .install_release_helper(target_release_version, download)
                .map_err(ExecuteError::from)
                .and_then(|installed| {
                    if let Some(metadata) = &helper_metadata {
                        metadata.write(&installed)?;
                    }

                    Ok(())
                });
            let download_cleanup_error = remove_download(download).err();
            if let Err(error) = promotion_result {
                let original = error.to_string();
                let mut rollback_errors = Vec::new();
                let mut helper_cleanup_error = None;
                let mut helper_restore_cleanup_warning = None;
                if let Some(helper_rollback) = &helper_rollback {
                    match restore_helper(environment, helper_rollback) {
                        Ok(cleanup_warning) => {
                            helper_restore_cleanup_warning = cleanup_warning;
                        }
                        Err(rollback_error) => {
                            rollback_errors.push(format!("registered helper: {rollback_error}"));
                        }
                    }
                    if let Err(rollback_error) = restore_helper_release(helper_rollback) {
                        rollback_errors.push(format!("release helper: {rollback_error}"));
                    }
                    if rollback_errors.is_empty() {
                        helper_cleanup_error = cleanup_helper_rollback(Some(helper_rollback)).err();
                    } else {
                        rollback_errors.push(format!(
                            "retained helper rollback candidate at {}",
                            helper_rollback.release_candidate
                        ));
                    }
                }
                let warning_result = (|| {
                    write_download_cleanup_warning(stderr, download_cleanup_error)?;
                    write_helper_rollback_cleanup_warning(stderr, helper_cleanup_error)?;
                    write_privileged_helper_cleanup_warning(
                        stderr,
                        helper_restore_cleanup_warning.as_deref(),
                    )?;

                    Ok::<(), ExecuteError>(())
                })();
                if !rollback_errors.is_empty() {
                    if let Err(diagnostic_error) = warning_result {
                        rollback_errors.push(format!(
                            "failed to report cleanup warnings: {diagnostic_error}"
                        ));
                    }
                    return Err(CliError::AppUpdateRollbackFailed {
                        original,
                        rollback: rollback_errors.join("; "),
                    }
                    .into());
                }
                let mut failure_message = original;
                if let Err(diagnostic_error) = warning_result {
                    failure_message.push_str(&format!(
                        "; additionally failed to report cleanup warnings: {diagnostic_error}"
                    ));
                }

                return Err(CliError::PrivilegedHelperPromotionFailed {
                    message: failure_message,
                }
                .into());
            }
            write_download_cleanup_warning(stderr, download_cleanup_error)?;
        }
    }

    let helper_install_cleanup_warning = helper_install_cleanup_warning
        .as_deref()
        .map(|warning| format!("; warning: {warning}"))
        .unwrap_or_default();
    if !app_update_required {
        write_helper_rollback_cleanup_warning(
            stderr,
            cleanup_helper_rollback(helper_rollback.as_ref()).err(),
        )?;
        output.line(&format!(
            "Privileged helper: updated to {} (protocol {}){helper_install_cleanup_warning}",
            asset.helper().version(),
            asset.helper().protocol_version()
        ))?;
        if manifest.version() < &current_version {
            output.line(&format!(
                "PV application: current {current_version}; app manifest {} is older",
                manifest.version()
            ))?;
        } else {
            output.line(&format!("PV application: current {current_version}"))?;
        }

        return Ok(AppUpdateOutcome::Current { paths });
    }

    let updated_version = manifest.version().as_str().to_string();
    let transition_result = layout
        .activate_release(&updated_version)
        .map_err(ExecuteError::from)
        .and_then(|()| {
            restart_daemon_without_reconciliation(
                environment,
                &paths,
                &launch_agent_path,
                DaemonHealthCheck::AcceptProtocolMismatch,
            )
        });
    if let Err(error) = transition_result {
        return rollback_app_update(
            environment,
            RollbackContext {
                paths: &paths,
                layout: &layout,
                launch_agent_path: &launch_agent_path,
                helper_rollback: helper_rollback.as_ref(),
            },
            RollbackVersions {
                previous: &previous_version,
                failed: &updated_version,
            },
            error,
            &mut output,
            stderr,
        );
    }

    if helper_update_required {
        output.line(&format!(
            "Privileged helper: updated to {} (protocol {}){helper_install_cleanup_warning}",
            asset.helper().version(),
            asset.helper().protocol_version()
        ))?;
    } else if let InstalledHelperState::Ready(status) = &installed_helper {
        output.line(&format!(
            "Privileged helper: current {} (protocol {})",
            status.version, status.protocol_version
        ))?;
    }
    output.line(&format!(
        "PV application: updated {previous_version} -> {}",
        manifest.version()
    ))?;
    output.line("Daemon restarted and healthy")?;
    write_helper_rollback_cleanup_warning(
        stderr,
        cleanup_helper_rollback(helper_rollback.as_ref()).err(),
    )?;
    if let Err(error) = layout.prune_releases(&previous_version) {
        let mut stderr_output = Output::new(stderr, OutputMode::plain());
        stderr_output.line(&format!(
            "warning: failed to prune old PV app releases: {error}"
        ))?;
    }

    Ok(AppUpdateOutcome::Updated { paths })
}

struct RollbackVersions<'a> {
    previous: &'a str,
    failed: &'a str,
}

struct RollbackContext<'a> {
    paths: &'a PvPaths,
    layout: &'a state::AppReleaseLayout,
    launch_agent_path: &'a Utf8Path,
    helper_rollback: Option<&'a HelperRollbackPlan>,
}

enum InstalledHelperState {
    Ready(platform::PrivilegedHelperStatus),
    Missing,
    ProtocolMismatch,
}

struct AppUpdatePlan {
    app: bool,
    helper: HelperUpdatePlan,
}

#[derive(Clone, Copy)]
enum HelperUpdatePlan {
    Current,
    Update,
    RetainedOlderManifest,
    UnavailableOlderManifest,
}

fn app_update_plan(
    current_app: &AppUpdateVersion,
    target_app: &AppUpdateVersion,
    installed_helper: &InstalledHelperState,
    target_helper: &PrivilegedHelperUpdateAsset,
) -> Result<AppUpdatePlan, platform::PlatformError> {
    if target_app < current_app {
        let helper = match installed_helper {
            InstalledHelperState::Ready(status)
                if status.protocol_version == target_helper.protocol_version() =>
            {
                if privileged_helper_version_is_older(status, target_helper) {
                    HelperUpdatePlan::Update
                } else {
                    HelperUpdatePlan::RetainedOlderManifest
                }
            }
            InstalledHelperState::Ready(_) => HelperUpdatePlan::RetainedOlderManifest,
            InstalledHelperState::Missing | InstalledHelperState::ProtocolMismatch => {
                HelperUpdatePlan::UnavailableOlderManifest
            }
        };
        return Ok(AppUpdatePlan { app: false, helper });
    }
    let app = target_app > current_app;
    let helper_update_required = privileged_helper_update_required(installed_helper, target_helper);
    if !app
        && helper_update_required
        && target_helper.protocol_version() != platform::HELPER_PROTOCOL_VERSION
    {
        return Err(platform::PlatformError::PrivilegedHelperInstallation(
            "a privileged-helper protocol change requires a matching PV application update"
                .to_string(),
        ));
    }
    let helper = if helper_update_required {
        HelperUpdatePlan::Update
    } else {
        HelperUpdatePlan::Current
    };

    Ok(AppUpdatePlan { app, helper })
}

enum RegisteredHelperRollback {
    Restore,
    Remove,
}

struct HelperRollbackPlan {
    registered: RegisteredHelperRollback,
    release_path: Utf8PathBuf,
    release_candidate: Utf8PathBuf,
    release_metadata: HelperReleaseMetadata,
}

fn installed_helper_state(
    result: Result<platform::PrivilegedHelperStatus, platform::PlatformError>,
) -> Result<InstalledHelperState, platform::PlatformError> {
    match result {
        Ok(status) => Ok(InstalledHelperState::Ready(status)),
        Err(platform::PlatformError::PrivilegedHelperUnavailable) => {
            Ok(InstalledHelperState::Missing)
        }
        Err(platform::PlatformError::PrivilegedHelperProtocolMismatch { .. }) => {
            Ok(InstalledHelperState::ProtocolMismatch)
        }
        Err(error) => Err(error),
    }
}

fn prepare_helper_rollback(
    paths: &PvPaths,
    current_version: &AppUpdateVersion,
    installed: &InstalledHelperState,
) -> Result<HelperRollbackPlan, ExecuteError> {
    let source = paths.app_release_helper(current_version.as_str());
    let release_metadata = HelperReleaseMetadata::read(&source)?;
    let registered = match installed {
        InstalledHelperState::Ready(status) => {
            if status.version != release_metadata.version()
                || status.protocol_version != release_metadata.protocol_version()
            {
                return Err(CliError::PrivilegedHelperRollbackPreflight {
                    reason: format!(
                        "registered helper {} (protocol {}) does not match active release helper {} (protocol {}); run `pv setup` before updating",
                        status.version,
                        status.protocol_version,
                        release_metadata.version(),
                        release_metadata.protocol_version()
                    ),
                }
                .into());
            }
            RegisteredHelperRollback::Restore
        }
        InstalledHelperState::ProtocolMismatch => {
            return Err(CliError::PrivilegedHelperRollbackPreflight {
                reason: "the registered helper protocol cannot be restored exactly; run `pv setup` before updating"
                    .to_string(),
            }
            .into());
        }
        InstalledHelperState::Missing => RegisteredHelperRollback::Remove,
    };
    let sha256 = sha256_file(&source)?;
    if sha256 != release_metadata.sha256() {
        return Err(CliError::InvalidPrivilegedHelperReleaseMetadata {
            path: helper_metadata_path(&source).to_string(),
            reason: format!(
                "helper checksum mismatch: expected {}, got {sha256}",
                release_metadata.sha256()
            ),
        }
        .into());
    }

    Ok(HelperRollbackPlan {
        registered,
        release_path: source,
        release_candidate: temporary_app_download_path(paths, "helper-rollback"),
        release_metadata,
    })
}

fn materialize_helper_rollback(rollback: &HelperRollbackPlan) -> Result<(), StateError> {
    if let Some(parent) = rollback.release_candidate.parent() {
        state::fs::ensure_user_dir(parent)?;
    }
    state::fs::copy_file_atomically(&rollback.release_path, &rollback.release_candidate)
}

fn restore_helper(
    environment: &impl Environment,
    rollback: &HelperRollbackPlan,
) -> Result<Option<String>, platform::PlatformError> {
    match &rollback.registered {
        RegisteredHelperRollback::Restore => environment
            .install_privileged_helper(
                &rollback.release_candidate,
                rollback
                    .release_candidate
                    .parent()
                    .unwrap_or(Utf8Path::new(".")),
                rollback.release_metadata.sha256(),
                rollback.release_metadata.version(),
                rollback.release_metadata.protocol_version(),
            )
            .map(|outcome| outcome.cleanup_warning().map(str::to_string)),
        RegisteredHelperRollback::Remove => {
            environment.remove_privileged_helper()?;
            Ok(None)
        }
    }
}

fn cleanup_helper_rollback(rollback: Option<&HelperRollbackPlan>) -> Result<(), StateError> {
    if let Some(rollback) = rollback {
        state::fs::remove_file_if_exists(&rollback.release_candidate)?;
    }

    Ok(())
}

fn restore_helper_release(rollback: &HelperRollbackPlan) -> Result<(), ExecuteError> {
    state::fs::copy_file_atomically(&rollback.release_candidate, &rollback.release_path)?;
    rollback.release_metadata.write(&rollback.release_path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "app updater hashes the active release helper during rollback preflight"
)]
fn sha256_file(path: &Utf8Path) -> Result<String, ExecuteError> {
    let bytes = std::fs::read(path)?;

    Ok(sha256_digest_hex(Sha256::digest(bytes)))
}

fn rollback_app_update(
    environment: &impl Environment,
    context: RollbackContext<'_>,
    versions: RollbackVersions<'_>,
    original_error: ExecuteError,
    output: &mut Output<'_, impl Write>,
    stderr: &mut impl Write,
) -> Result<AppUpdateOutcome, ExecuteError> {
    let original_message = app_update_failure_message(context.paths, &original_error);
    let mut rollback_errors = Vec::new();
    let mut helper_restore_cleanup_warning = None;
    let mut reactivation_error = None;
    let previous_release_is_active = match context.layout.activate_release(versions.previous) {
        Ok(()) => true,
        Err(restore_error) => match context.layout.active_release() {
            Ok(Some(active_version)) if active_version == versions.previous => {
                reactivation_error = Some(restore_error);
                true
            }
            Ok(_) => {
                rollback_errors.push(format!("application: {restore_error}"));
                false
            }
            Err(active_release_error) => {
                rollback_errors.push(format!("application: {restore_error}"));
                rollback_errors.push(format!("application state: {active_release_error}"));
                false
            }
        },
    };
    if previous_release_is_active && let Some(helper_rollback) = context.helper_rollback {
        match restore_helper(environment, helper_rollback) {
            Ok(cleanup_warning) => helper_restore_cleanup_warning = cleanup_warning,
            Err(restore_error) => {
                rollback_errors.push(format!("privileged helper: {restore_error}"));
            }
        }
    }
    if let Some(reactivation_error) = reactivation_error {
        let helper_restore_succeeded = rollback_errors.is_empty();
        rollback_errors.push(format!("application reactivation: {reactivation_error}"));
        rollback_errors.push(format!(
            "retained failed application release at {}",
            context.paths.app_releases_dir().join(versions.failed)
        ));
        if helper_restore_succeeded
            && let Err(restart_error) = restart_daemon_without_reconciliation(
                environment,
                context.paths,
                context.launch_agent_path,
                DaemonHealthCheck::RequireCompatibleProtocol,
            )
        {
            rollback_errors.push(format!("daemon restart: {restart_error}"));
        }
    }
    if !rollback_errors.is_empty() {
        if let Some(helper_rollback) = context.helper_rollback {
            rollback_errors.push(format!(
                "retained helper rollback candidate at {}",
                helper_rollback.release_candidate
            ));
        }
        if let Err(error) = output.line("PV application: update failed; rollback failed") {
            rollback_errors.push(format!("failed to report rollback status: {error}"));
        }
        if let Err(error) = write_privileged_helper_cleanup_warning(
            stderr,
            helper_restore_cleanup_warning.as_deref(),
        ) {
            rollback_errors.push(format!(
                "failed to report privileged-helper cleanup warning: {error}"
            ));
        }

        return Err(CliError::AppUpdateRollbackFailed {
            original: original_message,
            rollback: rollback_errors.join("; "),
        }
        .into());
    }
    let helper_cleanup_error = cleanup_helper_rollback(context.helper_rollback).err();
    let cleanup_error = context.layout.remove_release(versions.failed).err();
    if let Err(rollback_error) = restart_daemon_without_reconciliation(
        environment,
        context.paths,
        context.launch_agent_path,
        DaemonHealthCheck::RequireCompatibleProtocol,
    ) {
        let mut rollback_message = rollback_error.to_string();
        if let Err(error) = output.line(&format!(
            "PV application: update failed; restored {}",
            versions.previous
        )) {
            rollback_message.push_str(&format!("; failed to report rollback status: {error}"));
        }
        if let Err(error) = write_helper_rollback_cleanup_warning(stderr, helper_cleanup_error) {
            rollback_message.push_str(&format!(
                "; failed to report privileged-helper cleanup warning: {error}"
            ));
        }
        if let Err(error) = write_privileged_helper_cleanup_warning(
            stderr,
            helper_restore_cleanup_warning.as_deref(),
        ) {
            rollback_message.push_str(&format!(
                "; failed to report privileged-helper cleanup warning: {error}"
            ));
        }
        if let Err(error) = write_cleanup_warning(stderr, cleanup_error) {
            rollback_message.push_str(&format!("; failed to report app cleanup warning: {error}"));
        }

        return Err(CliError::AppUpdateRollbackDaemonFailed {
            original: original_message,
            rollback: rollback_message,
        }
        .into());
    }

    let mut failure_message = original_message;
    if let Err(error) = output.line(&format!(
        "PV application: update failed; rolled back to {}",
        versions.previous
    )) {
        failure_message.push_str(&format!("; failed to report rollback status: {error}"));
    }
    if let Err(error) = write_helper_rollback_cleanup_warning(stderr, helper_cleanup_error) {
        failure_message.push_str(&format!(
            "; failed to report privileged-helper cleanup warning: {error}"
        ));
    }
    if let Err(error) =
        write_privileged_helper_cleanup_warning(stderr, helper_restore_cleanup_warning.as_deref())
    {
        failure_message.push_str(&format!(
            "; failed to report privileged-helper cleanup warning: {error}"
        ));
    }
    if let Err(error) = write_cleanup_warning(stderr, cleanup_error) {
        failure_message.push_str(&format!("; failed to report app cleanup warning: {error}"));
    }

    Err(CliError::AppUpdatePostActivationFailed {
        message: failure_message,
    }
    .into())
}

fn write_cleanup_warning(
    stderr: &mut impl Write,
    cleanup_error: Option<StateError>,
) -> Result<(), ExecuteError> {
    if let Some(error) = cleanup_error {
        let mut output = Output::new(stderr, OutputMode::plain());
        output.line(&format!(
            "warning: failed to remove failed PV app release: {error}"
        ))?;
    }

    Ok(())
}

fn write_helper_rollback_cleanup_warning(
    stderr: &mut impl Write,
    cleanup_error: Option<StateError>,
) -> Result<(), ExecuteError> {
    if let Some(error) = cleanup_error {
        let mut output = Output::new(stderr, OutputMode::plain());
        output.line(&format!(
            "warning: failed to remove privileged-helper rollback candidate: {error}"
        ))?;
    }

    Ok(())
}

fn write_privileged_helper_cleanup_warning(
    stderr: &mut impl Write,
    cleanup_warning: Option<&str>,
) -> Result<(), ExecuteError> {
    if let Some(warning) = cleanup_warning {
        let mut output = Output::new(stderr, OutputMode::plain());
        output.line(&format!("warning: {warning}"))?;
    }

    Ok(())
}

fn write_download_cleanup_warning(
    stderr: &mut impl Write,
    cleanup_error: Option<ExecuteError>,
) -> Result<(), ExecuteError> {
    if let Some(error) = cleanup_error {
        let mut output = Output::new(stderr, OutputMode::plain());
        output.line(&format!(
            "warning: failed to remove temporary PV app download: {error}"
        ))?;
    }

    Ok(())
}

fn app_update_failure_message(paths: &PvPaths, error: &ExecuteError) -> String {
    match error {
        ExecuteError::Daemon(_error) => daemon_startup_failure_message(paths)
            .unwrap_or_else(|| "daemon did not become healthy after update".to_string()),
        error => error.to_string(),
    }
}

fn daemon_startup_failure_message(paths: &PvPaths) -> Option<String> {
    let marker = read_daemon_startup_failure(paths)?;
    match marker.kind.as_str() {
        "migration_failed" => Some(format!(
            "database migration failed after update: {}",
            marker.message
        )),
        "startup_failed" => Some(format!(
            "daemon startup failed after update: {}",
            marker.message
        )),
        _ => None,
    }
}

fn read_daemon_startup_failure(paths: &PvPaths) -> Option<DaemonStartupFailureMarker> {
    let content = state::fs::read_to_string(&paths.daemon_startup_error()).ok()?;
    let marker = serde_json::from_str::<DaemonStartupFailureMarker>(&content).ok()?;
    if marker.kind.is_empty() || marker.message.is_empty() {
        return None;
    }

    Some(marker)
}

#[derive(Deserialize)]
struct DaemonStartupFailureMarker {
    kind: String,
    message: String,
}

#[derive(Serialize)]
struct UpdateCheckOutput {
    app: AppUpdateStatus,
    managed_resources: Vec<ManagedResourceUpdateCheckTrack>,
}

impl UpdateCheckOutput {
    fn write_plain(&self, output: &mut Output<'_, impl Write>) -> Result<(), ExecuteError> {
        self.app.write_plain(output)?;
        output.line("Managed Resources:")?;
        if self.managed_resources.is_empty() {
            output.line("  none installed")?;
            return Ok(());
        }

        for resource in &self.managed_resources {
            output.line(&format!("  {}", managed_resource_plain(resource)))?;
        }

        Ok(())
    }
}

#[derive(Serialize)]
struct AppUpdateStatus {
    status: AppUpdateStatusValue,
    current_version: String,
    latest_version: Option<String>,
    platform: String,
    asset: Option<AppUpdateAssetStatus>,
    helper: Option<PrivilegedHelperUpdateStatus>,
    reason: Option<String>,
}

impl AppUpdateStatus {
    fn write_plain(&self, output: &mut Output<'_, impl Write>) -> Result<(), ExecuteError> {
        match self.status {
            AppUpdateStatusValue::Current => {
                output.line(&format!("PV application: current {}", self.current_version))?
            }
            AppUpdateStatusValue::UpdateAvailable => output.line(&format!(
                "PV application: update available {} -> {} ({})",
                self.current_version,
                self.latest_version.as_deref().unwrap_or("unknown"),
                self.platform,
            ))?,
            AppUpdateStatusValue::Unavailable => output.line(&format!(
                "PV application: unavailable {} ({})",
                self.current_version,
                self.reason.as_deref().unwrap_or("unknown reason"),
            ))?,
        }

        if let Some(helper) = &self.helper {
            helper.write_plain(output)?;
        }

        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum AppUpdateStatusValue {
    Current,
    UpdateAvailable,
    Unavailable,
}

#[derive(Serialize)]
struct AppUpdateAssetStatus {
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Serialize)]
struct PrivilegedHelperUpdateStatus {
    status: AppUpdateStatusValue,
    current_version: Option<String>,
    latest_version: String,
    current_protocol_version: Option<u32>,
    latest_protocol_version: u32,
    url: String,
    sha256: String,
    size: u64,
    reason: Option<String>,
}

impl PrivilegedHelperUpdateStatus {
    fn write_plain(&self, output: &mut Output<'_, impl Write>) -> Result<(), ExecuteError> {
        match self.status {
            AppUpdateStatusValue::Current => output.line(&format!(
                "Privileged helper: current {} (protocol {})",
                self.current_version.as_deref().unwrap_or("unknown"),
                self.current_protocol_version
                    .map_or_else(|| "unknown".to_string(), |version| version.to_string())
            ))?,
            AppUpdateStatusValue::UpdateAvailable => output.line(&format!(
                "Privileged helper: update required {} -> {} (protocol {})",
                self.current_version.as_deref().unwrap_or("not installed"),
                self.latest_version,
                self.latest_protocol_version
            ))?,
            AppUpdateStatusValue::Unavailable => output.line(&format!(
                "Privileged helper: unavailable ({})",
                self.reason.as_deref().unwrap_or("unknown reason")
            ))?,
        }

        Ok(())
    }
}

fn app_update_status(environment: &impl Environment) -> Result<AppUpdateStatus, ExecuteError> {
    let manifest = fetch_app_update_manifest(environment)?;
    let current_version = AppUpdateVersion::current()?;
    let platform = match environment.app_update_platform() {
        Some(platform) => Ok(platform),
        None => AppUpdatePlatform::current(),
    };
    let platform = match platform {
        Ok(platform) => platform,
        Err(error) => {
            return Ok(app_update_status_unavailable(
                &current_version,
                "unsupported".to_string(),
                error.to_string(),
            ));
        }
    };
    let asset = match manifest.select_platform(platform) {
        Ok(asset) => asset,
        Err(error) => {
            return Ok(app_update_status_unavailable(
                &current_version,
                platform.to_string(),
                error.to_string(),
            ));
        }
    };
    let status = if manifest.version() > &current_version {
        AppUpdateStatusValue::UpdateAvailable
    } else {
        AppUpdateStatusValue::Current
    };
    let installed_helper = environment.privileged_helper_status();
    let installed_helper_state = match installed_helper.as_ref() {
        Ok(status) => Ok(InstalledHelperState::Ready((*status).clone())),
        Err(platform::PlatformError::PrivilegedHelperUnavailable) => {
            Ok(InstalledHelperState::Missing)
        }
        Err(platform::PlatformError::PrivilegedHelperProtocolMismatch { .. }) => {
            Ok(InstalledHelperState::ProtocolMismatch)
        }
        Err(error) => Err(error.to_string()),
    };
    let (helper_status, helper_reason) = match installed_helper_state {
        Ok(installed_helper_state) => match app_update_plan(
            &current_version,
            manifest.version(),
            &installed_helper_state,
            asset.helper(),
        ) {
            Ok(AppUpdatePlan {
                helper: HelperUpdatePlan::Update,
                ..
            }) => (AppUpdateStatusValue::UpdateAvailable, None),
            Ok(AppUpdatePlan {
                helper: HelperUpdatePlan::UnavailableOlderManifest,
                ..
            }) => (
                AppUpdateStatusValue::Unavailable,
                Some("the older app manifest has no applicable helper repair".to_string()),
            ),
            Ok(AppUpdatePlan {
                helper: HelperUpdatePlan::Current | HelperUpdatePlan::RetainedOlderManifest,
                ..
            }) => (AppUpdateStatusValue::Current, None),
            Err(error) => (AppUpdateStatusValue::Unavailable, Some(error.to_string())),
        },
        Err(reason) => (AppUpdateStatusValue::Unavailable, Some(reason)),
    };
    let helper = PrivilegedHelperUpdateStatus {
        status: helper_status,
        current_version: installed_helper
            .as_ref()
            .ok()
            .map(|status| status.version.clone()),
        latest_version: asset.helper().version().to_string(),
        current_protocol_version: installed_helper
            .as_ref()
            .ok()
            .map(|status| status.protocol_version),
        latest_protocol_version: asset.helper().protocol_version(),
        url: asset.helper().url().to_string(),
        sha256: asset.helper().sha256().as_str().to_string(),
        size: asset.helper().size(),
        reason: helper_reason,
    };

    Ok(AppUpdateStatus {
        status,
        current_version: current_version.to_string(),
        latest_version: Some(manifest.version().to_string()),
        platform: platform.to_string(),
        asset: Some(AppUpdateAssetStatus {
            url: asset.url().to_string(),
            sha256: asset.sha256().as_str().to_string(),
            size: asset.size(),
        }),
        helper: Some(helper),
        reason: None,
    })
}

fn app_update_status_unavailable(
    current_version: &AppUpdateVersion,
    platform: String,
    reason: String,
) -> AppUpdateStatus {
    AppUpdateStatus {
        status: AppUpdateStatusValue::Unavailable,
        current_version: current_version.to_string(),
        latest_version: None,
        platform,
        asset: None,
        helper: None,
        reason: Some(reason),
    }
}

fn privileged_helper_update_required(
    status: &InstalledHelperState,
    target: &PrivilegedHelperUpdateAsset,
) -> bool {
    let InstalledHelperState::Ready(status) = status else {
        return true;
    };
    if status.protocol_version != target.protocol_version() {
        return true;
    }

    privileged_helper_version_is_older(status, target)
}

fn privileged_helper_version_is_older(
    status: &platform::PrivilegedHelperStatus,
    target: &PrivilegedHelperUpdateAsset,
) -> bool {
    AppUpdateVersion::parse(status.version.clone())
        .map(|version| version < *target.version())
        .unwrap_or(true)
}

fn managed_resource_plain(resource: &ManagedResourceUpdateCheckTrack) -> String {
    let mut line = match resource.status {
        ResourceUpdateStatus::Current => format!(
            "{} {}: current {}",
            resource.resource, resource.track, resource.current_artifact_version
        ),
        ResourceUpdateStatus::UpdateAvailable => format!(
            "{} {}: update available {} -> {}",
            resource.resource,
            resource.track,
            resource.current_artifact_version,
            resource
                .latest_artifact_version
                .as_deref()
                .unwrap_or("unknown"),
        ),
        ResourceUpdateStatus::Blocked => {
            let blocked_by = resource
                .blocked_by
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "requires newer PV".to_string());
            format!(
                "{} {}: blocked {} ({})",
                resource.resource, resource.track, resource.current_artifact_version, blocked_by
            )
        }
        ResourceUpdateStatus::Revoked => {
            let current_revocation = resource
                .current_revocation
                .as_ref()
                .map(|revocation| revocation.reason.as_str())
                .unwrap_or("revoked");
            let replacement = resource
                .latest_artifact_version
                .as_ref()
                .map(|version| format!("; replacement {version}"))
                .unwrap_or_default();
            format!(
                "{} {}: revoked {} ({}){}",
                resource.resource,
                resource.track,
                resource.current_artifact_version,
                current_revocation,
                replacement,
            )
        }
        ResourceUpdateStatus::Unavailable => format!(
            "{} {}: unavailable {} ({})",
            resource.resource,
            resource.track,
            resource.current_artifact_version,
            resource.reason.as_deref().unwrap_or("unknown reason"),
        ),
    };

    if let Some(revocation) = &resource.latest_revocation {
        line.push_str(&format!(
            "; newest {} revoked: {}",
            revocation.artifact_version, revocation.reason
        ));
    }

    line
}

fn update_check_daemon_error(error: daemon::DaemonError) -> ExecuteError {
    match error {
        daemon::DaemonError::Io(source)
            if matches!(
                source.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::TimedOut
            ) =>
        {
            CliError::UpdateCheckDaemonUnavailable.into()
        }
        daemon::DaemonError::DaemonRejected { message } => {
            CliError::UpdateCheckFailed { message }.into()
        }
        error => error.into(),
    }
}

fn managed_resource_update_daemon_error(error: daemon::DaemonError) -> ExecuteError {
    match error {
        daemon::DaemonError::Io(source)
            if matches!(
                source.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::TimedOut
            ) =>
        {
            CliError::ManagedResourceUpdateDaemonUnavailable.into()
        }
        daemon::DaemonError::DaemonRejected { message } => {
            CliError::ManagedResourceUpdateFailed { message }.into()
        }
        error => error.into(),
    }
}

fn with_resource_http_client<T>(
    environment: &impl Environment,
    operation: impl FnOnce(&dyn ResourceHttpClient) -> resources::Result<T>,
) -> Result<T, ExecuteError> {
    if let Some(client) = environment.resource_http_client() {
        return Ok(operation(client)?);
    }

    let client = UreqResourceHttpClient::default();
    Ok(operation(&client)?)
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn utf8_path(path: impl Into<std::path::PathBuf>) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(path.into()).map_err(|path| CliError::NonUtf8Path { path }.into())
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use camino_tempfile::tempdir;

    use super::create_download_file;

    #[test]
    fn create_download_file_rejects_existing_path_without_truncating() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let path = tempdir.path().join("pv-app-existing.tmp");
        state::fs::write_sensitive_file(&path, "existing content")?;

        let Err(error) = create_download_file(&path) else {
            anyhow::bail!("expected existing download path to be rejected");
        };

        let state::StateError::Filesystem {
            path: error_path,
            source,
        } = error
        else {
            anyhow::bail!("expected filesystem error for existing download path");
        };
        assert_eq!(error_path, path);
        assert_eq!(source.kind(), ErrorKind::AlreadyExists);
        assert_eq!(state::fs::read_to_string(&path)?, "existing content");

        Ok(())
    }
}
