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
use self_update::{AppUpdateAsset, AppUpdateManifest, AppUpdatePlatform, AppUpdateVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use state::{PvPaths, StateError};

use crate::args::UpdateArgs;
use crate::environment::{Environment, app_update_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

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

    run_managed_resource_update_phase(paths, stdout)
}

fn run_update(
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let outcome = run_app_update_phase(environment, stdout, stderr)?;

    match outcome {
        AppUpdateOutcome::Current { paths } => run_managed_resource_update_phase(paths, stdout),
        AppUpdateOutcome::Updated { paths } => {
            let active_pv_binary = paths.active_pv_binary();

            reexec_managed_resource_update(environment, &active_pv_binary)
        }
    }
}

fn run_managed_resource_update_phase(
    paths: PvPaths,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let job = daemon::run_job_blocking(paths, "update", "system")
        .map_err(managed_resource_update_daemon_error)?;
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
) -> Result<LaunchAgentReload, ExecuteError> {
    let expected = launch_agent_config(paths);
    let path = launch_agent_path(environment)?;
    match platform::inspect_launch_agent_file(&path, Some(&expected)) {
        LaunchAgentFileState::Current { .. } => Ok(LaunchAgentReload::NotRequired),
        LaunchAgentFileState::Stale { .. } => {
            platform::write_launch_agent_file(&path, &expected)?;

            Ok(LaunchAgentReload::Required { path })
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

#[derive(Clone, Debug)]
enum LaunchAgentReload {
    NotRequired,
    Required { path: Utf8PathBuf },
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
    asset: &AppUpdateAsset,
    stderr: &mut impl Write,
) -> Result<Utf8PathBuf, ExecuteError> {
    state::fs::ensure_user_dir(paths.downloads())?;
    let path = temporary_app_download_path(paths);
    let file = create_download_file(&path)?;
    let mut writer = CountingSha256Writer::new(file);
    let download_result = with_resource_http_client(environment, |client| {
        client.download(asset.url(), &mut writer)
    });
    if let Err(error) = download_result {
        write_download_cleanup_warning(stderr, remove_download(&path).err())?;
        return Err(error);
    }

    let stats = writer.finish();
    if stats.size != asset.size() {
        write_download_cleanup_warning(stderr, remove_download(&path).err())?;
        return Err(CliError::AppUpdateSizeMismatch {
            url: asset.url().to_string(),
            expected: asset.size(),
            actual: stats.size,
        }
        .into());
    }
    if stats.sha256 != asset.sha256().as_str() {
        write_download_cleanup_warning(stderr, remove_download(&path).err())?;
        return Err(CliError::AppUpdateChecksumMismatch {
            url: asset.url().to_string(),
            expected: asset.sha256().as_str().to_string(),
            actual: stats.sha256,
        }
        .into());
    }

    Ok(path)
}

fn temporary_app_download_path(paths: &PvPaths) -> Utf8PathBuf {
    let process_id = std::process::id();
    let counter = APP_DOWNLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);

    paths
        .downloads()
        .join(format!("pv-app-{process_id}-{counter}.tmp"))
}

fn remove_download(path: &Utf8Path) -> Result<(), ExecuteError> {
    state::fs::remove_file_if_exists(path)?;

    Ok(())
}

fn restart_daemon_without_reconciliation(
    environment: &impl Environment,
    paths: &PvPaths,
    reload: &LaunchAgentReload,
    health_check: DaemonHealthCheck,
) -> Result<(), ExecuteError> {
    if let LaunchAgentReload::Required { path } = reload {
        bootout_launch_agent_if_loaded(environment)?;
        environment.bootstrap_launch_agent(path)?;
    }
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
        }
        platform::PlatformError::LaunchAgentCommandStatus { .. } => false,
        _ => false,
    }
}

fn update_state_error(error: StateError) -> ExecuteError {
    match error {
        StateError::UpdateInProgress { path } => CliError::UpdateInProgress {
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
    let launch_agent_reload = normalize_launch_agent(environment, &paths)?;

    let manifest = fetch_app_update_manifest(environment)?;
    if manifest.version() <= &current_version {
        output.line(&format!("PV application: current {current_version}"))?;

        return Ok(AppUpdateOutcome::Current { paths });
    }

    let platform = environment
        .app_update_platform()
        .map(Ok)
        .unwrap_or_else(AppUpdatePlatform::current)?;
    let asset = manifest.select_platform(platform)?;
    let downloaded = download_app_asset(environment, &paths, asset, stderr)?;
    let install_result = layout.install_release_binary(manifest.version().as_str(), &downloaded);
    let cleanup_result = remove_download(&downloaded);
    match (install_result, cleanup_result) {
        (Ok(_install), Ok(())) => {}
        (Ok(_install), Err(cleanup_error)) => return Err(cleanup_error),
        (Err(install_error), cleanup_result) => {
            write_download_cleanup_warning(stderr, cleanup_result.err())?;
            return Err(install_error.into());
        }
    }
    let updated_version = manifest.version().as_str().to_string();
    layout.activate_release(&updated_version)?;
    if let Err(error) = restart_daemon_without_reconciliation(
        environment,
        &paths,
        &launch_agent_reload,
        DaemonHealthCheck::AcceptProtocolMismatch,
    ) {
        return rollback_app_update(
            environment,
            RollbackContext {
                paths: &paths,
                layout: &layout,
                launch_agent_reload: &launch_agent_reload,
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

    output.line(&format!(
        "PV application: updated {previous_version} -> {}",
        manifest.version()
    ))?;
    output.line("Daemon restarted and healthy")?;
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
    launch_agent_reload: &'a LaunchAgentReload,
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
    if let Err(restore_error) = context.layout.activate_release(versions.previous) {
        output.line("PV application: update failed; rollback failed")?;

        return Err(CliError::AppUpdateRollbackFailed {
            original: original_message,
            rollback: restore_error.to_string(),
        }
        .into());
    }

    let cleanup_error = context.layout.remove_release(versions.failed).err();
    if let Err(rollback_error) = restart_daemon_without_reconciliation(
        environment,
        context.paths,
        context.launch_agent_reload,
        DaemonHealthCheck::RequireCompatibleProtocol,
    ) {
        output.line(&format!(
            "PV application: update failed; restored {}",
            versions.previous
        ))?;
        write_cleanup_warning(stderr, cleanup_error)?;

        return Err(CliError::AppUpdateRollbackDaemonFailed {
            original: original_message,
            rollback: rollback_error.to_string(),
        }
        .into());
    }

    output.line(&format!(
        "PV application: update failed; rolled back to {}",
        versions.previous
    ))?;
    write_cleanup_warning(stderr, cleanup_error)?;

    Err(CliError::AppUpdatePostActivationFailed {
        message: original_message,
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
        reason: Some(reason),
    }
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
