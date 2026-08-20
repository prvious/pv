use std::collections::{BTreeMap, BTreeSet, btree_map};
use std::io;
use std::net::TcpListener;
use std::process::{ExitStatus, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use config::{ProjectConfig, ProjectConfigFile};
use resources::{ResourceAdapter, caddy_adapter, frankenphp_adapter};
#[cfg(target_os = "macos")]
use rustix::process::{Pid, Signal, kill_process_group};
use sha2::{Digest, Sha256};
use state::{
    Database, GatewayPort, ManagedResourceDesiredState, ManagedResourceTrackRecord, PortOwner,
    ProjectEnvObservedStatus, ProjectMode, PvPaths, RuntimeObservedStatus, RuntimeSubject,
    StateError, fs,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::{sleep, timeout};

use crate::gateway_config::{
    GATEWAY_HEALTH_HOSTNAME, GATEWAY_HEALTH_PATH, GatewayConfigInput, GatewayProjectRoute,
    PhpWorkerConfigInput, PhpWorkerProject, PromotedConfigDir, PromotedConfigTree,
    gateway_health_response, promote_config_dir, promote_validated_config_tree_async,
    render_gateway_config, render_gateway_project_config, render_php_worker_config,
    render_php_worker_project_config,
};
use crate::project_env::{
    ResolvedPhpRuntime, resolve_project_php_runtime, validate_project_config_for_gateway,
};
use crate::structured_log;
use crate::supervisor::{ManagedProcess, probe_readiness_once};
use crate::{
    CaddyAdminClient, CaddyAdminEndpoint, CaddyAdminError, CaddyAdminOperation, CaddyAdminVerifier,
    DaemonError, ProcessSpec, ProcessSupervisor, ReadinessCheck, wait_for_readiness,
};

#[expect(
    clippy::disallowed_types,
    reason = "daemon runtime owns managed Caddy CLI config validation process execution"
)]
type RuntimeProcessCommand = tokio::process::Command;

const PHP_INI_ENVIRONMENT_KEYS: [&str; 2] = ["PHPRC", "PHP_INI_SCAN_DIR"];
const CONFIG_VALIDATION_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_READINESS_TIMEOUT: Duration = Duration::from_secs(60);
const PF_PUBLIC_READINESS_TIMEOUT: Duration = Duration::from_secs(2);
const FOREIGN_LISTENER_PROBE_TIMEOUT: Duration = Duration::from_millis(100);
const OWNED_READINESS_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const OWNED_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const PUBLIC_HTTP_PORT: u16 = 80;
const PUBLIC_HTTPS_PORT: u16 = 443;
const GATEWAY_RUNTIME_RECONCILED: &str = "Gateway runtime reconciled";
pub(crate) const CADDY_NOT_INSTALLED: &str = "Gateway runtime skipped; Caddy is not installed";
static CANDIDATE_CONFIG_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaddyCliCommand {
    executable: Utf8PathBuf,
    runtime_label: RuntimeLabel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeLabel {
    Caddy,
    FrankenPhp,
}

impl RuntimeLabel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Caddy => "Caddy",
            Self::FrankenPhp => "FrankenPHP",
        }
    }
}

impl CaddyCliCommand {
    pub fn caddy(executable: impl Into<Utf8PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            runtime_label: RuntimeLabel::Caddy,
        }
    }

    pub fn frankenphp(executable: impl Into<Utf8PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            runtime_label: RuntimeLabel::FrankenPhp,
        }
    }

    pub fn executable(&self) -> &Utf8Path {
        &self.executable
    }

    pub fn validate_arguments(&self, config_path: &Utf8Path) -> Vec<String> {
        caddyfile_arguments("validate", config_path)
    }

    pub fn run_arguments(&self, config_path: &Utf8Path) -> Vec<String> {
        caddyfile_arguments("run", config_path)
    }

    fn runtime_label(&self) -> &'static str {
        self.runtime_label.as_str()
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
    pub admin_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
    pub storage_path: Utf8PathBuf,
}

#[doc(hidden)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GatewayPfRoutingState {
    Active,
    Inactive,
    Drifted,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerRuntimePlan {
    pub php_track: String,
    pub runtime_key: String,
    pub loaded_modules: Vec<resources::PhpExtensionModule>,
    pub port: u16,
    pub admin_port: u16,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct InstalledFrankenphpRuntime {
    command: CaddyCliCommand,
    artifact_root: Utf8PathBuf,
}

pub fn promote_validated_config_for_test(
    path: &Utf8Path,
    content: &str,
    validate: impl FnOnce(&Utf8Path) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    crate::gateway_config::promote_validated_config(path, content, validate)
}

pub async fn reconcile_gateway_runtimes(paths: &PvPaths) -> Result<String, DaemonError> {
    reconcile_gateway_runtimes_with_readiness_timeout(paths, RUNTIME_READINESS_TIMEOUT).await
}

pub fn probe_gateway_identity_blocking(
    expected: &platform::PfRedirectConfig,
    ca_certificate_path: &Utf8Path,
) -> Result<(), DaemonError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let check = gateway_identity_readiness_check(
        expected.http_port,
        expected.https_port,
        PUBLIC_HTTP_PORT,
        PUBLIC_HTTPS_PORT,
        ca_certificate_path,
    );

    runtime.block_on(wait_for_readiness(check, Duration::from_secs(1)))
}

#[doc(hidden)]
pub async fn reconcile_gateway_runtimes_with_readiness_timeout(
    paths: &PvPaths,
    readiness_timeout: Duration,
) -> Result<String, DaemonError> {
    reconcile_gateway_runtimes_with_pf_state(paths, readiness_timeout, None).await
}

#[doc(hidden)]
pub async fn reconcile_gateway_runtimes_with_pf_state_for_test(
    paths: &PvPaths,
    readiness_timeout: Duration,
    pf_routing_state: GatewayPfRoutingState,
) -> Result<String, DaemonError> {
    reconcile_gateway_runtimes_with_pf_state(paths, readiness_timeout, Some(pf_routing_state)).await
}

