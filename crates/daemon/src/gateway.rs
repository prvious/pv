use std::collections::{BTreeMap, btree_map};
use std::net::TcpListener;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use resources::{
    ArtifactManifestCache, ResourceAdapter, ResourceName, TrackSelector, frankenphp_adapter,
};
use state::{
    Database, ManagedResourceDesiredState, ManagedResourceTrackRecord, PortRequest, PvPaths,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, RuntimeObservedStatus, RuntimeSubject,
};
use tokio::time::timeout;

use crate::gateway_config::{
    GatewayConfigInput, GatewayProjectRoute, PhpWorkerConfigInput, PhpWorkerProject,
    promote_validated_config_async, render_gateway_config, render_php_worker_config,
};
use crate::{DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_readiness};

#[expect(
    clippy::disallowed_types,
    reason = "daemon runtime owns FrankenPHP config validation process execution"
)]
type FrankenphpProcessCommand = tokio::process::Command;

const CONFIG_VALIDATION_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const GATEWAY_RUNTIME_RECONCILED: &str = "Gateway runtime reconciled";
const FRANKENPHP_NOT_INSTALLED: &str = "Gateway runtime skipped; FrankenPHP is not installed";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrankenphpCommand {
    executable: Utf8PathBuf,
}

impl FrankenphpCommand {
    pub fn new(executable: impl Into<Utf8PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn executable(&self) -> &Utf8Path {
        &self.executable
    }

    pub fn validate_arguments(&self, config_path: &Utf8Path) -> Vec<String> {
        frankenphp_config_arguments("validate", config_path)
    }

    pub fn run_arguments(&self, config_path: &Utf8Path) -> Vec<String> {
        frankenphp_config_arguments("run", config_path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePlan {
    pub gateway: GatewayRuntimePlan,
    pub workers: Vec<PhpWorkerRuntimePlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayRuntimePlan {
    pub http_port: u16,
    pub https_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerRuntimePlan {
    pub php_track: String,
    pub port: u16,
    pub projects: Vec<RuntimeProject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeProject {
    pub id: String,
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub project_root: Utf8PathBuf,
    pub document_root: Utf8PathBuf,
}

pub fn promote_validated_config_for_test(
    path: &Utf8Path,
    content: &str,
    validate: impl FnOnce(&Utf8Path) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    crate::gateway_config::promote_validated_config(path, content, validate)
}

pub async fn reconcile_gateway_runtimes(paths: &PvPaths) -> Result<String, DaemonError> {
    let Some(gateway_command) = first_installed_frankenphp_command(paths)? else {
        record_runtime_observed(
            paths,
            RuntimeSubject::Gateway,
            RuntimeObservedStatus::Stopped,
            Some(FRANKENPHP_NOT_INSTALLED),
        )?;

        return Ok(FRANKENPHP_NOT_INSTALLED.to_owned());
    };

    let plan = match build_runtime_plan(paths) {
        Ok(plan) => plan,
        Err(error) => {
            record_runtime_error(paths, RuntimeSubject::Gateway, &error)?;

            return Err(error);
        }
    };
    let supervisor = ProcessSupervisor::new(paths.clone());

    reconcile_gateway_config(paths, &gateway_command, &plan).await?;
    start_or_adopt_runtime(
        paths,
        &supervisor,
        gateway_process_spec(paths, &gateway_command),
        ReadinessCheck::Tcp {
            host: "127.0.0.1".to_owned(),
            port: plan.gateway.http_port,
        },
        RuntimeSubject::Gateway,
    )
    .await?;

    for worker in &plan.workers {
        let subject = RuntimeSubject::PhpWorker {
            php_track: worker.php_track.clone(),
        };
        let worker_command = match installed_frankenphp_command_for_track(paths, &worker.php_track)
        {
            Ok(Some(command)) => command,
            Ok(None) => {
                let error = DaemonError::UnexpectedProtocolResponse {
                    reason: format!(
                        "FrankenPHP is not installed for PHP track `{}`",
                        worker.php_track
                    ),
                };
                record_runtime_error(paths, subject, &error)?;

                return Err(error);
            }
            Err(error) => {
                record_runtime_error(paths, subject, &error)?;

                return Err(error);
            }
        };
        reconcile_worker_config(paths, &worker_command, worker).await?;
        start_or_adopt_runtime(
            paths,
            &supervisor,
            worker_process_spec(paths, &worker.php_track, &worker_command),
            ReadinessCheck::Tcp {
                host: "127.0.0.1".to_owned(),
                port: worker.port,
            },
            subject,
        )
        .await?;
    }

    Ok(GATEWAY_RUNTIME_RECONCILED.to_owned())
}

pub async fn validate_config(
    command: &FrankenphpCommand,
    config_path: &Utf8Path,
) -> Result<(), DaemonError> {
    let output = timeout(CONFIG_VALIDATION_TIMEOUT, async {
        FrankenphpProcessCommand::new(command.executable())
            .args(command.validate_arguments(config_path))
            .output()
            .await
    })
    .await
    .map_err(|_elapsed| DaemonError::ProtocolTimedOut {
        phase: "FrankenPHP config validation",
    })??;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Err(DaemonError::UnexpectedProtocolResponse {
        reason: format!(
            "FrankenPHP config validation failed for {config_path}: status={status}; stdout={stdout}; stderr={stderr}",
            status = output.status
        ),
    })
}

pub fn gateway_process_spec(paths: &PvPaths, command: &FrankenphpCommand) -> ProcessSpec {
    ProcessSpec {
        name: "gateway".to_owned(),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.gateway_root_config()),
        config_path: paths.gateway_root_config(),
        log_path: paths.gateway_log(),
        pid_path: paths.gateway_pid(),
        metadata_path: paths.gateway_runtime_metadata(),
        resource_name: "gateway".to_owned(),
        track: "core".to_owned(),
    }
}

pub fn worker_process_spec(
    paths: &PvPaths,
    php_track: &str,
    command: &FrankenphpCommand,
) -> ProcessSpec {
    ProcessSpec {
        name: format!("php-worker-{php_track}"),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.worker_root_config(php_track)),
        config_path: paths.worker_root_config(php_track),
        log_path: paths.worker_log(php_track),
        pid_path: paths.worker_pid(php_track),
        metadata_path: paths.worker_runtime_metadata(php_track),
        resource_name: "php-worker".to_owned(),
        track: php_track.to_owned(),
    }
}

pub fn build_runtime_plan(paths: &PvPaths) -> Result<RuntimePlan, DaemonError> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(local_loopback_port_available)?;
    let mut projects_by_php_track: BTreeMap<String, PhpWorkerRuntimePlan> = BTreeMap::new();

    for project in database.projects()? {
        let config_file = ProjectConfigFile::read_from_root(&project.path)?;
        let php_track = resolve_project_php_track(
            paths,
            config_file.config.php.as_deref(),
            project.desired_php_track.as_deref(),
        )?;
        let document_root = match config_file.config.document_root {
            Some(document_root) => project.path.join(document_root),
            None => project.path.clone(),
        };
        let runtime_project = RuntimeProject {
            id: project.id,
            primary_hostname: project.primary_hostname.clone(),
            hostnames: additional_hostnames(
                &project.primary_hostname,
                project.additional_hostnames,
                config_file.config.hostnames,
            ),
            project_root: project.path,
            document_root,
        };

        match projects_by_php_track.entry(php_track.clone()) {
            btree_map::Entry::Occupied(mut entry) => {
                entry.get_mut().projects.push(runtime_project);
            }
            btree_map::Entry::Vacant(entry) => {
                let assignment = database.assign_port(
                    PortRequest::php_worker(
                        &php_track,
                        RUNTIME_PORT_FALLBACK_START,
                        RUNTIME_PORT_FALLBACK_START,
                        RUNTIME_PORT_FALLBACK_END,
                    ),
                    local_loopback_port_available,
                )?;

                entry.insert(PhpWorkerRuntimePlan {
                    php_track,
                    port: assignment.port,
                    projects: vec![runtime_project],
                });
            }
        }
    }

    let workers = projects_by_php_track
        .into_values()
        .map(|mut worker| {
            worker
                .projects
                .sort_by(|left, right| left.primary_hostname.cmp(&right.primary_hostname));
            worker
        })
        .collect();

    Ok(RuntimePlan {
        gateway: GatewayRuntimePlan {
            http_port: gateway_ports.http.port,
            https_port: gateway_ports.https.port,
            ca_certificate_path: paths.ca_certificate(),
            ca_private_key_path: paths.ca_private_key(),
        },
        workers,
    })
}

async fn reconcile_gateway_config(
    paths: &PvPaths,
    command: &FrankenphpCommand,
    plan: &RuntimePlan,
) -> Result<(), DaemonError> {
    let content = match render_gateway_config(&GatewayConfigInput {
        http_port: plan.gateway.http_port,
        https_port: plan.gateway.https_port,
        ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        ca_private_key_path: plan.gateway.ca_private_key_path.clone(),
        routes: gateway_project_routes(plan),
    }) {
        Ok(content) => content,
        Err(error) => {
            record_runtime_error(paths, RuntimeSubject::Gateway, &error)?;

            return Err(error);
        }
    };

    promote_runtime_config(
        paths,
        RuntimeSubject::Gateway,
        paths.gateway_root_config(),
        &content,
        command,
    )
    .await
}

async fn reconcile_worker_config(
    paths: &PvPaths,
    command: &FrankenphpCommand,
    worker: &PhpWorkerRuntimePlan,
) -> Result<(), DaemonError> {
    let subject = RuntimeSubject::PhpWorker {
        php_track: worker.php_track.clone(),
    };
    let content = match render_php_worker_config(&PhpWorkerConfigInput {
        php_track: worker.php_track.clone(),
        port: worker.port,
        projects: worker
            .projects
            .iter()
            .map(|project| PhpWorkerProject {
                primary_hostname: project.primary_hostname.clone(),
                hostnames: project.hostnames.clone(),
                project_root: project.project_root.clone(),
                document_root: project.document_root.clone(),
            })
            .collect(),
    }) {
        Ok(content) => content,
        Err(error) => {
            record_runtime_error(paths, subject, &error)?;

            return Err(error);
        }
    };

    promote_runtime_config(
        paths,
        subject,
        paths.worker_root_config(&worker.php_track),
        &content,
        command,
    )
    .await
}

async fn promote_runtime_config(
    paths: &PvPaths,
    subject: RuntimeSubject,
    config_path: Utf8PathBuf,
    content: &str,
    command: &FrankenphpCommand,
) -> Result<(), DaemonError> {
    let result =
        promote_validated_config_async(&config_path, content, |candidate_path| async move {
            validate_config(command, &candidate_path).await
        })
        .await;

    if let Err(error) = &result {
        record_runtime_error(paths, subject, error)?;
    }

    result
}

async fn start_or_adopt_runtime(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    spec: ProcessSpec,
    readiness: ReadinessCheck,
    subject: RuntimeSubject,
) -> Result<(), DaemonError> {
    let result = async {
        if supervisor.adopt(&spec)?.is_none() {
            let process = supervisor.start(spec).await?;
            if let Err(error) = wait_for_readiness(readiness, RUNTIME_READINESS_TIMEOUT).await {
                process.stop(Duration::from_secs(1)).await?;

                return Err(error);
            }

            return Ok(());
        }

        wait_for_readiness(readiness, RUNTIME_READINESS_TIMEOUT).await
    }
    .await;

    match result {
        Ok(()) => record_runtime_observed(
            paths,
            subject,
            RuntimeObservedStatus::Running,
            Some(GATEWAY_RUNTIME_RECONCILED),
        ),
        Err(error) => {
            record_runtime_error(paths, subject, &error)?;

            Err(error)
        }
    }
}

fn gateway_project_routes(plan: &RuntimePlan) -> Vec<GatewayProjectRoute> {
    plan.workers
        .iter()
        .flat_map(|worker| {
            worker.projects.iter().map(|project| GatewayProjectRoute {
                primary_hostname: project.primary_hostname.clone(),
                hostnames: project.hostnames.clone(),
                worker_port: worker.port,
            })
        })
        .collect()
}

fn first_installed_frankenphp_command(
    paths: &PvPaths,
) -> Result<Option<FrankenphpCommand>, DaemonError> {
    let database = Database::open(paths)?;
    let mut tracks = installed_frankenphp_tracks(&database)?;
    let Some(record) = tracks.pop() else {
        return Ok(None);
    };

    Ok(Some(frankenphp_command_from_record(record)?))
}

fn installed_frankenphp_command_for_track(
    paths: &PvPaths,
    php_track: &str,
) -> Result<Option<FrankenphpCommand>, DaemonError> {
    let database = Database::open(paths)?;
    let command = installed_frankenphp_tracks(&database)?
        .into_iter()
        .find(|record| record.track == php_track)
        .map(frankenphp_command_from_record)
        .transpose()?;

    Ok(command)
}

fn installed_frankenphp_tracks(
    database: &Database,
) -> Result<Vec<ManagedResourceTrackRecord>, DaemonError> {
    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .filter(|record| {
            record.resource_name == "frankenphp"
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
        .collect())
}

fn frankenphp_command_from_record(
    record: ManagedResourceTrackRecord,
) -> Result<FrankenphpCommand, DaemonError> {
    let Some(artifact_path) = record.current_artifact_path else {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "installed FrankenPHP track `{}` is missing an artifact path",
                record.track
            ),
        });
    };
    let adapter = frankenphp_adapter()?;

    adapter.validate_installation(&artifact_path)?;

    Ok(FrankenphpCommand::new(
        adapter.executable_path(&artifact_path),
    ))
}

fn record_runtime_error(
    paths: &PvPaths,
    subject: RuntimeSubject,
    error: &DaemonError,
) -> Result<(), DaemonError> {
    record_runtime_observed(
        paths,
        subject,
        RuntimeObservedStatus::Failed,
        Some(&error.to_string()),
    )
}

fn record_runtime_observed(
    paths: &PvPaths,
    subject: RuntimeSubject,
    status: RuntimeObservedStatus,
    message: Option<&str>,
) -> Result<(), DaemonError> {
    let mut database = Database::open(paths)?;
    database.record_runtime_observed_snapshot(subject, status, message)?;

    Ok(())
}

fn resolve_project_php_track(
    paths: &PvPaths,
    config_selector: Option<&str>,
    stored_selector: Option<&str>,
) -> Result<String, DaemonError> {
    let selector = config_selector
        .or(stored_selector)
        .map(TrackSelector::parse)
        .transpose()?
        .unwrap_or(TrackSelector::Latest);

    match selector {
        TrackSelector::Latest => latest_php_track(paths),
        TrackSelector::Track(track) => Ok(track.as_str().to_owned()),
    }
}

fn latest_php_track(paths: &PvPaths) -> Result<String, DaemonError> {
    let manifest = ArtifactManifestCache::new(paths.downloads().to_path_buf()).load_cached()?;
    let php = ResourceName::new("php")?;
    let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

    Ok(track.as_str().to_owned())
}

fn additional_hostnames(
    primary_hostname: &str,
    state_hostnames: Vec<String>,
    config_hostnames: Vec<String>,
) -> Vec<String> {
    let mut hostnames = state_hostnames
        .into_iter()
        .chain(config_hostnames)
        .filter(|hostname| hostname != primary_hostname)
        .collect::<Vec<_>>();

    hostnames.sort();
    hostnames.dedup();
    hostnames
}

fn local_loopback_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn frankenphp_config_arguments(action: &str, config_path: &Utf8Path) -> Vec<String> {
    vec![
        action.to_owned(),
        "--config".to_owned(),
        config_path.as_str().to_owned(),
        "--adapter".to_owned(),
        "caddyfile".to_owned(),
    ]
}
