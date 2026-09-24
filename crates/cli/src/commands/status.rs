use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{
    CaFileState, LaunchAgentFileState, LocalCaMetadata, ResolverConfig, ResolverFileState,
    TrustDomainState,
};
use serde::Serialize;
use state::{
    Database, JobRecord, ManagedResourceDesiredState, ManagedResourceTrackRecord,
    ProjectEnvObservedStatus, ProjectRecord, PvPaths, RuntimeObservedStateRecord,
    RuntimeObservedStatus, RuntimeSubject, StateError,
};

use crate::args::StatusArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Output, Streams, Table, Tone};

use super::pf_diagnostics::{PfRoutingDiagnostic, PfRoutingState};

pub(crate) fn run(
    args: StatusArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let snapshot = StatusSnapshot::read(environment)?;
    let exit_code = if snapshot.has_failure() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };

    if args.json {
        streams.out.json(&snapshot)?;

        return Ok(exit_code);
    }

    snapshot.write(&mut streams.out)?;

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
        let integrations = IntegrationStatuses::read(
            environment,
            &paths,
            database.as_ref(),
            daemon.state != "disabled",
        )?;
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
                .unresolved_job_failures()?
                .into_iter()
                .map(|failure| JobStatusSummary::from_job(failure.job))
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let has_failure = daemon.failure
            || integrations.failure
            || managed_resources.iter().any(|resource| resource.failure)
            || runtimes.iter().any(|runtime| runtime.failure)
            || projects.iter().any(|project| project.mark == Mark::Failure)
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

    fn write(&self, output: &mut Output<'_>) -> Result<(), ExecuteError> {
        if output.surface().decorated() {
            return self.write_decorated(output);
        }
        output.line("PV status")?;
        output.line(&format!("Overall: {}", self.overall))?;
        output.line(&format!("Daemon: {}", self.daemon.state))?;
        output.line(&format!("  LaunchAgent: {}", self.daemon.launch_agent))?;
        output.line(&format!("  Socket: {}", self.daemon.socket))?;
        output.line("Integrations:")?;
        output.line(&format!("  DNS: {}", self.integrations.dns))?;
        output.line(&format!(
            "  Ports: {}",
            self.integrations.ports.state.as_str()
        ))?;
        if !self.integrations.ports.is_active() {
            output.line("    repair: `pv ports:install`")?;
        }
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
                    project.display_name(),
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
                    "  {} {} {} failed at {}: {}",
                    job.id,
                    job.kind,
                    job.scope,
                    job.finished_at.as_deref().unwrap_or(&job.started_at),
                    job.error.as_deref().unwrap_or("-"),
                ))?;
            }
        }

        Ok(())
    }

    /// The terminal report: the daemon, then one section per area, then the
    /// overall outcome. It carries the same facts as the plain report.
    fn write_decorated(&self, output: &mut Output<'_>) -> Result<(), ExecuteError> {
        output.heading("status", None)?;
        output.line("")?;
        output.status(
            self.daemon.mark,
            Line::default().toned(Tone::Strong, format!("Daemon {}", self.daemon.state)),
        )?;
        output.detail(
            Line::field("LaunchAgent ", self.daemon.launch_agent)
                .text("  ·  Socket ")
                .value(self.daemon.socket),
        )?;

        output.section("Integrations")?;
        let integration = |name: &str, state: &str| Line::from(format!("{name:5}  ")).text(state);
        let ports = &self.integrations.ports;
        output.status(
            self.integrations.dns_mark,
            integration("DNS", self.integrations.dns),
        )?;
        output.status(
            self.integrations.ports_mark,
            integration("Ports", ports.state.as_str()),
        )?;
        if !ports.is_active() {
            output.hint("repair", "pv ports:install")?;
        }
        output.status(
            self.integrations.ca_mark,
            integration("CA", self.integrations.ca),
        )?;

        output.section("Managed Resources")?;
        if self.managed_resources.is_empty() {
            output.note("none")?;
        } else {
            let mut table = Table::new(&["Resource", "Track", "State", "Projects", "Version"]);
            for resource in &self.managed_resources {
                table.row(vec![
                    Line::from(resource.name.as_str()),
                    Line::from(resource.track.as_str()),
                    Line::marked(resource.mark, resource.status),
                    Line::from(resource.projects.to_string()),
                    Line::default().value(resource.version.as_deref().unwrap_or("-")),
                ]);
            }
            output.table(&table)?;
        }

        if !self.runtimes.is_empty() {
            output.section("Runtimes")?;
            for runtime in &self.runtimes {
                output.status(
                    runtime.mark,
                    Line::from(format!("{}  ", runtime.subject)).text(runtime.status),
                )?;
                if let Some(message) = &runtime.message {
                    output.detail(message)?;
                }
            }
        }

        output.section("Projects")?;
        if self.projects.is_empty() {
            output.note("none")?;
        }
        for project in &self.projects {
            output.status(
                project.mark,
                Line::from(format!("{}  env ", project.display_name())).text(project.env_status),
            )?;
            if let Some(message) = &project.message {
                output.detail(message)?;
            }
        }

        output.section("Recent errors")?;
        if self.recent_errors.is_empty() {
            output.note("none")?;
        }
        for job in &self.recent_errors {
            output.failure(Line::default().value(job.id.as_str()).text(format!(
                " {} {} failed at {}",
                job.kind,
                job.scope,
                job.finished_at.as_deref().unwrap_or(&job.started_at),
            )))?;
            if let Some(error) = &job.error {
                output.detail(error)?;
            }
        }

        output.line("")?;
        let overall_mark = if self.has_failure() {
            Mark::Failure
        } else {
            Mark::Success
        };
        output.status(overall_mark, format!("Overall: {}", self.overall))?;
        output.hint("logs", &self.log_directory)?;

        Ok(())
    }
}