async fn reconcile_gateway_runtimes_with_pf_state(
    paths: &PvPaths,
    readiness_timeout: Duration,
    pf_routing_state: Option<GatewayPfRoutingState>,
) -> Result<String, DaemonError> {
    let Some(gateway_command) = first_installed_caddy_command(paths)? else {
        record_runtime_observed(
            paths,
            RuntimeSubject::Gateway,
            RuntimeObservedStatus::Stopped,
            Some(CADDY_NOT_INSTALLED),
        )?;

        return Ok(CADDY_NOT_INSTALLED.to_owned());
    };

    let supervisor = ProcessSupervisor::new(paths.clone());
    if let Err((subject, error)) = recover_unowned_admin_port_assignments(paths, &supervisor) {
        record_runtime_error(paths, subject, &error)?;

        return Err(*error);
    }
    let plan = match build_runtime_plan(paths) {
        Ok(plan) => plan,
        Err(error) => {
            record_runtime_error(paths, RuntimeSubject::Gateway, &error)?;

            return Err(error);
        }
    };
    let mut worker_commands = Vec::new();

    for worker in &plan.workers {
        let subject = worker_runtime_subject(worker);
        let worker_runtime = match installed_frankenphp_runtime_for_track(paths, &worker.php_track)
        {
            Ok(Some(runtime)) => runtime,
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
        worker_commands.push((worker, worker_runtime));
    }

    for (worker, worker_runtime) in worker_commands {
        let subject = worker_runtime_subject(worker);
        let process_spec = match worker_process_spec(
            paths,
            worker,
            &worker_runtime.command,
            &worker_runtime.artifact_root,
        ) {
            Ok(process_spec) => process_spec,
            Err(error) => {
                record_runtime_error(paths, subject.clone(), &error)?;

                return Err(error);
            }
        };
        let promoted_config = reconcile_worker_config(
            paths,
            &worker_runtime.command,
            &worker_runtime.artifact_root,
            worker,
        )
        .await?;
        start_or_adopt_promoted_runtime(
            paths,
            &supervisor,
            promoted_config,
            process_spec,
            RuntimeReadinessPlan {
                check: ReadinessCheck::Tcp {
                    host: "127.0.0.1".to_owned(),
                    port: worker.port,
                },
                failure_policy: ReadinessFailurePolicy::FailRuntime,
                timeout: readiness_timeout,
                admin_endpoint: CaddyAdminEndpoint::new(worker.admin_port),
            },
            subject.clone(),
        )
        .await?;
        record_runtime_observed(
            paths,
            subject,
            RuntimeObservedStatus::Running,
            Some(GATEWAY_RUNTIME_RECONCILED),
        )?;
    }
    let gateway_config = reconcile_gateway_config(paths, &gateway_command, &plan).await?;
    let pf_routing_state =
        pf_routing_state.unwrap_or_else(|| gateway_pf_routing_state(paths, &plan));
    let gateway_readiness = gateway_readiness_plan(
        &plan,
        gateway_config.readiness_hostname.clone(),
        pf_routing_state,
        readiness_timeout,
    );
    let readiness_outcome = start_or_adopt_promoted_runtime(
        paths,
        &supervisor,
        gateway_config.promoted_config,
        gateway_process_spec(paths, &gateway_command),
        gateway_readiness,
        RuntimeSubject::Gateway,
    )
    .await?;
    record_gateway_runtime_observed(paths, pf_routing_state, readiness_outcome)?;
    stop_stale_worker_runtimes(paths, &supervisor, &plan).await?;

    Ok(GATEWAY_RUNTIME_RECONCILED.to_owned())
}

fn gateway_readiness_plan(
    plan: &RuntimePlan,
    readiness_hostname: Option<String>,
    pf_routing_state: GatewayPfRoutingState,
    readiness_timeout: Duration,
) -> RuntimeReadinessPlan {
    let ports = gateway_readiness_ports(plan, pf_routing_state);
    let failure_policy = match pf_routing_state {
        GatewayPfRoutingState::Active | GatewayPfRoutingState::Inactive => {
            ReadinessFailurePolicy::FailRuntime
        }
        GatewayPfRoutingState::Drifted | GatewayPfRoutingState::Unknown => {
            ReadinessFailurePolicy::PreserveRuntime
        }
    };
    let timeout = match failure_policy {
        ReadinessFailurePolicy::FailRuntime => readiness_timeout,
        ReadinessFailurePolicy::PreserveRuntime => {
            readiness_timeout.min(PF_PUBLIC_READINESS_TIMEOUT)
        }
    };

    let check = if pf_routing_state == GatewayPfRoutingState::Inactive {
        gateway_readiness_check_for_ports(plan, readiness_hostname, ports)
    } else {
        gateway_public_readiness_check(plan, ports)
    };

    RuntimeReadinessPlan {
        check,
        failure_policy,
        timeout,
        admin_endpoint: CaddyAdminEndpoint::new(plan.gateway.admin_port),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeReadinessPlan {
    check: ReadinessCheck,
    failure_policy: ReadinessFailurePolicy,
    timeout: Duration,
    admin_endpoint: CaddyAdminEndpoint,
}

fn previous_runtime_readiness(
    promoted_config: &PromotedConfigTree,
    readiness: &RuntimeReadinessPlan,
) -> Result<RuntimeReadinessPlan, DaemonError> {
    let Some(previous_root_content) = promoted_config.previous_root_content() else {
        return Ok(readiness.clone());
    };
    previous_runtime_readiness_from_parts(
        previous_root_content,
        promoted_config.previous_fragment_contents(),
        readiness,
    )
}

fn previous_runtime_readiness_from_parts(
    previous_root_content: &str,
    previous_fragment_contents: &[String],
    readiness: &RuntimeReadinessPlan,
) -> Result<RuntimeReadinessPlan, DaemonError> {
    let http_port = match optional_config_port(previous_root_content, "http_port ")? {
        Some(port) => Some(port),
        None => previous_fragment_contents
            .iter()
            .find_map(|fragment| fragment_port(fragment))
            .transpose()?,
    };
    let https_port = optional_config_port(previous_root_content, "https_port ")?;
    let admin_port = previous_admin_port(previous_root_content)?;
    let check = previous_readiness_check(&readiness.check, http_port, https_port)?;

    Ok(RuntimeReadinessPlan {
        check,
        admin_endpoint: CaddyAdminEndpoint::new(admin_port),
        ..readiness.clone()
    })
}

fn previous_readiness_check(
    check: &ReadinessCheck,
    http_port: Option<u16>,
    https_port: Option<u16>,
) -> Result<ReadinessCheck, DaemonError> {
    match check {
        ReadinessCheck::Tcp { host, .. } => Ok(ReadinessCheck::Tcp {
            host: host.clone(),
            port: http_port.ok_or_else(missing_previous_service_port)?,
        }),
        ReadinessCheck::GatewayHttps {
            http_host,
            https_host,
            server_name,
            ca_certificate_path,
            ..
        } => {
            let Some(https_port) = https_port else {
                return Err(DaemonError::UnexpectedProtocolResponse {
                    reason: "previous gateway config is missing https_port".to_owned(),
                });
            };
            let Some(http_port) = http_port else {
                return Err(missing_previous_service_port());
            };

            Ok(ReadinessCheck::GatewayHttps {
                http_host: http_host.clone(),
                http_port,
                https_host: https_host.clone(),
                https_port,
                server_name: server_name.clone(),
                ca_certificate_path: ca_certificate_path.clone(),
            })
        }
        ReadinessCheck::GatewayIdentity {
            http_host,
            http_port: probe_http_port,
            https_host,
            https_port: probe_https_port,
            server_name,
            path,
            ca_certificate_path,
            ..
        } => {
            let Some(https_port) = https_port else {
                return Err(DaemonError::UnexpectedProtocolResponse {
                    reason: "previous gateway config is missing https_port".to_owned(),
                });
            };
            let Some(http_port) = http_port else {
                return Err(missing_previous_service_port());
            };

            Ok(ReadinessCheck::GatewayIdentity {
                http_host: http_host.clone(),
                http_port: *probe_http_port,
                https_host: https_host.clone(),
                https_port: *probe_https_port,
                server_name: server_name.clone(),
                path: path.clone(),
                expected_body: gateway_health_response(http_port, https_port),
                ca_certificate_path: ca_certificate_path.clone(),
            })
        }
        _ => Ok(check.clone()),
    }
}

fn optional_config_port(config: &str, prefix: &str) -> Result<Option<u16>, DaemonError> {
    let Some(port) = config
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(prefix))
    else {
        return Ok(None);
    };
    port.trim()
        .parse()
        .map(Some)
        .map_err(|error| DaemonError::UnexpectedProtocolResponse {
            reason: format!("previous runtime config has invalid port `{port}`: {error}"),
        })
}

fn previous_admin_port(config: &str) -> Result<u16, DaemonError> {
    optional_config_port(config, "admin 127.0.0.1:")?.ok_or_else(|| {
        DaemonError::UnexpectedProtocolResponse {
            reason: "previous runtime config is missing an enabled admin port".to_owned(),
        }
    })
}

fn fragment_port(fragment: &str) -> Option<Result<u16, DaemonError>> {
    fragment.split_whitespace().find_map(|token| {
        let token = token.trim_end_matches(',');
        let token = token.strip_prefix("http://")?;
        let (_host, port) = token.rsplit_once(':')?;
        Some(
            port.parse()
                .map_err(|error| DaemonError::UnexpectedProtocolResponse {
                    reason: format!("previous worker fragment has invalid port `{port}`: {error}"),
                }),
        )
    })
}

fn missing_previous_service_port() -> DaemonError {
    DaemonError::UnexpectedProtocolResponse {
        reason: "previous runtime config is missing a service port".to_owned(),
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct GatewayReadinessPorts {
    http: u16,
    https: u16,
}

fn gateway_readiness_ports(
    plan: &RuntimePlan,
    pf_routing_state: GatewayPfRoutingState,
) -> GatewayReadinessPorts {
    // macOS pf can make direct connections to an active rdr target port hang.
    if pf_routing_state != GatewayPfRoutingState::Inactive {
        return GatewayReadinessPorts {
            http: PUBLIC_HTTP_PORT,
            https: PUBLIC_HTTPS_PORT,
        };
    }

    GatewayReadinessPorts {
        http: plan.gateway.http_port,
        https: plan.gateway.https_port,
    }
}

fn gateway_pf_routing_state(paths: &PvPaths, plan: &RuntimePlan) -> GatewayPfRoutingState {
    let expected = platform::PfRedirectConfig::new(plan.gateway.http_port, plan.gateway.https_port);
    let files_current = pf_files_current(paths, &expected);

    match platform::inspect_active_pf_redirects_unprivileged() {
        Ok(inspection) => {
            classify_gateway_pf_routing_state(&expected, Some(&inspection), files_current)
        }
        Err(_error) => classify_gateway_pf_routing_state(&expected, None, files_current),
    }
}

fn classify_gateway_pf_routing_state(
    expected: &platform::PfRedirectConfig,
    inspection: Option<&platform::ActivePfRedirectInspection>,
    files_current: bool,
) -> GatewayPfRoutingState {
    let Some(inspection) = inspection else {
        return if files_current {
            GatewayPfRoutingState::Unknown
        } else {
            GatewayPfRoutingState::Drifted
        };
    };

    match inspection.pv_config.as_ref() {
        Some(active) if active == expected && inspection.pf_enabled && files_current => {
            GatewayPfRoutingState::Active
        }
        Some(active) if active == expected => GatewayPfRoutingState::Drifted,
        Some(_active) => GatewayPfRoutingState::Drifted,
        None if inspection.pv_anchor_has_unparsed_rules
            || inspection.has_unresolved_redirect_targets
            || inspection
                .resolved_target_ports
                .contains(&expected.http_port)
            || inspection
                .resolved_target_ports
                .contains(&expected.https_port) =>
        {
            GatewayPfRoutingState::Drifted
        }
        None => GatewayPfRoutingState::Inactive,
    }
}

fn pf_files_current(paths: &PvPaths, expected: &platform::PfRedirectConfig) -> bool {
    let prepared_anchor =
        platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), Some(expected));
    let prepared_reference = platform::inspect_pf_conf_reference(
        &paths.pf_conf_reference_config(),
        Some(&platform::PfConfReference),
    );
    let system_anchor = platform::inspect_pf_anchor_file(
        Utf8Path::new(platform::SYSTEM_PF_ANCHOR_PATH),
        Some(expected),
    );
    let system_reference = platform::inspect_pf_conf_reference(
        Utf8Path::new(platform::SYSTEM_PF_CONF_PATH),
        Some(&platform::PfConfReference),
    );

    matches!(prepared_anchor, platform::PfFileState::Current { .. })
        && matches!(prepared_reference, platform::PfFileState::Current { .. })
        && matches!(system_anchor, platform::PfFileState::Current { .. })
        && matches!(system_reference, platform::PfFileState::Current { .. })
}

fn gateway_readiness_check_for_ports(
    plan: &RuntimePlan,
    readiness_hostname: Option<String>,
    ports: GatewayReadinessPorts,
) -> ReadinessCheck {
    match readiness_hostname {
        Some(server_name) => ReadinessCheck::GatewayHttps {
            http_host: "127.0.0.1".to_owned(),
            http_port: ports.http,
            https_host: "127.0.0.1".to_owned(),
            https_port: ports.https,
            server_name,
            ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        },
        None => ReadinessCheck::Tcp {
            host: "127.0.0.1".to_owned(),
            port: ports.http,
        },
    }
}

fn gateway_public_readiness_check(
    plan: &RuntimePlan,
    ports: GatewayReadinessPorts,
) -> ReadinessCheck {
    gateway_identity_readiness_check(
        plan.gateway.http_port,
        plan.gateway.https_port,
        ports.http,
        ports.https,
        &plan.gateway.ca_certificate_path,
    )
}

fn gateway_identity_readiness_check(
    expected_http_port: u16,
    expected_https_port: u16,
    probe_http_port: u16,
    probe_https_port: u16,
    ca_certificate_path: &Utf8Path,
) -> ReadinessCheck {
    ReadinessCheck::GatewayIdentity {
        http_host: "127.0.0.1".to_owned(),
        http_port: probe_http_port,
        https_host: "127.0.0.1".to_owned(),
        https_port: probe_https_port,
        server_name: GATEWAY_HEALTH_HOSTNAME.to_owned(),
        path: GATEWAY_HEALTH_PATH.to_owned(),
        expected_body: gateway_health_response(expected_http_port, expected_https_port),
        ca_certificate_path: ca_certificate_path.to_path_buf(),
    }
}

fn gateway_readiness_hostname(fragments: &[ProjectConfigFragment]) -> Option<String> {
    fragments
        .first()
        .map(|fragment| fragment.primary_hostname.clone())
}

pub async fn validate_config(
    command: &CaddyCliCommand,
    config_path: &Utf8Path,
    private_environment: &BTreeMap<String, String>,
) -> Result<(), DaemonError> {
    let output = run_validation_command(command, config_path, private_environment).await?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Err(DaemonError::UnexpectedProtocolResponse {
        reason: format!(
            "{runtime} config validation failed for {config_path}: status={status}; stdout={stdout}; stderr={stderr}",
            runtime = command.runtime_label(),
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
    command: &CaddyCliCommand,
    config_path: &Utf8Path,
    private_environment: &BTreeMap<String, String>,
) -> Result<ValidationOutput, DaemonError> {
    let mut command_process = RuntimeProcessCommand::new(command.executable());
    command_process
        .args(command.validate_arguments(config_path))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in PHP_INI_ENVIRONMENT_KEYS {
        command_process.env_remove(key);
    }
    command_process.envs(private_environment);
    #[cfg(target_os = "macos")]
    command_process.process_group(0);

    let mut child = command_process.spawn()?;
    let Some(pid) = child.id() else {
        return Err(DaemonError::MissingProcessId {
            name: format!("{} config validation", command.runtime_label()),
        });
    };
    let stdout = tokio::spawn(read_child_output(child.stdout.take()));
    let stderr = tokio::spawn(read_child_output(child.stderr.take()));
    let status = match timeout(CONFIG_VALIDATION_TIMEOUT, child.wait()).await {
        Ok(result) => result?,
        Err(_elapsed) => {
            terminate_validation_process(pid, &mut child).await;

            return Err(DaemonError::ProtocolTimedOut {
                phase: match command.runtime_label() {
                    "Caddy" => "Caddy config validation",
                    "FrankenPHP" => "FrankenPHP config validation",
                    _ => "runtime config validation",
                },
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
    #[cfg(not(target_os = "macos"))]
    let _ = pid;

    #[cfg(target_os = "macos")]
    {
        if let Some(process_group) = validation_process_group(pid) {
            let _result = kill_process_group(process_group, Signal::KILL);
        }
    }

    let _result = child.kill().await;
    let _result = child.wait().await;
}

#[cfg(target_os = "macos")]
fn validation_process_group(pid: u32) -> Option<Pid> {
    i32::try_from(pid).ok().and_then(Pid::from_raw)
}

pub fn gateway_process_spec(paths: &PvPaths, command: &CaddyCliCommand) -> ProcessSpec {
    ProcessSpec {
        name: "gateway".to_owned(),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.gateway_root_config()),
        private_environment: caddy_xdg_environment(paths),
        config_path: paths.gateway_root_config(),
        log_path: paths.gateway_error_log(),
        pid_path: paths.gateway_pid(),
        metadata_path: paths.gateway_runtime_metadata(),
        resource_name: "caddy".to_owned(),
        track: "2".to_owned(),
    }
}

pub fn worker_process_spec(
    paths: &PvPaths,
    worker: &PhpWorkerRuntimePlan,
    command: &CaddyCliCommand,
    artifact_root: &Utf8Path,
) -> Result<ProcessSpec, DaemonError> {
    Ok(ProcessSpec {
        name: format!("php-worker-{}", worker.runtime_key),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.worker_root_config(&worker.runtime_key)),
        private_environment: frankenphp_worker_environment(paths, worker, artifact_root)?,
        config_path: paths.worker_root_config(&worker.runtime_key),
        log_path: paths.worker_log(&worker.runtime_key),
        pid_path: paths.worker_pid(&worker.runtime_key),
        metadata_path: paths.worker_runtime_metadata(&worker.runtime_key),
        resource_name: "frankenphp".to_owned(),
        track: worker.runtime_key.clone(),
    })
}

pub fn build_runtime_plan(paths: &PvPaths) -> Result<RuntimePlan, DaemonError> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(local_loopback_port_available)?;
    let mut projects_by_runtime_key: BTreeMap<String, PhpWorkerRuntimePlan> = BTreeMap::new();

    for project in database.projects()? {
        if project.mode == ProjectMode::ResourceOnly {
            continue;
        }

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
                    &mut database,
                    &mut projects_by_runtime_key,
                    project,
                )?;
                continue;
            }
        };
        let config = match config_file {
            Some(config_file) => {
                if config_file.config.serve != (project.mode == ProjectMode::Served) {
                    append_persisted_runtime_project(
                        &mut database,
                        &mut projects_by_runtime_key,
                        project,
                    )?;
                    continue;
                }
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
                            &mut database,
                            &mut projects_by_runtime_key,
                            project,
                        )?;
                        continue;
                    }
                }
            }
            None => None,
        };
        let primary_hostname =
            project
                .primary_hostname
                .clone()
                .ok_or_else(|| StateError::ProjectNotServed {
                    project_id: project.id.clone(),
                })?;
        let runtime = resolve_project_php_runtime(
            paths,
            &database,
            &project,
            config.as_ref().and_then(|config| config.php.as_ref()),
        )?;
        let document_root = resolve_project_document_root(&project.path, config.as_ref())?;
        let runtime_project = RuntimeProject {
            id: project.id,
            render_config: true,
            primary_hostname: primary_hostname.clone(),
            hostnames: additional_hostnames(
                &primary_hostname,
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
            &mut projects_by_runtime_key,
            runtime,
            runtime_project,
        )?;
    }

    let workers = projects_by_runtime_key
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
            admin_port: gateway_ports.admin.port,
            ca_certificate_path: paths.ca_certificate(),
            ca_private_key_path: paths.ca_private_key(),
            storage_path: gateway_storage_path(paths)?,
        },
        workers,
    })
}

