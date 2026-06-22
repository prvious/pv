use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{
    CaFileState, LaunchAgentFileState, LocalCaMetadata, PfConfReference, PfFileState,
    ResolverConfig, ResolverFileState, TrustDomainState,
};
use serde::Serialize;
use state::{
    Database, JobRecord, JobStatus, ManagedResourceDesiredState, ManagedResourceTrackRecord,
    ProjectEnvObservedStatus, ProjectRecord, PvPaths, RuntimeObservedStateRecord,
    RuntimeObservedStatus, RuntimeSubject, StateError,
};

use crate::args::StatusArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn run(
    args: StatusArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let snapshot = StatusSnapshot::read(environment)?;
    let exit_code = if snapshot.has_failure() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };

    if args.json {
        serde_json::to_writer(&mut *stdout, &snapshot)?;
        writeln!(stdout)?;

        return Ok(exit_code);
    }

    let mut output = Output::new(stdout, OutputMode::plain());
    snapshot.write_plain(&mut output)?;

    Ok(exit_code)
}

#[derive(Serialize)]
struct StatusSnapshot {
    overall: &'static str,
    daemon: DaemonStatus,
    integrations: IntegrationStatuses,
    managed_resources: Vec<ManagedResourceStatus>,
    runtimes: Vec<RuntimeStatus>,
    projects: Vec<ProjectStatus>,
    recent_errors: Vec<JobStatusSummary>,
    log_directory: String,
}

