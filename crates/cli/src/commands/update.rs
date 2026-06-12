use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use protocol::{
    ManagedResourceUpdateCheckTrack, ManagedResourceUpdateStatus as ResourceUpdateStatus,
};
use resources::{ResourceHttpClient, UreqResourceHttpClient};
use serde::Serialize;
use state::{PvPaths, StateError};

use crate::args::UpdateArgs;
use crate::environment::{Environment, app_update_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn run(
    args: UpdateArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    if !args.check {
        return Err(CliError::UpdateNotImplemented.into());
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
    let url = app_update_manifest_url(environment);
    let json = with_resource_http_client(environment, |client| client.get_text(&url))?;
    let manifest = self_update::AppUpdateManifest::parse(&json)?;
    let current_version = self_update::AppUpdateVersion::current()?;
    let platform = match environment.app_update_platform() {
        Some(platform) => Ok(platform),
        None => self_update::AppUpdatePlatform::current(),
    };
    let platform = match platform {
        Ok(platform) => platform,
        Err(error) => {
            return Ok(AppUpdateStatus {
                status: AppUpdateStatusValue::Unavailable,
                current_version: current_version.to_string(),
                latest_version: None,
                platform: "unsupported".to_string(),
                asset: None,
                reason: Some(error.to_string()),
            });
        }
    };
    let asset = match manifest.select_platform(platform) {
        Ok(asset) => asset,
        Err(error) => {
            return Ok(AppUpdateStatus {
                status: AppUpdateStatusValue::Unavailable,
                current_version: current_version.to_string(),
                latest_version: None,
                platform: platform.to_string(),
                asset: None,
                reason: Some(error.to_string()),
            });
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
                .map(|blocked_by| {
                    format!(
                        "requires PV {}, current PV {}",
                        blocked_by.minimum_pv_version, blocked_by.current_pv_version
                    )
                })
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
        daemon::DaemonError::Io(source) if daemon_is_unavailable(&source) => {
            CliError::UpdateCheckDaemonUnavailable.into()
        }
        daemon::DaemonError::DaemonRejected { message } => {
            CliError::UpdateCheckFailed { message }.into()
        }
        error => error.into(),
    }
}

fn daemon_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
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