fn resolve_project_document_root(
    project_root: &Utf8Path,
    config: Option<&ProjectConfig>,
) -> Result<Utf8PathBuf, DaemonError> {
    if let Some(document_root) = config.and_then(|config| config.document_root.as_ref()) {
        return Ok(project_root.join(document_root));
    }

    let public_root = project_root.join("public");
    if fs::path_is_directory(&public_root)? {
        Ok(public_root)
    } else {
        Ok(project_root.to_path_buf())
    }
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
    database: &mut Database,
    projects_by_runtime_key: &mut BTreeMap<String, PhpWorkerRuntimePlan>,
    project: state::ProjectRecord,
) -> Result<(), DaemonError> {
    if project.mode == ProjectMode::ResourceOnly {
        return Ok(());
    }
    let primary_hostname =
        project
            .primary_hostname
            .clone()
            .ok_or_else(|| StateError::ProjectNotServed {
                project_id: project.id.clone(),
            })?;
    let runtime = match persisted_project_php_runtime(database, &project) {
        Ok(Some(runtime)) => runtime,
        Ok(None) => return Ok(()),
        Err(
            error @ DaemonError::Resources(resources::ResourcesError::InvalidArtifactLayout {
                ..
            }),
        ) => {
            let message = error.to_string();
            database.record_project_env_observed_snapshot(
                &project.id,
                ProjectEnvObservedStatus::Failed,
                Some(&message),
                &[],
            )?;

            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let runtime_project = RuntimeProject {
        id: project.id,
        render_config: false,
        primary_hostname: primary_hostname.clone(),
        hostnames: additional_hostnames(
            &primary_hostname,
            project.additional_hostnames,
            Vec::new(),
        ),
        project_root: project.path.clone(),
        document_root: project.path,
    };

    append_runtime_project(database, projects_by_runtime_key, runtime, runtime_project)
}

fn append_runtime_project(
    database: &mut Database,
    projects_by_runtime_key: &mut BTreeMap<String, PhpWorkerRuntimePlan>,
    runtime: ResolvedPhpRuntime,
    runtime_project: RuntimeProject,
) -> Result<(), DaemonError> {
    match projects_by_runtime_key.entry(runtime.runtime_key.clone()) {
        btree_map::Entry::Occupied(mut entry) => {
            entry.get_mut().projects.push(runtime_project);
        }
        btree_map::Entry::Vacant(entry) => {
            let assignments = database
                .assign_php_worker_ports(&runtime.runtime_key, local_loopback_port_available)?;

            entry.insert(PhpWorkerRuntimePlan {
                php_track: runtime.track,
                runtime_key: runtime.runtime_key,
                loaded_modules: runtime.loaded_modules,
                port: assignments.service.port,
                admin_port: assignments.admin.port,
                projects: vec![runtime_project],
            });
        }
    }

    Ok(())
}

fn persisted_project_php_runtime(
    database: &Database,
    project: &state::ProjectRecord,
) -> Result<Option<ResolvedPhpRuntime>, DaemonError> {
    let Some(track) = &project.php_runtime.track else {
        return Ok(None);
    };
    let loaded_modules =
        loaded_php_extension_modules(database, track, &project.php_runtime.loaded_extensions)?;
    let runtime_key = state::php_runtime_key(track, &project.php_runtime.loaded_extensions)?;

    Ok(Some(ResolvedPhpRuntime {
        track: track.clone(),
        runtime_key,
        requested_extensions: project.php_runtime.requested_extensions.clone(),
        loaded_extensions: project.php_runtime.loaded_extensions.clone(),
        ignored_extensions: project.php_runtime.ignored_extensions.clone(),
        loaded_modules,
    }))
}

fn loaded_php_extension_modules(
    database: &Database,
    track: &str,
    loaded_extensions: &[String],
) -> Result<Vec<resources::PhpExtensionModule>, DaemonError> {
    let Some(release) = installed_php_release(database, track)? else {
        if !loaded_extensions.is_empty() {
            return Err(DaemonError::Resources(
                resources::ResourcesError::InvalidArtifactLayout {
                    resource: "php".to_string(),
                    reason: format!(
                        "persisted PHP extension `{}` cannot be reconstructed because PHP track `{track}` is not installed",
                        loaded_extensions.join(", ")
                    ),
                },
            ));
        }

        return Ok(Vec::new());
    };

    Ok(resources::resolve_persisted_php_extension_modules(
        &release,
        loaded_extensions,
    )?)
}

fn installed_php_release(
    database: &Database,
    track: &str,
) -> Result<Option<Utf8PathBuf>, DaemonError> {
    let release = database
        .managed_resource_tracks()?
        .into_iter()
        .find_map(|record| {
            if record.resource_name == "php"
                && record.track == track
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
            {
                return record.current_artifact_path;
            }

            None
        });

    Ok(release)
}

async fn reconcile_gateway_config(
    paths: &PvPaths,
    command: &CaddyCliCommand,
    plan: &RuntimePlan,
) -> Result<GatewayConfigReconciliation, DaemonError> {
    let routes = gateway_project_routes(paths, plan);
    let active_dir = paths.gateway_projects_config_dir();
    let candidate_dir = candidate_config_dir_for(&active_dir);
    let fragments = gateway_project_config_fragments(paths, &routes)?;
    let readiness_hostname = gateway_readiness_hostname(&fragments);
    let import_project_configs = !fragments.is_empty();
    let active_content = match render_gateway_config(&GatewayConfigInput {
        http_port: plan.gateway.http_port,
        https_port: plan.gateway.https_port,
        admin_port: plan.gateway.admin_port,
        ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        ca_private_key_path: plan.gateway.ca_private_key_path.clone(),
        storage_path: plan.gateway.storage_path.clone(),
        access_log_path: paths.gateway_access_log(),
        error_log_path: paths.gateway_error_log(),
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
        admin_port: plan.gateway.admin_port,
        ca_certificate_path: plan.gateway.ca_certificate_path.clone(),
        ca_private_key_path: plan.gateway.ca_private_key_path.clone(),
        storage_path: plan.gateway.storage_path.clone(),
        access_log_path: paths.gateway_access_log(),
        error_log_path: paths.gateway_error_log(),
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
            let promotion = RuntimeConfigTreePromotion {
                subject: RuntimeSubject::Gateway,
                config_path: paths.gateway_root_config(),
                candidate_content: &candidate_content,
                active_content: &active_content,
                private_environment: caddy_xdg_environment(paths),
                promote_fragments: || promote_config_dir(&active_dir, &candidate_dir),
                command,
            };

            promote_runtime_config_tree(paths, promotion).await
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
    command: &CaddyCliCommand,
    artifact_root: &Utf8Path,
    worker: &PhpWorkerRuntimePlan,
) -> Result<PromotedConfigTree, DaemonError> {
    let subject = worker_runtime_subject(worker);
    let private_environment = match worker_config_private_environment(paths, worker, artifact_root)
    {
        Ok(private_environment) => private_environment,
        Err(error) => {
            record_runtime_error(paths, subject.clone(), &error)?;

            return Err(error);
        }
    };
    let active_dir = paths.worker_projects_config_dir(&worker.runtime_key);
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
        admin_port: worker.admin_port,
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
        admin_port: worker.admin_port,
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
            let promotion = RuntimeConfigTreePromotion {
                subject,
                config_path: paths.worker_root_config(&worker.runtime_key),
                candidate_content: &candidate_content,
                active_content: &active_content,
                private_environment,
                promote_fragments: || promote_config_dir(&active_dir, &candidate_dir),
                command,
            };

            promote_runtime_config_tree(paths, promotion).await
        }
        Err(error) => {
            record_runtime_error(paths, subject, &error)?;
            Err(error)
        }
    };
    let _cleanup_result = delete_optional_dir(&candidate_dir);

    result
}

struct RuntimeConfigTreePromotion<'a, PromoteFragments> {
    subject: RuntimeSubject,
    config_path: Utf8PathBuf,
    candidate_content: &'a str,
    active_content: &'a str,
    private_environment: BTreeMap<String, String>,
    promote_fragments: PromoteFragments,
    command: &'a CaddyCliCommand,
}

async fn promote_runtime_config_tree<PromoteFragments>(
    paths: &PvPaths,
    promotion: RuntimeConfigTreePromotion<'_, PromoteFragments>,
) -> Result<PromotedConfigTree, DaemonError>
where
    PromoteFragments: FnOnce() -> Result<PromotedConfigDir, DaemonError>,
{
    let RuntimeConfigTreePromotion {
        subject,
        config_path,
        candidate_content,
        active_content,
        private_environment,
        promote_fragments,
        command,
    } = promotion;
    let result = promote_validated_config_tree_async(
        &config_path,
        candidate_content,
        active_content,
        |candidate_path| async move {
            validate_config(command, &candidate_path, &private_environment).await
        },
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
    mut promoted_config: PromotedConfigTree,
    spec: ProcessSpec,
    readiness: RuntimeReadinessPlan,
    subject: RuntimeSubject,
) -> Result<RuntimeReadinessOutcome, DaemonError> {
    let matching_runtime = match supervisor.verify_ownership(&spec) {
        Ok(Some(runtime)) => !runtime.replacement_required(),
        Ok(None) => false,
        Err(error) => {
            let error = match promoted_config.rollback() {
                Ok(()) => error,
                Err(rollback_error) => runtime_config_rollback_failed_error(error, rollback_error),
            };
            record_runtime_error(paths, subject, &error)?;

            return Err(error);
        }
    };
    let previous_readiness = if matching_runtime {
        match previous_runtime_readiness(&promoted_config, &readiness) {
            Ok(readiness) => Some(readiness),
            Err(error) => {
                let error = match promoted_config.rollback() {
                    Ok(()) => error,
                    Err(rollback_error) => {
                        runtime_config_rollback_failed_error(error, rollback_error)
                    }
                };
                record_runtime_error(paths, subject, &error)?;

                return Err(error);
            }
        }
    } else {
        None
    };
    let restoration_readiness = previous_readiness
        .as_ref()
        .map_or(&readiness, |readiness| readiness);
    let result = start_or_adopt_runtime(
        paths,
        supervisor,
        &spec,
        readiness.clone(),
        matching_runtime,
    )
    .await;

    match result {
        Ok(RuntimeTransactionSuccess {
            outcome,
            restore_active_runtime,
        }) => {
            if let Err(commit_error) = promoted_config.commit() {
                let error = compensate_commit_failure(
                    paths,
                    supervisor,
                    &spec,
                    restoration_readiness,
                    promoted_config,
                    commit_error,
                    restore_active_runtime,
                )
                .await;
                record_runtime_error(paths, subject, &error)?;

                return Err(error);
            }

            Ok(outcome)
        }
        Err(RuntimeTransactionError {
            error,
            restore_active_runtime,
            config_disposition,
        }) => {
            let error = match config_disposition {
                RuntimeConfigDisposition::Rollback => match promoted_config.rollback() {
                    Ok(()) if restore_active_runtime => {
                        restore_runtime_after_failed_load(
                            paths,
                            supervisor,
                            &spec,
                            restoration_readiness,
                            error,
                        )
                        .await
                    }
                    Ok(()) => error,
                    Err(rollback_error) => {
                        runtime_config_rollback_failed_error(error, rollback_error)
                    }
                },
                RuntimeConfigDisposition::Preserve => {
                    // Keep the desired promoted tree when runtime state is uncertain. The branch
                    // that selected this disposition owns the specific recovery rationale.
                    let _cleanup_result = promoted_config.commit();
                    error
                }
            };
            record_runtime_error(paths, subject, &error)?;

            Err(error)
        }
    }
}

async fn load_runtime_config(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    client: CaddyAdminClient,
    admin_endpoint: CaddyAdminEndpoint,
    content: Vec<u8>,
) -> Result<(), RuntimeTransactionError> {
    match supervisor.mark_replacement_required(spec) {
        Ok(true) => {}
        Ok(false) => {
            return Err(RuntimeTransactionError::new(
                CaddyAdminError::runtime_ownership_changed(spec.name.clone()).into(),
            ));
        }
        Err(error) => return Err(RuntimeTransactionError::new(error)),
    }

    match client
        .load_caddyfile_with(
            admin_endpoint,
            content,
            runtime_ownership_verifier(paths, spec),
        )
        .await
    {
        Ok(()) => match supervisor.clear_replacement_required(spec) {
            Ok(true) => Ok(()),
            Ok(false) => Err(RuntimeTransactionError::preserve_promoted_config(
                CaddyAdminError::runtime_ownership_changed(spec.name.clone()).into(),
            )),
            Err(error) => Err(RuntimeTransactionError::preserve_promoted_config(error)),
        },
        Err(
            error @ CaddyAdminError::RequestOutcomeUnknown {
                operation: CaddyAdminOperation::Load,
                ..
            },
        ) => Err(RuntimeTransactionError::preserve_promoted_config(
            error.into(),
        )),
        Err(error) => {
            let _clear_result = supervisor.clear_replacement_required(spec);

            Err(RuntimeTransactionError::new(error.into()))
        }
    }
}

async fn start_or_adopt_runtime(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    readiness: RuntimeReadinessPlan,
    matching_runtime: bool,
) -> Result<RuntimeTransactionSuccess, RuntimeTransactionError> {
    let RuntimeReadinessPlan {
        check,
        failure_policy,
        timeout: readiness_timeout,
        admin_endpoint,
    } = readiness;
    async {
        if matching_runtime {
            if supervisor.verify_ownership(spec)?.is_none() {
                return Err(RuntimeTransactionError::new(
                    CaddyAdminError::runtime_ownership_changed(spec.name.clone()).into(),
                ));
            }

            let active_content = read_config_bytes(&spec.config_path)?;
            let client = CaddyAdminClient::new().with_timeout(readiness_timeout);
            load_runtime_config(
                paths,
                supervisor,
                spec,
                client,
                admin_endpoint,
                active_content,
            )
            .await?;
            verify_runtime_ownership(supervisor, spec)?;

            if let Err(error) = wait_for_owned_readiness(check.clone(), readiness_timeout, || {
                verify_runtime_ownership(supervisor, spec)
            })
            .await
            {
                if failure_policy == ReadinessFailurePolicy::PreserveRuntime
                    && supervisor.verify_ownership(spec)?.is_some()
                    && client
                        .wait_until_ready_with(
                            admin_endpoint,
                            readiness_timeout,
                            runtime_ownership_verifier(paths, spec),
                        )
                        .await
                        .is_ok()
                {
                    return Ok(RuntimeTransactionSuccess {
                        outcome: RuntimeReadinessOutcome::Unverified,
                        restore_active_runtime: true,
                    });
                }

                return Err(RuntimeTransactionError::requiring_restore(error));
            }
            verify_runtime_ownership(supervisor, spec)?;

            return Ok(RuntimeTransactionSuccess {
                outcome: RuntimeReadinessOutcome::Verified,
                restore_active_runtime: true,
            });
        } else if let Some(adopted) =
            supervisor.adopt_recorded(&spec.pid_path, &spec.metadata_path)?
        {
            adopted.stop(Duration::from_secs(1)).await?;
        } else if foreign_listener_is_ready(&check).await {
            return Err(RuntimeTransactionError::new(
                DaemonError::UnexpectedProtocolResponse {
                    reason: format!(
                        "runtime `{}` is listening but no PV-owned process could be verified",
                        spec.name
                    ),
                },
            ));
        }

        let mut process = supervisor.start(spec.clone()).await?;
        if let Err(error) = CaddyAdminClient::new()
            .with_timeout(readiness_timeout)
            .wait_until_ready_with(
                admin_endpoint,
                readiness_timeout,
                runtime_ownership_verifier(paths, spec),
            )
            .await
            .map_err(DaemonError::from)
        {
            record_runtime_readiness_diagnostics(paths, spec, &mut process, &error);
            return Err(cleanup_fresh_runtime(supervisor, spec, process, error).await);
        }

        if let Err(error) = wait_for_owned_readiness(check, readiness_timeout, || {
            verify_runtime_ownership(supervisor, spec)
        })
        .await
        {
            record_runtime_readiness_diagnostics(paths, spec, &mut process, &error);
            if failure_policy == ReadinessFailurePolicy::PreserveRuntime
                && !process.has_exited()?
                && supervisor.verify_ownership(spec)?.is_some()
            {
                return Ok(RuntimeTransactionSuccess {
                    outcome: RuntimeReadinessOutcome::Unverified,
                    restore_active_runtime: false,
                });
            }
            return Err(cleanup_fresh_runtime(supervisor, spec, process, error).await);
        }

        if process.has_exited()? {
            let error = DaemonError::UnexpectedProtocolResponse {
                reason: format!(
                    "runtime `{}` exited before readiness was verified",
                    spec.name
                ),
            };
            return Err(cleanup_fresh_runtime(supervisor, spec, process, error).await);
        }

        Ok(RuntimeTransactionSuccess {
            outcome: RuntimeReadinessOutcome::Verified,
            restore_active_runtime: false,
        })
    }
    .await
}

async fn cleanup_fresh_runtime(
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    process: ManagedProcess,
    readiness_error: DaemonError,
) -> RuntimeTransactionError {
    if let Err(cleanup_error) = process.stop(Duration::from_secs(1)).await {
        match supervisor.mark_replacement_required(spec) {
            Ok(true) => {
                // Keep the promoted config and marked runtime metadata when the process may still
                // be alive. The next reconciliation replaces it without a disk/runtime split.
                return RuntimeTransactionError::preserve_promoted_config(
                    runtime_cleanup_failed_error(&spec.name, readiness_error, cleanup_error),
                );
            }
            Ok(false) => {}
            Err(replacement_error) => {
                return RuntimeTransactionError::preserve_promoted_config(
                    runtime_cleanup_failed_error(
                        &spec.name,
                        readiness_error,
                        runtime_cleanup_failed_error(&spec.name, cleanup_error, replacement_error),
                    ),
                );
            }
        }
    }

    match cleanup_fresh_runtime_files(spec) {
        Ok(()) => RuntimeTransactionError::new(readiness_error),
        Err(cleanup_error) => RuntimeTransactionError::new(runtime_cleanup_failed_error(
            &spec.name,
            readiness_error,
            cleanup_error,
        )),
    }
}

fn cleanup_fresh_runtime_files(spec: &ProcessSpec) -> Result<(), DaemonError> {
    let pid_error = delete_optional_file(&spec.pid_path).err();
    let metadata_error = delete_optional_file(&spec.metadata_path).err();

    match (pid_error, metadata_error) {
        (None, None) => Ok(()),
        (Some(error), None) | (None, Some(error)) => Err(error),
        (Some(pid_error), Some(metadata_error)) => Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!(
                "failed to remove runtime `{}` pid file: {pid_error}; failed to remove metadata file: {metadata_error}",
                spec.name
            ),
        }),
    }
}