/// A runtime's mark. Status counts a degraded runtime as a failure, so it is
/// marked as one, as doctor does.
fn runtime_mark(status: Option<RuntimeObservedStatus>) -> Mark {
    match status {
        Some(RuntimeObservedStatus::Running) => Mark::Running,
        Some(RuntimeObservedStatus::Degraded | RuntimeObservedStatus::Failed) => Mark::Failure,
        Some(RuntimeObservedStatus::Pending | RuntimeObservedStatus::Stopped) | None => Mark::Idle,
    }
}

#[derive(Serialize)]
struct DaemonStatus {
    state: &'static str,
    launch_agent: &'static str,
    socket: &'static str,
    failure: bool,
    #[serde(skip)]
    mark: Mark,
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
        let mark = match state {
            "running" => Mark::Running,
            "disabled" => Mark::Idle,
            "down" | "repair-required" | "socket-stale" => Mark::Failure,
            _ => Mark::Warning,
        };

        Ok(Self {
            state,
            launch_agent: launch_agent_status,
            socket,
            failure: mark == Mark::Failure,
            mark,
        })
    }
}

#[derive(Serialize)]
struct IntegrationStatuses {
    dns: &'static str,
    ports: PfRoutingDiagnostic,
    ca: &'static str,
    #[serde(skip)]
    failure: bool,
    #[serde(skip)]
    dns_mark: Mark,
    #[serde(skip)]
    ports_mark: Mark,
    #[serde(skip)]
    ca_mark: Mark,
}

