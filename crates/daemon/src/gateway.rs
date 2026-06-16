use std::collections::{BTreeMap, BTreeSet, btree_map};
use std::io;
use std::net::TcpListener;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use resources::{ResourceAdapter, frankenphp_adapter};
use rustix::process::{Pid, Signal, kill_process_group};
use sha2::{Digest, Sha256};
use state::{
    Database, ManagedResourceDesiredState, ManagedResourceTrackRecord, PortOwner, PortRequest,
    ProjectEnvObservedStatus, PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START,
    RuntimeObservedStatus, RuntimeSubject, StateError, fs,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::timeout;

use crate::gateway_config::{
    GatewayConfigInput, GatewayProjectRoute, PhpWorkerConfigInput, PhpWorkerProject,
    PromotedConfigDir, PromotedConfigTree, promote_config_dir, promote_validated_config_tree_async,
    render_gateway_config, render_gateway_project_config, render_php_worker_config,
    render_php_worker_project_config,
};
use crate::project_env::{resolve_project_php_track, validate_project_config_for_gateway};
use crate::supervisor::probe_readiness_once;
use crate::{DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_readiness};

#[expect(
    clippy::disallowed_types,
    reason = "daemon runtime owns FrankenPHP config validation process execution"
)]
type FrankenphpProcessCommand = tokio::process::Command;

const CONFIG_VALIDATION_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const GATEWAY_RUNTIME_RECONCILED: &str = "Gateway runtime reconciled";
pub(crate) const FRANKENPHP_NOT_INSTALLED: &str =
    "Gateway runtime skipped; FrankenPHP is not installed";
static CANDIDATE_CONFIG_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    pub storage_path: Utf8PathBuf,
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
    pub render_config: bool,
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
    let mut worker_commands = Vec::new();

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
        worker_commands.push((worker, worker_command));
    }

    for (worker, worker_command) in worker_commands {
        let subject = RuntimeSubject::PhpWorker {
            php_track: worker.php_track.clone(),
        };
        let promoted_config = reconcile_worker_config(paths, &worker_command, worker).await?;
        start_or_adopt_promoted_runtime(
            paths,
            &supervisor,
            promoted_config,
            worker_process_spec(paths, &worker.php_track, &worker_command),
            ReadinessCheck::Tcp {
                host: "127.0.0.1".to_owned(),
                port: worker.port,
            },
            subject,
        )
        .await?;
    }
    let gateway_config = reconcile_gateway_config(paths, &gateway_command, &plan).await?;
    let gateway_readiness =
        gateway_readiness_check(&plan, gateway_config.readiness_hostname.clone())?;
    start_or_adopt_promoted_runtime(
        paths,
        &supervisor,
        gateway_config.promoted_config,
        gateway_process_spec(paths, &gateway_command),
        gateway_readiness,
        RuntimeSubject::Gateway,
    )
    .await?;
    stop_stale_worker_runtimes(paths, &supervisor, &plan).await?;

    Ok(GATEWAY_RUNTIME_RECONCILED.to_owned())
}

fn gateway_readiness_check(
    plan: &RuntimePlan,
    readiness_hostname: Option<String>,
) -> Result<ReadinessCheck, DaemonError> {
    let ca_certificate_exists = fs::modified_at(&plan.gateway.ca_certificate_path)?.is_some();
    let readiness = match (ca_certificate_exists, readiness_hostname) {
        (true, Some(server_name)) => ReadinessCheck::GatewayHttps {
            http_host: "127.0.0.1".to_owned(),
            http_port: plan.gateway.http_port,
            https_host: "127.0.0.1".to_owned(),
            https_port: plan.gateway.https_port,
            server_name,
            ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        },
        _ => ReadinessCheck::Tcp {
            host: "127.0.0.1".to_owned(),
            port: plan.gateway.http_port,
        },
    };

    Ok(readiness)
}

fn gateway_readiness_hostname(fragments: &[ProjectConfigFragment]) -> Option<String> {
    fragments
        .first()
        .map(|fragment| fragment.primary_hostname.clone())
}