#[derive(Debug)]
struct RuntimeTransactionSuccess {
    outcome: RuntimeReadinessOutcome,
    restore_active_runtime: bool,
}

#[derive(Debug)]
struct RuntimeTransactionError {
    error: DaemonError,
    restore_active_runtime: bool,
    config_disposition: RuntimeConfigDisposition,
}

#[derive(Debug)]
enum RuntimeConfigDisposition {
    Rollback,
    Preserve,
}

impl RuntimeTransactionError {
    fn new(error: DaemonError) -> Self {
        Self {
            error,
            restore_active_runtime: false,
            config_disposition: RuntimeConfigDisposition::Rollback,
        }
    }

    fn requiring_restore(error: DaemonError) -> Self {
        Self {
            error,
            restore_active_runtime: true,
            config_disposition: RuntimeConfigDisposition::Rollback,
        }
    }

    fn preserve_promoted_config(error: DaemonError) -> Self {
        Self {
            error,
            restore_active_runtime: false,
            config_disposition: RuntimeConfigDisposition::Preserve,
        }
    }
}

impl From<DaemonError> for RuntimeTransactionError {
    fn from(error: DaemonError) -> Self {
        Self::new(error)
    }
}

async fn compensate_commit_failure(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    readiness: &RuntimeReadinessPlan,
    promoted_config: PromotedConfigTree,
    commit_error: DaemonError,
    restore_active_runtime: bool,
) -> DaemonError {
    match promoted_config.rollback() {
        Ok(()) if restore_active_runtime => {
            restore_runtime_after_failed_load(paths, supervisor, spec, readiness, commit_error)
                .await
        }
        Ok(()) => stop_runtime_after_failed_transaction(supervisor, spec, commit_error).await,
        Err(rollback_error) => runtime_config_rollback_failed_error(commit_error, rollback_error),
    }
}