impl IntegrationStatuses {
    fn read(
        environment: &impl Environment,
        paths: &PvPaths,
        database: Option<&Database>,
        low_port_routing_required: bool,
    ) -> Result<Self, ExecuteError> {
        let prepared_resolver = platform::inspect_resolver_file(&paths.resolver_config(), None);
        let expected_resolver = resolver_config_from_state(&prepared_resolver);
        let system_resolver_path = resolver_test_path(environment)?;
        let system_resolver =
            environment.inspect_resolver_file(&system_resolver_path, expected_resolver.as_ref());
        let (dns, dns_mark) = resolver_status(&prepared_resolver, &system_resolver);

        let ports = PfRoutingDiagnostic::read(environment, paths, database)?;
        // Low-port routing is only required while the daemon is enabled.
        let ports_mark = match ports.state {
            PfRoutingState::Active => Mark::Success,
            _ if low_port_routing_required => Mark::Failure,
            PfRoutingState::Inactive => Mark::Idle,
            PfRoutingState::Drifted | PfRoutingState::Unknown => Mark::Warning,
        };

        let local_ca =
            platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
        let local_metadata = metadata_from_local_ca(&local_ca);
        let trust = ca_trust_state(environment, local_metadata.as_ref());
        let (ca, ca_mark) = ca_status(&local_ca, &trust);

        Ok(Self {
            dns,
            ports,
            ca,
            failure: [dns_mark, ports_mark, ca_mark].contains(&Mark::Failure),
            dns_mark,
            ports_mark,
            ca_mark,
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
    #[serde(skip)]
    mark: Mark,
}

#[derive(Serialize)]
struct RuntimeStatus {
    subject: String,
    status: &'static str,
    message: Option<String>,
    observed_at: String,
    failure: bool,
    #[serde(skip)]
    mark: Mark,
}

#[derive(Serialize)]
struct ProjectStatus {
    mode: &'static str,
    slug: String,
    hostname: Option<String>,
    env_status: &'static str,
    message: Option<String>,
    observed_at: Option<String>,
    #[serde(skip)]
    mark: Mark,
}

impl ProjectStatus {
    fn display_name(&self) -> &str {
        if self.mode == "resource-only" {
            return &self.slug;
        }

        self.hostname.as_deref().unwrap_or(&self.slug)
    }
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
    let mark = runtime_mark(runtime_status);

    ManagedResourceStatus {
        name: track.resource_name,
        track: track.track,
        desired: desired_state_label(track.desired_state),
        status,
        projects: track.usage_count,
        version: track.installed_version,
        failure: mark == Mark::Failure,
        mark,
    }
}

fn runtime_statuses(runtime_states: &[RuntimeObservedStateRecord]) -> Vec<RuntimeStatus> {
    runtime_states
        .iter()
        .filter_map(|state| match &state.subject {
            RuntimeSubject::Gateway
            | RuntimeSubject::PhpWorker { .. }
            | RuntimeSubject::PhpRuntimeWorker { .. } => {
                let mark = runtime_mark(Some(state.status));
                Some(RuntimeStatus {
                    subject: runtime_subject_label(&state.subject),
                    status: runtime_status_label(state.status),
                    message: state.message.clone(),
                    observed_at: state.observed_at.clone(),
                    failure: mark == Mark::Failure,
                    mark,
                })
            }
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
            mode: project.mode.as_str(),
            slug: project.slug,
            hostname: project.primary_hostname,
            env_status: "pending",
            message: Some("Project env has not been observed yet".to_string()),
            observed_at: None,
            mark: Mark::Idle,
        };
    };
    let env_status = project_env_status_label(observed.status);
    let mark = match observed.status {
        ProjectEnvObservedStatus::Pending => Mark::Idle,
        ProjectEnvObservedStatus::Rendered => Mark::Success,
        ProjectEnvObservedStatus::Warning => Mark::Warning,
        ProjectEnvObservedStatus::Failed => Mark::Failure,
    };
    let message = if observed.status == ProjectEnvObservedStatus::Warning {
        project_env_warning_message(&observed)
    } else {
        observed.message
    };

    ProjectStatus {
        mode: project.mode.as_str(),
        slug: project.slug,
        hostname: project.primary_hostname,
        env_status,
        message,
        observed_at: Some(observed.observed_at),
        mark,
    }
}

fn project_env_warning_message(observed: &state::ProjectEnvObservedStateRecord) -> Option<String> {
    let ignored = observed
        .warnings
        .iter()
        .filter(|warning| warning.kind == "ignored_php_extension")
        .map(|warning| warning.message.as_str())
        .collect::<Vec<_>>();
    if !ignored.is_empty() {
        return Some(ignored.join("; "));
    }

    observed
        .warnings
        .first()
        .map(|warning| warning.message.clone())
        .or_else(|| observed.message.clone())
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
) -> (&'static str, Mark) {
    match prepared {
        ResolverFileState::Missing { .. } => ("missing", Mark::Idle),
        ResolverFileState::Current { .. } => match system {
            ResolverFileState::Current { .. } => ("current", Mark::Success),
            ResolverFileState::Missing { .. } => ("prepared-only", Mark::Failure),
            ResolverFileState::Stale { .. } => ("stale", Mark::Failure),
            ResolverFileState::Conflict { .. } => ("conflict", Mark::Failure),
            ResolverFileState::Unreadable { .. } => ("unreadable", Mark::Failure),
        },
        ResolverFileState::Stale { .. } => ("stale", Mark::Failure),
        ResolverFileState::Conflict { .. } => ("conflict", Mark::Failure),
        ResolverFileState::Unreadable { .. } => ("unreadable", Mark::Failure),
    }
}

fn ca_status(state: &CaFileState, trust: &TrustDomainState) -> (&'static str, Mark) {
    match state {
        CaFileState::Missing { .. } => ("missing", Mark::Idle),
        CaFileState::Current { .. } => match trust {
            TrustDomainState::Current { .. } => ("current", Mark::Success),
            TrustDomainState::NotTrusted { .. } => ("not-trusted", Mark::Failure),
            TrustDomainState::Stale { .. } => ("stale", Mark::Failure),
            TrustDomainState::Denied { .. } => ("denied", Mark::Failure),
            TrustDomainState::Unknown { .. } => ("unknown", Mark::Failure),
            TrustDomainState::Unreadable { .. } => ("unreadable", Mark::Failure),
        },
        CaFileState::RepairRequired { .. } => ("repair-required", Mark::Failure),
        CaFileState::Unreadable { .. } => ("unreadable", Mark::Failure),
    }
}

fn runtime_subject_label(subject: &RuntimeSubject) -> String {
    match subject {
        RuntimeSubject::Gateway => "gateway".to_string(),
        RuntimeSubject::PhpWorker { php_track } => format!("worker:{php_track}"),
        RuntimeSubject::PhpRuntimeWorker { php_runtime_key } => {
            format!("worker:{php_runtime_key}")
        }
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