pub async fn validate_config(
    command: &FrankenphpCommand,
    config_path: &Utf8Path,
) -> Result<(), DaemonError> {
    let output = run_validation_command(command, config_path).await?;

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

struct ValidationOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn run_validation_command(
    command: &FrankenphpCommand,
    config_path: &Utf8Path,
) -> Result<ValidationOutput, DaemonError> {
    let mut command_process = FrankenphpProcessCommand::new(command.executable());
    command_process
        .args(command.validate_arguments(config_path))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command_process.process_group(0);

    let mut child = command_process.spawn()?;
    let Some(pid) = child.id() else {
        return Err(DaemonError::MissingProcessId {
            name: "FrankenPHP config validation".to_owned(),
        });
    };
    let stdout = tokio::spawn(read_child_output(child.stdout.take()));
    let stderr = tokio::spawn(read_child_output(child.stderr.take()));
    let status = match timeout(CONFIG_VALIDATION_TIMEOUT, child.wait()).await {
        Ok(result) => result?,
        Err(_elapsed) => {
            terminate_validation_process(pid, &mut child).await;

            return Err(DaemonError::ProtocolTimedOut {
                phase: "FrankenPHP config validation",
            });
        }
    };
    let stdout = stdout.await.map_err(io::Error::other)??;
    let stderr = stderr.await.map_err(io::Error::other)??;

    Ok(ValidationOutput {
        status,
        stdout,
        stderr,
    })
}

async fn read_child_output<Output>(output: Option<Output>) -> io::Result<Vec<u8>>
where
    Output: AsyncRead + Unpin,
{
    let mut content = Vec::new();
    let Some(mut output) = output else {
        return Ok(content);
    };

    output.read_to_end(&mut content).await?;

    Ok(content)
}

async fn terminate_validation_process(pid: u32, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(process_group) = validation_process_group(pid) {
            let _result = kill_process_group(process_group, Signal::KILL);
        }
    }

    let _result = child.kill().await;
    let _result = child.wait().await;
}

#[cfg(unix)]
fn validation_process_group(pid: u32) -> Option<Pid> {
    i32::try_from(pid).ok().and_then(Pid::from_raw)
}

pub fn gateway_process_spec(paths: &PvPaths, command: &FrankenphpCommand) -> ProcessSpec {
    ProcessSpec {
        name: "gateway".to_owned(),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.gateway_root_config()),
        private_environment: Default::default(),
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
        private_environment: Default::default(),
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
        let config_file = match ProjectConfigFile::read_from_root(&project.path) {
            Ok(config_file) => Some(config_file),
            Err(error) => {
                database.record_project_env_observed_snapshot(
                    &project.id,
                    ProjectEnvObservedStatus::Failed,
                    Some(error.to_string().as_str()),
                    &[],
                )?;
                append_persisted_runtime_project(
                    paths,
                    &mut database,
                    &mut projects_by_php_track,
                    project,
                )?;
                continue;
            }
        };
        let config = match config_file {
            Some(config_file) => {
                match validate_project_config_for_gateway(paths, &database, &project, &config_file)
                {
                    Ok(()) => Some(config_file.config),
                    Err(error) => {
                        database.record_project_env_observed_snapshot(
                            &project.id,
                            ProjectEnvObservedStatus::Failed,
                            Some(error.to_string().as_str()),
                            &[],
                        )?;
                        append_persisted_runtime_project(
                            paths,
                            &mut database,
                            &mut projects_by_php_track,
                            project,
                        )?;
                        continue;
                    }
                }
            }
            None => None,
        };
        let config_php = config.as_ref().and_then(|config| config.php.as_deref());
        let stored_php_track = if config_php.is_some() {
            project.desired_php_track.as_deref()
        } else {
            None
        };
        let php_track = resolve_project_php_track(paths, config_php, stored_php_track)?;
        let document_root = match config
            .as_ref()
            .and_then(|config| config.document_root.as_ref())
        {
            Some(document_root) => project.path.join(document_root),
            None => project.path.clone(),
        };
        let runtime_project = RuntimeProject {
            id: project.id,
            render_config: true,
            primary_hostname: project.primary_hostname.clone(),
            hostnames: additional_hostnames(
                &project.primary_hostname,
                project.additional_hostnames,
                config
                    .as_ref()
                    .map(|config| config.hostnames.clone())
                    .unwrap_or_default(),
            ),
            project_root: project.path,
            document_root,
        };

        append_runtime_project(
            &mut database,
            &mut projects_by_php_track,
            php_track,
            runtime_project,
        )?;
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
            storage_path: gateway_storage_path(paths)?,
        },
        workers,
    })
}