impl StatusSnapshot {
    fn read(environment: &impl Environment) -> Result<Self, ExecuteError> {
        let paths = pv_paths(environment)?;
        let database = Database::open_read_only(&paths)?;
        let daemon = DaemonStatus::read(environment, &paths)?;
        let integrations = IntegrationStatuses::read(environment, &paths)?;
        let runtime_states = match &database {
            Some(database) => database.runtime_observed_states()?,
            None => Vec::new(),
        };
        let managed_resources = match &database {
            Some(database) => managed_resource_statuses(database, &runtime_states)?,
            None => Vec::new(),
        };
        let runtimes = runtime_statuses(&runtime_states);
        let projects = match &database {
            Some(database) => project_statuses(database)?,
            None => Vec::new(),
        };
        let recent_errors = match &database {
            Some(database) => database
                .recent_jobs()?
                .into_iter()
                .filter(|job| job.status == JobStatus::Failed)
                .map(JobStatusSummary::from_job)
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let has_failure = daemon.failure
            || integrations.failure
            || managed_resources.iter().any(|resource| resource.failure)
            || runtimes.iter().any(|runtime| runtime.failure)
            || projects.iter().any(|project| project.failure)
            || !recent_errors.is_empty();
        let overall = if has_failure { "failed" } else { "ok" };

        Ok(Self {
            overall,
            daemon,
            integrations,
            managed_resources,
            runtimes,
            projects,
            recent_errors,
            log_directory: paths.logs().to_string(),
        })
    }

    fn has_failure(&self) -> bool {
        self.overall == "failed"
    }

    fn write_plain(&self, output: &mut Output<'_, impl Write>) -> Result<(), ExecuteError> {
        output.line("PV status")?;
        output.line(&format!("Overall: {}", self.overall))?;
        output.line(&format!("Daemon: {}", self.daemon.state))?;
        output.line(&format!("  LaunchAgent: {}", self.daemon.launch_agent))?;
        output.line(&format!("  Socket: {}", self.daemon.socket))?;
        output.line("Integrations:")?;
        output.line(&format!("  DNS: {}", self.integrations.dns))?;
        output.line(&format!("  Ports: {}", self.integrations.ports))?;
        output.line(&format!("  CA: {}", self.integrations.ca))?;
        output.line(&format!("Logs: {}", self.log_directory))?;
        output.line("Managed Resources:")?;
        if self.managed_resources.is_empty() {
            output.line("  none")?;
        } else {
            for resource in &self.managed_resources {
                output.line(&format!(
                    "  {} {} {} projects={} version={}",
                    resource.name,
                    resource.track,
                    resource.status,
                    resource.projects,
                    resource.version.as_deref().unwrap_or("-"),
                ))?;
            }
        }
        if !self.runtimes.is_empty() {
            output.line("Runtimes:")?;
            for runtime in &self.runtimes {
                output.line(&format!(
                    "  {} {} {}",
                    runtime.subject,
                    runtime.status,
                    runtime.message.as_deref().unwrap_or("-"),
                ))?;
            }
        }
        output.line("Projects:")?;
        if self.projects.is_empty() {
            output.line("  none")?;
        } else {
            for project in &self.projects {
                output.line(&format!(
                    "  {} env={} {}",
                    project.hostname,
                    project.env_status,
                    project.message.as_deref().unwrap_or("-"),
                ))?;
            }
        }
        output.line("Recent errors:")?;
        if self.recent_errors.is_empty() {
            output.line("  none")?;
        } else {
            for job in &self.recent_errors {
                output.line(&format!(
                    "  {} {} {} failed: {}",
                    job.id,
                    job.kind,
                    job.scope,
                    job.error.as_deref().unwrap_or("-"),
                ))?;
            }
        }

        Ok(())
    }
}

#[derive(Serialize)]
struct DaemonStatus {
    state: &'static str,
    launch_agent: &'static str,
    socket: &'static str,
    failure: bool,
}

impl DaemonStatus {
    fn read(environment: &impl Environment, paths: &PvPaths) -> Result<Self, ExecuteError> {
        let launch_agent_path = Utf8PathBuf::from_path_buf(environment.launch_agent_path())
            .map_err(|path| crate::error::CliError::NonUtf8Path { path })?;
        let launch_agent = platform::inspect_launch_agent_file(&launch_agent_path, None);
        let launch_agent_status = launch_agent_status(&launch_agent);
        let socket_exists = state::fs::path_exists(&paths.daemon_socket());
        let socket = if !socket_exists {
            "missing"
        } else if daemon::health_blocking(paths.clone()).is_ok() {
            "healthy"
        } else {
            "unhealthy"
        };
        let state = match &launch_agent {
            LaunchAgentFileState::Missing { .. } if socket == "missing" => "disabled",
            LaunchAgentFileState::Missing { .. } if socket == "healthy" => "socket-only",
            LaunchAgentFileState::Missing { .. } => "socket-stale",
            LaunchAgentFileState::Current { .. } if socket == "healthy" => "running",
            LaunchAgentFileState::Current { .. } => "down",
            LaunchAgentFileState::Stale { .. } => "repair-required",
            LaunchAgentFileState::Conflict { .. } => "repair-required",
            LaunchAgentFileState::Unreadable { .. } => "unknown",
        };
        let failure = matches!(state, "down" | "repair-required" | "socket-stale");

        Ok(Self {
            state,
            launch_agent: launch_agent_status,
            socket,
            failure,
        })
    }
}

#[derive(Serialize)]
struct IntegrationStatuses {
    dns: &'static str,
    ports: &'static str,
    ca: &'static str,
    #[serde(skip)]
    failure: bool,
}

impl IntegrationStatuses {
    fn read(environment: &impl Environment, paths: &PvPaths) -> Result<Self, ExecuteError> {
        let prepared_resolver = platform::inspect_resolver_file(&paths.resolver_config(), None);
        let expected_resolver = resolver_config_from_state(&prepared_resolver);
        let system_resolver_path = resolver_test_path(environment)?;
        let system_resolver =
            platform::inspect_resolver_file(&system_resolver_path, expected_resolver.as_ref());
        let (dns, dns_failure) = resolver_status(&prepared_resolver, &system_resolver);

        let prepared_pf_anchor = platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), None);
        let prepared_pf_reference =
            platform::inspect_pf_conf_reference(&paths.pf_conf_reference_config(), None);
        let expected_pf_anchor = pf_config_from_anchor_state(&prepared_pf_anchor);
        let expected_pf_reference = pf_reference_from_state(&prepared_pf_reference);
        let system_pf_anchor_path = pf_anchor_path(environment)?;
        let system_pf_conf_path = pf_conf_path(environment)?;
        let system_pf_anchor =
            platform::inspect_pf_anchor_file(&system_pf_anchor_path, expected_pf_anchor.as_ref());
        let system_pf_reference = platform::inspect_pf_conf_reference(
            &system_pf_conf_path,
            expected_pf_reference.as_ref(),
        );
        let active_pf = if pf_file_status(&prepared_pf_anchor, &prepared_pf_reference) == "current"
            && pf_file_status(&system_pf_anchor, &system_pf_reference) == "current"
        {
            Some(environment.active_pf_redirect_config())
        } else {
            None
        };
        let (ports, ports_failure) = pf_status(
            &prepared_pf_anchor,
            &prepared_pf_reference,
            &system_pf_anchor,
            &system_pf_reference,
            active_pf
                .as_ref()
                .and_then(|active| active.as_ref().map(|config| config.as_ref()).ok()),
        );