async fn stop_runtime_after_failed_transaction(
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    original_error: DaemonError,
) -> DaemonError {
    let adopted = match supervisor.adopt(spec) {
        Ok(adopted) => adopted,
        Err(error) => return compound_runtime_restore_error(original_error, error),
    };
    let Some(adopted) = adopted else {
        return compound_runtime_restore_error(
            original_error,
            CaddyAdminError::runtime_ownership_changed(spec.name.clone()).into(),
        );
    };

    if let Err(error) = adopted.stop(Duration::from_secs(1)).await {
        return compound_runtime_restore_error(original_error, error);
    }
    if let Err(error) = delete_optional_file(&spec.pid_path) {
        return compound_runtime_restore_error(original_error, error);
    }
    if let Err(error) = delete_optional_file(&spec.metadata_path) {
        return compound_runtime_restore_error(original_error, error);
    }

    original_error
}

async fn restore_runtime_after_failed_load(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    readiness: &RuntimeReadinessPlan,
    original_error: DaemonError,
) -> DaemonError {
    if let Err(error) = verify_runtime_ownership(supervisor, spec) {
        return compound_runtime_restore_error(original_error, error);
    }

    let restored_content = match read_config_bytes(&spec.config_path) {
        Ok(content) => content,
        Err(error) => return compound_runtime_restore_error(original_error, error),
    };
    let client = CaddyAdminClient::new().with_timeout(readiness.timeout);
    if let Err(error) = load_runtime_config(
        paths,
        supervisor,
        spec,
        client,
        readiness.admin_endpoint,
        restored_content,
    )
    .await
    {
        return compound_runtime_restore_error(original_error, error.error);
    }
    if let Err(error) = verify_runtime_ownership(supervisor, spec) {
        return compound_runtime_restore_error(original_error, error);
    }
    if let Err(error) = client
        .wait_until_ready_with(
            readiness.admin_endpoint,
            readiness.timeout,
            runtime_ownership_verifier(paths, spec),
        )
        .await
    {
        return compound_runtime_restore_error(original_error, error.into());
    }
    if let Err(error) = wait_for_owned_readiness(readiness.check.clone(), readiness.timeout, || {
        verify_runtime_ownership(supervisor, spec)
    })
    .await
    {
        return compound_runtime_restore_error(original_error, error);
    }

    original_error
}