fn gateway_storage_path(paths: &PvPaths) -> Result<Utf8PathBuf, DaemonError> {
    let suffix = match fs::read_to_string(&paths.ca_certificate()) {
        Ok(certificate) => {
            let digest = Sha256::digest(certificate.as_bytes());
            format!("{digest:x}")
        }
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            "missing-ca".to_owned()
        }
        Err(error) => return Err(error.into()),
    };

    Ok(paths.certificates().join(format!("caddy-{suffix}")))
}

fn append_persisted_runtime_project(
    paths: &PvPaths,
    database: &mut Database,
    projects_by_php_track: &mut BTreeMap<String, PhpWorkerRuntimePlan>,
    project: state::ProjectRecord,
) -> Result<(), DaemonError> {
    let php_track = resolve_project_php_track(paths, None, project.desired_php_track.as_deref())?;
    let runtime_project = RuntimeProject {
        id: project.id,
        render_config: false,
        primary_hostname: project.primary_hostname.clone(),
        hostnames: additional_hostnames(
            &project.primary_hostname,
            project.additional_hostnames,
            Vec::new(),
        ),
        project_root: project.path.clone(),
        document_root: project.path,
    };

    append_runtime_project(database, projects_by_php_track, php_track, runtime_project)
}

fn append_runtime_project(
    database: &mut Database,
    projects_by_php_track: &mut BTreeMap<String, PhpWorkerRuntimePlan>,
    php_track: String,
    runtime_project: RuntimeProject,
) -> Result<(), DaemonError> {
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

    Ok(())
}

async fn reconcile_gateway_config(
    paths: &PvPaths,
    command: &FrankenphpCommand,
    plan: &RuntimePlan,
) -> Result<GatewayConfigReconciliation, DaemonError> {
    let routes = gateway_project_routes(plan);
    let active_dir = paths.gateway_projects_config_dir();
    let candidate_dir = candidate_config_dir_for(&active_dir);
    let fragments = gateway_project_config_fragments(paths, &routes)?;
    let readiness_hostname = gateway_readiness_hostname(&fragments);
    let import_project_configs = !fragments.is_empty();
    let active_content = match render_gateway_config(&GatewayConfigInput {
        http_port: plan.gateway.http_port,
        https_port: plan.gateway.https_port,
        ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        ca_private_key_path: plan.gateway.ca_private_key_path.clone(),
        storage_path: plan.gateway.storage_path.clone(),
        projects_config_glob: active_dir.join("*.Caddyfile"),
        import_project_configs,
    }) {
        Ok(content) => content,
        Err(error) => {
            record_runtime_error(paths, RuntimeSubject::Gateway, &error)?;

            return Err(error);
        }
    };
    let candidate_content = match render_gateway_config(&GatewayConfigInput {
        http_port: plan.gateway.http_port,
        https_port: plan.gateway.https_port,
        ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        ca_private_key_path: plan.gateway.ca_private_key_path.clone(),
        storage_path: plan.gateway.storage_path.clone(),
        projects_config_glob: candidate_dir.join("*.Caddyfile"),
        import_project_configs,
    }) {
        Ok(content) => content,
        Err(error) => {
            record_runtime_error(paths, RuntimeSubject::Gateway, &error)?;

            return Err(error);
        }
    };
    let result = match write_project_config_fragments(&candidate_dir, &fragments) {
        Ok(()) => {
            promote_runtime_config_tree(
                paths,
                RuntimeSubject::Gateway,
                paths.gateway_root_config(),
                &candidate_content,
                &active_content,
                || promote_config_dir(&active_dir, &candidate_dir),
                command,
            )
            .await
        }
        Err(error) => {
            record_runtime_error(paths, RuntimeSubject::Gateway, &error)?;
            Err(error)
        }
    };
    let _cleanup_result = delete_optional_dir(&candidate_dir);

    result.map(|promoted_config| GatewayConfigReconciliation {
        promoted_config,
        readiness_hostname,
    })
}

struct GatewayConfigReconciliation {
    promoted_config: PromotedConfigTree,
    readiness_hostname: Option<String>,
}