        let local_ca =
            platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
        let local_metadata = metadata_from_local_ca(&local_ca);
        let trust = ca_trust_state(environment, local_metadata.as_ref());
        let (ca, ca_failure) = ca_status(&local_ca, &trust);

        Ok(Self {
            dns,
            ports,
            ca,
            failure: dns_failure || ports_failure || ca_failure,
        })
    }
}

#[derive(Serialize)]
struct ManagedResourceStatus {
    name: String,
    track: String,
    desired: &'static str,
    status: &'static str,
    projects: i64,
    version: Option<String>,
    failure: bool,
}

#[derive(Serialize)]
struct RuntimeStatus {
    subject: String,
    status: &'static str,
    message: Option<String>,
    observed_at: String,
    failure: bool,
}

#[derive(Serialize)]
struct ProjectStatus {
    hostname: String,
    env_status: &'static str,
    message: Option<String>,
    observed_at: Option<String>,
    #[serde(skip)]
    failure: bool,
}

#[derive(Serialize)]
struct JobStatusSummary {
    id: String,
    kind: String,
    scope: String,
    started_at: String,
    finished_at: Option<String>,
    error: Option<String>,
}

impl JobStatusSummary {
    fn from_job(job: JobRecord) -> Self {
        Self {
            id: job.id,
            kind: job.kind,
            scope: job.scope,
            started_at: job.started_at,
            finished_at: job.finished_at,
            error: job.error,
        }
    }
}

