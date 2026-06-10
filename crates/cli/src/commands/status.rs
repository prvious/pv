use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{
    CaFileState, LaunchAgentFileState, PfConfReference, PfFileState, ResolverFileState,
};
use serde::Serialize;
use state::{
    Database, JobRecord, JobStatus, ManagedResourceDesiredState, ManagedResourceTrackRecord,
    PvPaths, RuntimeObservedStateRecord, RuntimeObservedStatus, RuntimeSubject, StateError,
};

use crate::args::StatusArgs;
use crate::environment::Environment;
use crate::error::ExecuteError;
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
    recent_errors: Vec<JobStatusSummary>,
    log_directory: String,
}

impl StatusSnapshot {
    fn read(environment: &impl Environment) -> Result<Self, ExecuteError> {
        let paths = pv_paths(environment)?;
        let database = Database::open(&paths)?;
        let daemon = DaemonStatus::read(environment, &paths)?;
        let integrations = IntegrationStatuses::read(&paths);
        let runtime_states = database.runtime_observed_states()?;
        let managed_resources = managed_resource_statuses(&database, &runtime_states)?;
        let runtimes = runtime_statuses(&runtime_states);
        let recent_errors = database
            .recent_jobs()?
            .into_iter()
            .filter(|job| job.status == JobStatus::Failed)
            .map(JobStatusSummary::from_job)
            .collect::<Vec<_>>();
        let has_failure = daemon.failure
            || managed_resources.iter().any(|resource| resource.failure)
            || runtimes.iter().any(|runtime| runtime.failure)
            || !recent_errors.is_empty();
        let overall = if has_failure { "failed" } else { "ok" };

        Ok(Self {
            overall,
            daemon,
            integrations,
            managed_resources,
            runtimes,
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
        let socket = if socket_exists { "present" } else { "missing" };
        let state = match (&launch_agent, socket_exists) {
            (LaunchAgentFileState::Missing { .. }, false) => "disabled",
            (LaunchAgentFileState::Current { .. }, true) => "running",
            (LaunchAgentFileState::Current { .. }, false) => "down",
            (LaunchAgentFileState::Missing { .. }, true) => "socket-only",
            (LaunchAgentFileState::Stale { .. }, _) => "repair-required",
            (LaunchAgentFileState::Conflict { .. }, _) => "repair-required",
            (LaunchAgentFileState::Unreadable { .. }, _) => "unknown",
        };
        let failure = matches!(state, "down" | "repair-required");

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
}

impl IntegrationStatuses {
    fn read(paths: &PvPaths) -> Self {
        let resolver = platform::inspect_resolver_file(&paths.resolver_config(), None);
        let pf_anchor = platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), None);
        let pf_reference =
            platform::inspect_pf_conf_reference(&paths.pf_conf_reference_config(), None);
        let ca = platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());

        Self {
            dns: resolver_status(&resolver),
            ports: pf_status(&pf_anchor, &pf_reference),
            ca: ca_status(&ca),
        }
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

fn launch_agent_status(state: &LaunchAgentFileState) -> &'static str {
    match state {
        LaunchAgentFileState::Missing { .. } => "missing",
        LaunchAgentFileState::Current { .. } => "current",
        LaunchAgentFileState::Stale { .. } => "stale",
        LaunchAgentFileState::Conflict { .. } => "conflict",
        LaunchAgentFileState::Unreadable { .. } => "unreadable",
    }
}

fn resolver_status(state: &ResolverFileState) -> &'static str {
    match state {
        ResolverFileState::Missing { .. } => "missing",
        ResolverFileState::Current { .. } => "current",
        ResolverFileState::Stale { .. } => "stale",
        ResolverFileState::Conflict { .. } => "conflict",
        ResolverFileState::Unreadable { .. } => "unreadable",
    }
}

fn pf_status(
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

fn ca_status(state: &CaFileState) -> &'static str {
    match state {
        CaFileState::Missing { .. } => "missing",
        CaFileState::Current { .. } => "current",
        CaFileState::RepairRequired { .. } => "repair-required",
        CaFileState::Unreadable { .. } => "unreadable",
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

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