async fn reconcile_worker_config(
    paths: &PvPaths,
    command: &FrankenphpCommand,
    worker: &PhpWorkerRuntimePlan,
) -> Result<PromotedConfigTree, DaemonError> {
    let subject = RuntimeSubject::PhpWorker {
        php_track: worker.php_track.clone(),
    };
    let active_dir = paths.worker_projects_config_dir(&worker.php_track);
    let candidate_dir = candidate_config_dir_for(&active_dir);
    let fragments = worker_project_config_fragments(paths, worker)?;
    let fragment_project_ids = fragments
        .iter()
        .map(|fragment| fragment.project_id.as_str())
        .collect::<BTreeSet<_>>();
    let projects = worker
        .projects
        .iter()
        .filter(|project| fragment_project_ids.contains(project.id.as_str()))
        .map(|project| PhpWorkerProject {
            primary_hostname: project.primary_hostname.clone(),
            hostnames: project.hostnames.clone(),
            project_root: project.project_root.clone(),
            document_root: project.document_root.clone(),
        })
        .collect::<Vec<_>>();
    let active_content = match render_php_worker_config(&PhpWorkerConfigInput {
        php_track: worker.php_track.clone(),
        port: worker.port,
        projects_config_glob: active_dir.join("*.Caddyfile"),
        projects: projects.clone(),
    }) {
        Ok(content) => content,
        Err(error) => {
            record_runtime_error(paths, subject, &error)?;

            return Err(error);
        }
    };
    let candidate_content = match render_php_worker_config(&PhpWorkerConfigInput {
        php_track: worker.php_track.clone(),
        port: worker.port,
        projects_config_glob: candidate_dir.join("*.Caddyfile"),
        projects,
    }) {
        Ok(content) => content,
        Err(error) => {
            record_runtime_error(paths, subject, &error)?;

            return Err(error);
        }
    };
    let result = match write_project_config_fragments(&candidate_dir, &fragments) {
        Ok(()) => {
            promote_runtime_config_tree(
                paths,
                subject,
                paths.worker_root_config(&worker.php_track),
                &candidate_content,
                &active_content,
                || promote_config_dir(&active_dir, &candidate_dir),
                command,
            )
            .await
        }
        Err(error) => {
            record_runtime_error(paths, subject, &error)?;
            Err(error)
        }
    };
    let _cleanup_result = delete_optional_dir(&candidate_dir);

    result
}

async fn promote_runtime_config_tree(
    paths: &PvPaths,
    subject: RuntimeSubject,
    config_path: Utf8PathBuf,
    candidate_content: &str,
    active_content: &str,
    promote_fragments: impl FnOnce() -> Result<PromotedConfigDir, DaemonError>,
    command: &FrankenphpCommand,
) -> Result<PromotedConfigTree, DaemonError> {
    let result = promote_validated_config_tree_async(
        &config_path,
        candidate_content,
        active_content,
        |candidate_path| async move { validate_config(command, &candidate_path).await },
        promote_fragments,
    )
    .await;

    if let Err(error) = &result {
        record_runtime_error(paths, subject, error)?;
    }

    result
}

async fn start_or_adopt_promoted_runtime(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    promoted_config: PromotedConfigTree,
    spec: ProcessSpec,
    readiness: ReadinessCheck,
    subject: RuntimeSubject,
) -> Result<(), DaemonError> {
    let result = start_or_adopt_runtime(paths, supervisor, spec, readiness, subject.clone()).await;

    match result {
        Ok(()) => {
            if let Err(error) = promoted_config.commit() {
                record_runtime_error(paths, subject, &error)?;

                return Err(error);
            }

            Ok(())
        }
        Err(error) => {
            if let Err(rollback_error) = promoted_config.rollback() {
                let error = runtime_config_rollback_failed_error(error, rollback_error);
                record_runtime_error(paths, subject, &error)?;

                return Err(error);
            }

            Err(error)
        }
    }
}