fn managed_resource_statuses(
    database: &Database,
    runtime_states: &[RuntimeObservedStateRecord],
) -> Result<Vec<ManagedResourceStatus>, ExecuteError> {
    let runtime_by_resource = runtime_states
        .iter()
        .filter_map(|state| {
            if let RuntimeSubject::Resource { name, track } = &state.subject {
                return Some(((name.as_str(), track.as_str()), state.status));
            }

            None
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .map(|track| managed_resource_status(track, &runtime_by_resource))
        .collect())
}

fn managed_resource_status(
    track: ManagedResourceTrackRecord,
    runtime_by_resource: &std::collections::BTreeMap<(&str, &str), RuntimeObservedStatus>,
) -> ManagedResourceStatus {
    let runtime_status = runtime_by_resource
        .get(&(track.resource_name.as_str(), track.track.as_str()))
        .copied();
    let status = runtime_status
        .map(runtime_status_label)
        .unwrap_or("not-running");
    let failure = matches!(
        runtime_status,
        Some(RuntimeObservedStatus::Failed | RuntimeObservedStatus::Degraded)
    );

    ManagedResourceStatus {
        name: track.resource_name,
        track: track.track,
        desired: desired_state_label(track.desired_state),
        status,
        projects: track.usage_count,
        version: track.installed_version,
        failure,
    }
}

fn runtime_statuses(runtime_states: &[RuntimeObservedStateRecord]) -> Vec<RuntimeStatus> {
    runtime_states
        .iter()
        .filter_map(|state| match &state.subject {
            RuntimeSubject::Gateway | RuntimeSubject::PhpWorker { .. } => Some(RuntimeStatus {
                subject: runtime_subject_label(&state.subject),
                status: runtime_status_label(state.status),
                message: state.message.clone(),
                observed_at: state.observed_at.clone(),
                failure: matches!(
                    state.status,
                    RuntimeObservedStatus::Failed | RuntimeObservedStatus::Degraded
                ),
            }),
            RuntimeSubject::Resource { .. } => None,
        })
        .collect()
}

fn project_statuses(database: &Database) -> Result<Vec<ProjectStatus>, ExecuteError> {
    let mut statuses = Vec::new();

    for project in database.projects()? {
        let observed = database.project_env_observed_state(&project.id)?;
        let status = project_status(project, observed);
        if status.env_status != "rendered" {
            statuses.push(status);
        }
    }

    Ok(statuses)
}

fn project_status(
    project: ProjectRecord,
    observed: Option<state::ProjectEnvObservedStateRecord>,
) -> ProjectStatus {
    let Some(observed) = observed else {
        return ProjectStatus {
            hostname: project.primary_hostname,
            env_status: "pending",
            message: Some("Project env has not been observed yet".to_string()),
            observed_at: None,
            failure: false,
        };
    };
    let env_status = project_env_status_label(observed.status);
    let failure = observed.status == ProjectEnvObservedStatus::Failed;
    let message = if observed.status == ProjectEnvObservedStatus::Warning {
        observed
            .warnings
            .first()
            .map(|warning| warning.message.clone())
            .or(observed.message)
    } else {
        observed.message
    };

    ProjectStatus {
        hostname: project.primary_hostname,
        env_status,
        message,
        observed_at: Some(observed.observed_at),
        failure,
    }
}

fn launch_agent_status(state: &LaunchAgentFileState) -> &'static str {
    match state {
        LaunchAgentFileState::Missing { .. } => "missing",
        LaunchAgentFileState::Current { .. } => "current",
        LaunchAgentFileState::Stale { .. } => "stale",
        LaunchAgentFileState::Conflict { .. } => "conflict",
        LaunchAgentFileState::Unreadable { .. } => "unreadable",
    }
}

fn resolver_status(
    prepared: &ResolverFileState,
    system: &ResolverFileState,
) -> (&'static str, bool) {
    match prepared {
        ResolverFileState::Missing { .. } => ("missing", false),
        ResolverFileState::Current { .. } => match system {
            ResolverFileState::Current { .. } => ("current", false),
            ResolverFileState::Missing { .. } => ("prepared-only", true),
            ResolverFileState::Stale { .. } => ("stale", true),
            ResolverFileState::Conflict { .. } => ("conflict", true),
            ResolverFileState::Unreadable { .. } => ("unreadable", true),
        },
        ResolverFileState::Stale { .. } => ("stale", true),
        ResolverFileState::Conflict { .. } => ("conflict", true),
        ResolverFileState::Unreadable { .. } => ("unreadable", true),
    }
}

fn pf_status(
    prepared_anchor: &PfFileState<platform::PfRedirectConfig>,
    prepared_reference: &PfFileState<PfConfReference>,
    system_anchor: &PfFileState<platform::PfRedirectConfig>,
    system_reference: &PfFileState<PfConfReference>,
    active: Option<Option<&platform::PfRedirectConfig>>,
) -> (&'static str, bool) {
    let prepared_status = pf_file_status(prepared_anchor, prepared_reference);
    if prepared_status != "current" {
        return (prepared_status, prepared_status != "missing");
    }

    let system_status = pf_file_status(system_anchor, system_reference);
    if system_status != "current" {
        if system_status == "missing" {
            return ("prepared-only", true);
        }

        return (system_status, true);
    }

    let Some(active) = active else {
        return ("unreadable", true);
    };
    let expected = pf_config_from_anchor_state(prepared_anchor);
    if active == expected.as_ref() {
        ("current", false)
    } else {
        ("inactive", true)
    }
}