fn verify_runtime_ownership(
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
) -> Result<(), DaemonError> {
    verify_runtime_ownership_for_admin(supervisor, spec, CaddyAdminOperation::Readiness)
        .map_err(DaemonError::from)
}

fn verify_runtime_ownership_for_admin(
    supervisor: &ProcessSupervisor,
    spec: &ProcessSpec,
    operation: CaddyAdminOperation,
) -> Result<(), CaddyAdminError> {
    match supervisor.verify_ownership(spec) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(CaddyAdminError::runtime_ownership_changed(
            spec.name.clone(),
        )),
        Err(error) => Err(CaddyAdminError::TaskFailed {
            operation,
            reason: error.to_string(),
        }),
    }
}

fn runtime_ownership_verifier(paths: &PvPaths, spec: &ProcessSpec) -> CaddyAdminVerifier {
    let paths = paths.clone();
    let spec = spec.clone();

    Arc::new(move |operation| {
        let supervisor = ProcessSupervisor::new(paths.clone());
        verify_runtime_ownership_for_admin(&supervisor, &spec, operation)
    })
}

fn compound_runtime_restore_error(original: DaemonError, restored: DaemonError) -> DaemonError {
    DaemonError::CaddyAdmin(CaddyAdminError::restored_config_reload_failed(
        daemon_error_as_caddy_admin(original),
        daemon_error_as_caddy_admin_for_operation(restored, CaddyAdminOperation::Rollback),
    ))
}

fn daemon_error_as_caddy_admin(error: DaemonError) -> CaddyAdminError {
    daemon_error_as_caddy_admin_for_operation(error, CaddyAdminOperation::Readiness)
}

fn daemon_error_as_caddy_admin_for_operation(
    error: DaemonError,
    operation: CaddyAdminOperation,
) -> CaddyAdminError {
    match error {
        DaemonError::CaddyAdmin(error) => error,
        error => CaddyAdminError::TaskFailed {
            operation,
            reason: error.to_string(),
        },
    }
}