async fn start_or_adopt_runtime(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    spec: ProcessSpec,
    readiness: ReadinessCheck,
    subject: RuntimeSubject,
) -> Result<(), DaemonError> {
    let result = async {
        if supervisor.adopt(&spec)?.is_some() {
            if supervisor.reload(&spec)? {
                wait_for_readiness(readiness, RUNTIME_READINESS_TIMEOUT).await?;

                return Ok(());
            }

            return Err(DaemonError::UnexpectedProtocolResponse {
                reason: format!(
                    "runtime `{}` could not be reloaded because PV ownership changed",
                    spec.name
                ),
            });
        } else if probe_readiness_once(&readiness).await.is_ok() {
            return Err(DaemonError::UnexpectedProtocolResponse {
                reason: format!(
                    "runtime `{}` is listening but no PV-owned process could be verified",
                    spec.name
                ),
            });
        }

        let mut process = supervisor.start(spec.clone()).await?;
        if let Err(error) = wait_for_readiness(readiness, RUNTIME_READINESS_TIMEOUT).await {
            process.stop(Duration::from_secs(1)).await?;

            return Err(error);
        }

        if process.has_exited()? {
            return Err(DaemonError::UnexpectedProtocolResponse {
                reason: format!(
                    "runtime `{}` exited before readiness was verified",
                    spec.name
                ),
            });
        }

        Ok(())
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
                id: project.id.clone(),
                render_config: project.render_config,
                primary_hostname: project.primary_hostname.clone(),
                hostnames: project.hostnames.clone(),
                worker_port: worker.port,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectConfigFragment {
    project_id: String,
    file_name: String,
    primary_hostname: String,
    content: String,
}

fn gateway_project_config_fragments(
    paths: &PvPaths,
    routes: &[GatewayProjectRoute],
) -> Result<Vec<ProjectConfigFragment>, DaemonError> {
    let active_dir = paths.gateway_projects_config_dir();
    let mut fragments = Vec::new();

    for route in routes {
        let file_name = project_config_file_name(&route.id);
        let content = if route.render_config {
            Some(render_gateway_project_config(route)?)
        } else {
            read_preserved_project_config_fragment(&active_dir, &file_name)?
        };
        let Some(content) = content else {
            continue;
        };

        fragments.push(ProjectConfigFragment {
            project_id: route.id.clone(),
            file_name,
            primary_hostname: route.primary_hostname.clone(),
            content,
        });
    }

    Ok(fragments)
}

fn worker_project_config_fragments(
    paths: &PvPaths,
    worker: &PhpWorkerRuntimePlan,
) -> Result<Vec<ProjectConfigFragment>, DaemonError> {
    let active_dir = paths.worker_projects_config_dir(&worker.php_track);
    let mut fragments = Vec::new();

    for project in &worker.projects {
        let file_name = project_config_file_name(&project.id);
        let content = if project.render_config {
            let input = PhpWorkerProject {
                primary_hostname: project.primary_hostname.clone(),
                hostnames: project.hostnames.clone(),
                project_root: project.project_root.clone(),
                document_root: project.document_root.clone(),
            };

            Some(render_php_worker_project_config(&input, worker.port)?)
        } else {
            read_preserved_project_config_fragment(&active_dir, &file_name)?
        };
        let Some(content) = content else {
            continue;
        };

        fragments.push(ProjectConfigFragment {
            project_id: project.id.clone(),
            file_name,
            primary_hostname: project.primary_hostname.clone(),
            content,
        });
    }

    Ok(fragments)
}

fn write_project_config_fragments(
    directory: &Utf8Path,
    fragments: &[ProjectConfigFragment],
) -> Result<(), DaemonError> {
    if fragments.is_empty() {
        let marker_path = directory.join(".pv-empty");
        fs::write_sensitive_file(&marker_path, "")?;
        delete_optional_file(&marker_path)?;

        return Ok(());
    }

    for fragment in fragments {
        fs::write_sensitive_file(&directory.join(&fragment.file_name), &fragment.content)?;
    }

    Ok(())
}

async fn stop_stale_worker_runtimes(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    plan: &RuntimePlan,
) -> Result<(), DaemonError> {
    let desired_tracks = plan
        .workers
        .iter()
        .map(|worker| worker.php_track.as_str())
        .collect::<BTreeSet<_>>();

    for php_track in runtime_worker_tracks(paths)? {
        if desired_tracks.contains(php_track.as_str()) {
            continue;
        }
        let subject = RuntimeSubject::PhpWorker {
            php_track: php_track.clone(),
        };

        if let Some(adopted) = supervisor.adopt_recorded(
            &paths.worker_pid(&php_track),
            &paths.worker_runtime_metadata(&php_track),
        )? {
            adopted.stop(Duration::from_secs(1)).await?;
        }
        record_runtime_observed(
            paths,
            subject,
            RuntimeObservedStatus::Stopped,
            Some("PHP worker stopped; no Projects remain on this track"),
        )?;
        cleanup_stale_worker_runtime(paths, &php_track)?;
    }

    Ok(())
}

fn cleanup_stale_worker_runtime(paths: &PvPaths, php_track: &str) -> Result<(), DaemonError> {
    delete_optional_file(&paths.worker_pid(php_track))?;
    delete_optional_file(&paths.worker_runtime_metadata(php_track))?;
    delete_optional_file(&paths.worker_root_config(php_track))?;
    delete_optional_dir(&paths.worker_projects_config_dir(php_track))?;

    let mut database = Database::open(paths)?;
    database.release_port(PortOwner::PhpWorker {
        php_track: php_track.to_owned(),
    })?;

    Ok(())
}

fn project_config_file_name(project_id: &str) -> String {
    format!("{project_id}.Caddyfile")
}

fn read_preserved_project_config_fragment(
    directory: &Utf8Path,
    file_name: &str,
) -> Result<Option<String>, DaemonError> {
    match fs::read_to_string(&directory.join(file_name)) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn candidate_config_dir_for(directory: &Utf8Path) -> Utf8PathBuf {
    let file_name = directory.file_name().unwrap_or("projects");
    let process_id = std::process::id();
    let counter = CANDIDATE_CONFIG_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);

    directory.with_file_name(format!("{file_name}.candidate.{process_id}.{counter}.tmp"))
}

fn runtime_worker_tracks(paths: &PvPaths) -> Result<Vec<String>, DaemonError> {
    let mut tracks = Vec::new();

    for path in read_directory_files(&paths.run().join("workers"))? {
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let Some(track) = file_name
            .strip_prefix("php-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };

        tracks.push(track.to_string());
    }

    Ok(tracks)
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon Gateway reconciliation prunes generated Caddyfile fragments"
)]
fn read_directory_files(directory: &Utf8Path) -> Result<Vec<Utf8PathBuf>, DaemonError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut paths = Vec::new();

    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|path| {
            DaemonError::UnexpectedProtocolResponse {
                reason: format!("generated Gateway config path is not UTF-8: {path:?}"),
            }
        })?;
        paths.push(path);
    }

    Ok(paths)
}