fn pf_file_status(
    anchor: &PfFileState<platform::PfRedirectConfig>,
    reference: &PfFileState<PfConfReference>,
) -> &'static str {
    match (anchor, reference) {
        (PfFileState::Missing { .. }, PfFileState::Missing { .. }) => "missing",
        (PfFileState::Current { .. }, PfFileState::Current { .. }) => "current",
        (PfFileState::Conflict { .. }, _) | (_, PfFileState::Conflict { .. }) => "conflict",
        (PfFileState::Unreadable { .. }, _) | (_, PfFileState::Unreadable { .. }) => "unreadable",
        _ => "stale",
    }
}

fn ca_status(state: &CaFileState, trust: &TrustDomainState) -> (&'static str, bool) {
    match state {
        CaFileState::Missing { .. } => ("missing", false),
        CaFileState::Current { .. } => match trust {
            TrustDomainState::Current { .. } => ("current", false),
            TrustDomainState::NotTrusted { .. } => ("not-trusted", true),
            TrustDomainState::Stale { .. } => ("stale", true),
            TrustDomainState::Denied { .. } => ("denied", true),
            TrustDomainState::Unknown { .. } => ("unknown", true),
            TrustDomainState::Unreadable { .. } => ("unreadable", true),
        },
        CaFileState::RepairRequired { .. } => ("repair-required", true),
        CaFileState::Unreadable { .. } => ("unreadable", true),
    }
}

fn runtime_subject_label(subject: &RuntimeSubject) -> String {
    match subject {
        RuntimeSubject::Gateway => "gateway".to_string(),
        RuntimeSubject::PhpWorker { php_track } => format!("worker:{php_track}"),
        RuntimeSubject::Resource { name, track } => format!("{name}:{track}"),
    }
}

fn runtime_status_label(status: RuntimeObservedStatus) -> &'static str {
    match status {
        RuntimeObservedStatus::Pending => "pending",
        RuntimeObservedStatus::Running => "running",
        RuntimeObservedStatus::Degraded => "degraded",
        RuntimeObservedStatus::Failed => "failed",
        RuntimeObservedStatus::Stopped => "stopped",
    }
}

fn desired_state_label(state: ManagedResourceDesiredState) -> &'static str {
    match state {
        ManagedResourceDesiredState::Installed => "installed",
        ManagedResourceDesiredState::Removed => "removed",
    }
}

fn project_env_status_label(status: ProjectEnvObservedStatus) -> &'static str {
    match status {
        ProjectEnvObservedStatus::Pending => "pending",
        ProjectEnvObservedStatus::Rendered => "rendered",
        ProjectEnvObservedStatus::Warning => "warning",
        ProjectEnvObservedStatus::Failed => "failed",
    }
}

fn resolver_config_from_state(state: &ResolverFileState) -> Option<ResolverConfig> {
    match state {
        ResolverFileState::Current { port, .. } => Some(ResolverConfig::new(*port)),
        ResolverFileState::Missing { .. }
        | ResolverFileState::Stale { .. }
        | ResolverFileState::Conflict { .. }
        | ResolverFileState::Unreadable { .. } => None,
    }
}

fn pf_config_from_anchor_state(
    state: &PfFileState<platform::PfRedirectConfig>,
) -> Option<platform::PfRedirectConfig> {
    match state {
        PfFileState::Current { value, .. } => Some(value.clone()),
        PfFileState::Missing { .. }
        | PfFileState::Stale { .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn pf_reference_from_state(state: &PfFileState<PfConfReference>) -> Option<PfConfReference> {
    match state {
        PfFileState::Current { value, .. } => Some(*value),
        PfFileState::Missing { .. }
        | PfFileState::Stale { .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn metadata_from_local_ca(state: &CaFileState) -> Option<LocalCaMetadata> {
    match state {
        CaFileState::Current { metadata, .. } => Some(metadata.clone()),
        CaFileState::Missing { .. }
        | CaFileState::RepairRequired { .. }
        | CaFileState::Unreadable { .. } => None,
    }
}

fn ca_trust_state(
    environment: &impl Environment,
    metadata: Option<&LocalCaMetadata>,
) -> TrustDomainState {
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

    let inspector = EnvironmentTrustInspector { environment };
    platform::inspect_system_ca_trust(metadata, &inspector)
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn resolver_test_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.resolver_test_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_anchor_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_anchor_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_conf_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_conf_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}