async fn wait_for_owned_readiness<BeforeProbe>(
    check: ReadinessCheck,
    readiness_timeout: Duration,
    mut before_probe: BeforeProbe,
) -> Result<(), DaemonError>
where
    BeforeProbe: FnMut() -> Result<(), DaemonError>,
{
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = readiness_timeout
        .checked_sub(started_at.elapsed())
        .filter(|remaining| !remaining.is_zero())
    {
        before_probe()?;
        let probe_timeout = remaining.min(OWNED_READINESS_PROBE_TIMEOUT);
        match timeout(probe_timeout, probe_readiness_once(&check)).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                sleep(remaining.min(OWNED_READINESS_POLL_INTERVAL)).await;
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: format!("{check:?}"),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ReadinessFailurePolicy {
    FailRuntime,
    PreserveRuntime,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RuntimeReadinessOutcome {
    Verified,
    Unverified,
}

async fn foreign_listener_is_ready(readiness: &ReadinessCheck) -> bool {
    matches!(
        timeout(
            FOREIGN_LISTENER_PROBE_TIMEOUT,
            probe_readiness_once(readiness)
        )
        .await,
        Ok(Ok(()))
    )
}

fn record_runtime_readiness_diagnostics(
    paths: &PvPaths,
    spec: &ProcessSpec,
    process: &mut ManagedProcess,
    error: &DaemonError,
) {
    let process_exited = runtime_process_exit_state(process);
    let loopback_listener_ports = loopback_listener_port_snapshot();

    structured_log::runtime_readiness_diagnostics(
        paths,
        &spec.name,
        &error.to_string(),
        &process_exited,
        &loopback_listener_ports,
    );
}

fn runtime_process_exit_state(process: &mut ManagedProcess) -> String {
    match process.has_exited() {
        Ok(exited) => exited.to_string(),
        Err(error) => format!("unknown: {error}"),
    }
}

fn loopback_listener_port_snapshot() -> String {
    match platform::loopback_tcp_listener_ports() {
        Ok(ports) => format!("{ports:?}"),
        Err(error) => format!("unavailable: {error}"),
    }
}

fn gateway_project_routes(paths: &PvPaths, plan: &RuntimePlan) -> Vec<GatewayProjectRoute> {
    plan.workers
        .iter()
        .flat_map(|worker| {
            worker.projects.iter().map(|project| GatewayProjectRoute {
                id: project.id.clone(),
                render_config: project.render_config,
                primary_hostname: project.primary_hostname.clone(),
                hostnames: project.hostnames.clone(),
                worker_port: worker.port,
                access_log_path: paths.gateway_access_log(),
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
    let active_dir = paths.worker_projects_config_dir(&worker.runtime_key);
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
    let desired_runtime_keys = plan
        .workers
        .iter()
        .map(|worker| worker.runtime_key.as_str())
        .collect::<BTreeSet<_>>();

    for runtime_key in runtime_worker_tracks(paths)? {
        if desired_runtime_keys.contains(runtime_key.as_str()) {
            continue;
        }
        let subject = php_runtime_subject(&runtime_key);

        if let Some(adopted) = supervisor.adopt_recorded(
            &paths.worker_pid(&runtime_key),
            &paths.worker_runtime_metadata(&runtime_key),
        )? {
            adopted.stop(Duration::from_secs(1)).await?;
        }
        record_runtime_observed(
            paths,
            subject,
            RuntimeObservedStatus::Stopped,
            Some("PHP worker stopped; no Projects remain on this track"),
        )?;
        cleanup_stale_worker_runtime(paths, &runtime_key)?;
    }

    Ok(())
}

fn cleanup_stale_worker_runtime(paths: &PvPaths, runtime_key: &str) -> Result<(), DaemonError> {
    delete_optional_file(&paths.worker_pid(runtime_key))?;
    delete_optional_file(&paths.worker_runtime_metadata(runtime_key))?;
    delete_optional_file(&paths.worker_root_config(runtime_key))?;
    delete_optional_dir(&paths.worker_projects_config_dir(runtime_key))?;

    let mut database = Database::open(paths)?;
    database.release_port(PortOwner::PhpWorker {
        php_runtime_key: runtime_key.to_owned(),
    })?;
    database.release_port(PortOwner::PhpWorkerAdmin {
        php_runtime_key: runtime_key.to_owned(),
    })?;

    Ok(())
}

fn worker_runtime_subject(worker: &PhpWorkerRuntimePlan) -> RuntimeSubject {
    if worker.runtime_key == worker.php_track {
        RuntimeSubject::PhpWorker {
            php_track: worker.php_track.clone(),
        }
    } else {
        RuntimeSubject::PhpRuntimeWorker {
            php_runtime_key: worker.runtime_key.clone(),
        }
    }
}

fn php_runtime_subject(runtime_key: &str) -> RuntimeSubject {
    if runtime_key.contains('+') {
        RuntimeSubject::PhpRuntimeWorker {
            php_runtime_key: runtime_key.to_owned(),
        }
    } else {
        RuntimeSubject::PhpWorker {
            php_track: runtime_key.to_owned(),
        }
    }
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
    DaemonError::CaddyAdmin(CaddyAdminError::restored_config_reload_failed(
        daemon_error_as_caddy_admin(original),
        daemon_error_as_caddy_admin_for_operation(rollback, CaddyAdminOperation::Rollback),
    ))
}

fn runtime_cleanup_failed_error(
    runtime: &str,
    original: DaemonError,
    cleanup: DaemonError,
) -> DaemonError {
    DaemonError::RuntimeCleanupFailed {
        runtime: runtime.to_owned(),
        source: Box::new(original),
        cleanup: Box::new(cleanup),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Caddy admin load must preserve the promoted root bytes exactly, including trailing bytes"
)]
fn read_config_bytes(path: &Utf8Path) -> Result<Vec<u8>, DaemonError> {
    Ok(std::fs::read(path)?)
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

fn first_installed_caddy_command(paths: &PvPaths) -> Result<Option<CaddyCliCommand>, DaemonError> {
    let database = Database::open(paths)?;
    let Some(record) = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| {
            record.resource_name == "caddy"
                && record.track == "2"
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
    else {
        return Ok(None);
    };
    let Some(artifact_path) = record.current_artifact_path else {
        return Ok(None);
    };
    let adapter = caddy_adapter()?;

    adapter.validate_installation(&artifact_path)?;

    Ok(Some(CaddyCliCommand::caddy(
        adapter.executable_path(&artifact_path),
    )))
}

fn installed_frankenphp_runtime_for_track(
    paths: &PvPaths,
    php_track: &str,
) -> Result<Option<InstalledFrankenphpRuntime>, DaemonError> {
    let database = Database::open(paths)?;
    let runtime = installed_frankenphp_tracks(&database)?
        .into_iter()
        .find(|record| record.track == php_track)
        .map(frankenphp_runtime_from_record)
        .transpose()?;

    Ok(runtime)
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

fn frankenphp_runtime_from_record(
    record: ManagedResourceTrackRecord,
) -> Result<InstalledFrankenphpRuntime, DaemonError> {
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

    Ok(InstalledFrankenphpRuntime {
        command: CaddyCliCommand::frankenphp(adapter.executable_path(&artifact_path)),
        artifact_root: artifact_path,
    })
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

fn record_gateway_runtime_observed(
    paths: &PvPaths,
    pf_routing_state: GatewayPfRoutingState,
    readiness_outcome: RuntimeReadinessOutcome,
) -> Result<(), DaemonError> {
    let (status, message) = match (pf_routing_state, readiness_outcome) {
        (GatewayPfRoutingState::Active, _)
        | (GatewayPfRoutingState::Unknown, RuntimeReadinessOutcome::Verified) => {
            (RuntimeObservedStatus::Running, GATEWAY_RUNTIME_RECONCILED)
        }
        (GatewayPfRoutingState::Inactive, _) => (
            RuntimeObservedStatus::Degraded,
            "Low-port routing is inactive; run `pv ports:install` to restore ports 80 and 443",
        ),
        (GatewayPfRoutingState::Drifted, _) => (
            RuntimeObservedStatus::Degraded,
            "Low-port routing is drifted; run `pv ports:install` to restore ports 80 and 443",
        ),
        (GatewayPfRoutingState::Unknown, RuntimeReadinessOutcome::Unverified) => (
            RuntimeObservedStatus::Degraded,
            "Low-port routing is unknown; run `pv ports:install` to verify ports 80 and 443",
        ),
    };

    record_runtime_observed(paths, RuntimeSubject::Gateway, status, Some(message))
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

fn recover_unowned_admin_port_assignments(
    paths: &PvPaths,
    supervisor: &ProcessSupervisor,
) -> Result<(), (RuntimeSubject, Box<DaemonError>)> {
    let mut database =
        Database::open(paths).map_err(|error| (RuntimeSubject::Gateway, Box::new(error.into())))?;
    let assignments = database
        .assigned_ports()
        .map_err(|error| (RuntimeSubject::Gateway, Box::new(error.into())))?;

    for assignment in assignments {
        let runtime = match &assignment.owner {
            PortOwner::Gateway(GatewayPort::Admin) => Some((
                RuntimeSubject::Gateway,
                paths.gateway_pid(),
                paths.gateway_runtime_metadata(),
                paths.gateway_root_config(),
            )),
            PortOwner::PhpWorkerAdmin { php_runtime_key } => Some((
                php_runtime_subject(php_runtime_key),
                paths.worker_pid(php_runtime_key),
                paths.worker_runtime_metadata(php_runtime_key),
                paths.worker_root_config(php_runtime_key),
            )),
            _ => None,
        };
        let Some((subject, pid_path, metadata_path, config_path)) = runtime else {
            continue;
        };

        if local_loopback_port_available(assignment.port) {
            continue;
        }
        let recorded_runtime = supervisor
            .adopt_recorded(&pid_path, &metadata_path)
            .map_err(|error| (subject.clone(), Box::new(error)))?;
        if recorded_runtime.is_some()
            && runtime_config_uses_admin_port(&config_path, assignment.port)
                .map_err(|error| (subject.clone(), Box::new(error)))?
        {
            continue;
        }

        database
            .release_port(assignment.owner)
            .map_err(|error| (subject, Box::new(error.into())))?;
    }

    Ok(())
}

fn runtime_config_uses_admin_port(
    config_path: &Utf8Path,
    assigned_port: u16,
) -> Result<bool, DaemonError> {
    match fs::read_to_string(config_path) {
        Ok(config) => Ok(optional_config_port(&config, "admin 127.0.0.1:")? == Some(assigned_port)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn caddyfile_arguments(action: &str, config_path: &Utf8Path) -> Vec<String> {
    vec![
        action.to_owned(),
        "--config".to_owned(),
        config_path.as_str().to_owned(),
        "--adapter".to_owned(),
        "caddyfile".to_owned(),
    ]
}

fn caddy_xdg_environment(paths: &PvPaths) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "XDG_CONFIG_HOME".to_owned(),
            paths.config().as_str().to_owned(),
        ),
        (
            "XDG_DATA_HOME".to_owned(),
            paths.certificates().as_str().to_owned(),
        ),
    ])
}

fn frankenphp_worker_environment(
    paths: &PvPaths,
    worker: &PhpWorkerRuntimePlan,
    artifact_root: &Utf8Path,
) -> Result<BTreeMap<String, String>, DaemonError> {
    let mut environment = caddy_xdg_environment(paths);
    environment.extend(resources::php_runtime_environment(
        paths,
        &worker.php_track,
        &worker.runtime_key,
        artifact_root,
        &worker.loaded_modules,
    )?);

    Ok(environment)
}

fn worker_config_private_environment(
    paths: &PvPaths,
    worker: &PhpWorkerRuntimePlan,
    artifact_root: &Utf8Path,
) -> Result<BTreeMap<String, String>, DaemonError> {
    resources::ensure_php_track_defaults(paths, &worker.php_track)?;

    frankenphp_worker_environment(paths, worker, artifact_root)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use anyhow::Result;
    use camino::Utf8PathBuf;
    use camino_tempfile::tempdir;
    use platform::{ActivePfRedirectInspection, PfRedirectConfig};
    use state::PvPaths;

    use crate::ReadinessCheck;
    use crate::gateway_config::GatewayProjectRoute;

    use super::{
        GatewayPfRoutingState, GatewayReadinessPorts, GatewayRuntimePlan, ReadinessFailurePolicy,
        RuntimePlan, classify_gateway_pf_routing_state, gateway_project_config_fragments,
        gateway_public_readiness_check, gateway_readiness_check_for_ports,
        gateway_readiness_hostname, gateway_readiness_plan, gateway_readiness_ports,
        previous_runtime_readiness_from_parts, project_config_file_name,
    };

    #[test]
    fn gateway_readiness_uses_https_for_hostname_before_ca_file_exists() -> Result<()> {
        let plan = runtime_plan();

        let readiness = gateway_readiness_check_for_ports(
            &plan,
            Some("project.test".to_string()),
            GatewayReadinessPorts {
                http: plan.gateway.http_port,
                https: plan.gateway.https_port,
            },
        );

        assert_eq!(
            readiness,
            ReadinessCheck::GatewayHttps {
                http_host: "127.0.0.1".to_string(),
                http_port: 45080,
                https_host: "127.0.0.1".to_string(),
                https_port: 45443,
                server_name: "project.test".to_string(),
                ca_certificate_path: Utf8PathBuf::from("/tmp/pv-missing-ca.pem"),
            }
        );

        Ok(())
    }

    #[test]
    fn gateway_readiness_uses_identity_probes_on_public_ports() -> Result<()> {
        let plan = runtime_plan();

        let readiness = gateway_public_readiness_check(
            &plan,
            GatewayReadinessPorts {
                http: 80,
                https: 443,
            },
        );

        assert_eq!(
            readiness,
            ReadinessCheck::GatewayIdentity {
                http_host: "127.0.0.1".to_string(),
                http_port: 80,
                https_host: "127.0.0.1".to_string(),
                https_port: 443,
                server_name: "pv-gateway.localhost".to_string(),
                path: "/__pv/health".to_string(),
                expected_body: "pv-gateway-health-v1:45080:45443".to_string(),
                ca_certificate_path: Utf8PathBuf::from("/tmp/pv-missing-ca.pem"),
            }
        );

        Ok(())
    }

    #[test]
    fn previous_runtime_readiness_uses_previous_root_admin_endpoint() -> Result<()> {
        let plan = runtime_plan();
        let readiness = gateway_readiness_plan(
            &plan,
            None,
            GatewayPfRoutingState::Inactive,
            Duration::from_secs(1),
        );
        let previous = previous_runtime_readiness_from_parts(
            "{\n    admin 127.0.0.1:41019\n    http_port 45080\n    https_port 45443\n}\n",
            &[],
            &readiness,
        )?;

        assert_eq!(
            previous.admin_endpoint,
            super::CaddyAdminEndpoint::new(41019)
        );
        assert_eq!(
            previous.check,
            ReadinessCheck::Tcp {
                host: "127.0.0.1".to_owned(),
                port: 45080,
            }
        );

        Ok(())
    }

    #[test]
    fn gateway_readiness_uses_public_ports_for_active_drifted_and_unknown_pf() {
        let plan = runtime_plan();

        for state in [
            GatewayPfRoutingState::Active,
            GatewayPfRoutingState::Drifted,
            GatewayPfRoutingState::Unknown,
        ] {
            assert_eq!(
                gateway_readiness_ports(&plan, state),
                GatewayReadinessPorts {
                    http: 80,
                    https: 443,
                }
            );
        }
    }

    #[test]
    fn gateway_readiness_uses_backend_ports_only_when_pf_is_inactive() {
        let plan = runtime_plan();

        assert_eq!(
            gateway_readiness_ports(&plan, GatewayPfRoutingState::Inactive),
            GatewayReadinessPorts {
                http: plan.gateway.http_port,
                https: plan.gateway.https_port,
            }
        );
    }

    #[test]
    fn confirmed_inactive_evidence_selects_backend_readiness() {
        let plan = runtime_plan();
        let expected = PfRedirectConfig::new(plan.gateway.http_port, plan.gateway.https_port);
        let inspection = pf_inspection(None, BTreeSet::new());
        let state = classify_gateway_pf_routing_state(&expected, Some(&inspection), true);
        let readiness = gateway_readiness_plan(&plan, None, state, Duration::from_secs(60));

        assert_eq!(state, GatewayPfRoutingState::Inactive);
        assert_eq!(
            readiness.check,
            ReadinessCheck::Tcp {
                host: "127.0.0.1".to_string(),
                port: plan.gateway.http_port,
            }
        );
    }

    #[test]
    fn matching_loaded_rules_are_active_only_with_current_files() {
        let expected = PfRedirectConfig::new(45080, 45443);
        let inspection = pf_inspection(Some(expected.clone()), BTreeSet::new());

        assert_eq!(
            classify_gateway_pf_routing_state(&expected, Some(&inspection), true),
            GatewayPfRoutingState::Active
        );
        assert_eq!(
            classify_gateway_pf_routing_state(&expected, Some(&inspection), false),
            GatewayPfRoutingState::Drifted
        );
    }

    #[test]
    fn redirects_targeting_backend_ports_are_drifted_not_inactive() {
        let expected = PfRedirectConfig::new(45080, 45443);
        let inspection = pf_inspection(None, BTreeSet::from([45080]));

        assert_eq!(
            classify_gateway_pf_routing_state(&expected, Some(&inspection), true),
            GatewayPfRoutingState::Drifted
        );
    }

    #[test]
    fn unresolved_redirect_targets_are_drifted_not_inactive() {
        let expected = PfRedirectConfig::new(45080, 45443);
        let mut inspection = pf_inspection(None, BTreeSet::new());
        inspection.has_unresolved_redirect_targets = true;

        assert_eq!(
            classify_gateway_pf_routing_state(&expected, Some(&inspection), true),
            GatewayPfRoutingState::Drifted
        );
    }

    #[test]
    fn disabled_pf_and_partial_pv_rules_are_drifted() {
        let expected = PfRedirectConfig::new(45080, 45443);
        let mut disabled = pf_inspection(Some(expected.clone()), BTreeSet::from([45080, 45443]));
        disabled.pf_enabled = false;
        let mut partial = pf_inspection(None, BTreeSet::from([44080]));
        partial.pv_anchor_has_unparsed_rules = true;

        assert_eq!(
            classify_gateway_pf_routing_state(&expected, Some(&disabled), true),
            GatewayPfRoutingState::Drifted
        );
        assert_eq!(
            classify_gateway_pf_routing_state(&expected, Some(&partial), true),
            GatewayPfRoutingState::Drifted
        );
    }

    #[test]
    fn unavailable_rule_inspection_uses_bounded_advisory_public_readiness() {
        let plan = runtime_plan();

        let readiness = gateway_readiness_plan(
            &plan,
            Some("project.test".to_string()),
            GatewayPfRoutingState::Unknown,
            Duration::from_secs(60),
        );

        assert_eq!(
            readiness.failure_policy,
            ReadinessFailurePolicy::PreserveRuntime
        );
        assert_eq!(readiness.timeout, Duration::from_secs(2));
        assert_eq!(
            readiness.check,
            ReadinessCheck::GatewayIdentity {
                http_host: "127.0.0.1".to_string(),
                http_port: 80,
                https_host: "127.0.0.1".to_string(),
                https_port: 443,
                server_name: "pv-gateway.localhost".to_string(),
                path: "/__pv/health".to_string(),
                expected_body: "pv-gateway-health-v1:45080:45443".to_string(),
                ca_certificate_path: Utf8PathBuf::from("/tmp/pv-missing-ca.pem"),
            }
        );
    }

    #[test]
    fn unavailable_rule_inspection_is_unknown_only_when_files_are_current() {
        let expected = PfRedirectConfig::new(45080, 45443);

        assert_eq!(
            classify_gateway_pf_routing_state(&expected, None, true),
            GatewayPfRoutingState::Unknown
        );
        assert_eq!(
            classify_gateway_pf_routing_state(&expected, None, false),
            GatewayPfRoutingState::Drifted
        );
    }

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
                access_log_path: paths.gateway_access_log(),
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

    fn runtime_plan() -> RuntimePlan {
        RuntimePlan {
            gateway: GatewayRuntimePlan {
                http_port: 45080,
                https_port: 45443,
                admin_port: 42019,
                ca_certificate_path: Utf8PathBuf::from("/tmp/pv-missing-ca.pem"),
                ca_private_key_path: Utf8PathBuf::from("/tmp/pv-missing-ca-key.pem"),
                storage_path: Utf8PathBuf::from("/tmp/pv-gateway-storage"),
            },
            workers: Vec::new(),
        }
    }

    fn pf_inspection(
        pv_config: Option<PfRedirectConfig>,
        resolved_target_ports: BTreeSet<u16>,
    ) -> ActivePfRedirectInspection {
        ActivePfRedirectInspection {
            pf_enabled: true,
            pv_config,
            pv_anchor_has_unparsed_rules: false,
            resolved_target_ports,
            has_unresolved_redirect_targets: false,
        }
    }
}