fn delete_optional_file(path: &Utf8Path) -> Result<(), DaemonError> {
    match fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn runtime_config_rollback_failed_error(
    original: DaemonError,
    rollback: DaemonError,
) -> DaemonError {
    DaemonError::UnexpectedProtocolResponse {
        reason: format!(
            "Gateway runtime config rollback failed after runtime reconciliation failed: {original}; rollback failed: {rollback}"
        ),
    }
}

fn delete_optional_dir(path: &Utf8Path) -> Result<(), DaemonError> {
    match fs::delete_dir_all(path) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use camino_tempfile::tempdir;
    use state::PvPaths;

    use crate::gateway_config::GatewayProjectRoute;

    use super::{
        gateway_project_config_fragments, gateway_readiness_hostname, project_config_file_name,
    };

    #[test]
    fn gateway_readiness_hostname_uses_imported_project_fragments() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_id = "project-1";
        let preserved_content = "preserved.test {\n    respond 200\n}\n";
        let fragment_path = paths
            .gateway_projects_config_dir()
            .join(project_config_file_name(project_id));
        state::fs::write_sensitive_file(&fragment_path, preserved_content)?;

        let fragments = gateway_project_config_fragments(
            &paths,
            &[GatewayProjectRoute {
                id: project_id.to_owned(),
                render_config: false,
                primary_hostname: "preserved.test".to_owned(),
                hostnames: Vec::new(),
                worker_port: 8123,
            }],
        )?;

        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].content, preserved_content);
        assert_eq!(
            gateway_readiness_hostname(&fragments).as_deref(),
            Some("preserved.test")
        );

        Ok(())
    }
}
