use std::collections::{BTreeMap, btree_map};
use std::net::TcpListener;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use resources::{ArtifactManifestCache, ResourceName, TrackSelector};
use state::{
    Database, PortRequest, PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START,
};
use tokio::time::timeout;

use crate::{DaemonError, ProcessSpec};

#[expect(
    clippy::disallowed_types,
    reason = "daemon runtime owns FrankenPHP config validation process execution"
)]
type FrankenphpProcessCommand = tokio::process::Command;

const CONFIG_VALIDATION_TIMEOUT: Duration = Duration::from_secs(10);

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
