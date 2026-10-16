use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{
    CaFileState, DaemonRegistrationFileState, LocalCaMetadata, PlatformTarget, ResolverConfig,
    ResolverFileState, TrustDomainState, daemon_registration_name,
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

use super::linux_distribution::LinuxDistribution;
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
    #[serde(skip)]
    target: PlatformTarget,
    overall: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    linux_distribution: Option<LinuxDistribution>,
    #[serde(skip)]
    system_failure: bool,
    daemon: DaemonStatus,
    integrations: IntegrationStatuses,
    managed_resources: Vec<ManagedResourceStatus>,
    runtimes: Vec<RuntimeStatus>,
    projects: Vec<ProjectStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    worktree_cleanups: Vec<WorktreeCleanupStatus>,
    recent_errors: Vec<JobStatusSummary>,
    log_directory: String,
}

/// An allocation object of a removed worktree that PV hasn't deleted yet.
#[derive(Serialize)]
struct WorktreeCleanupStatus {
    resource: String,
    track: String,
    name: String,
    error: Option<String>,
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
        let worktree_cleanups = match &database {
            Some(database) => database
                .pending_cleanups()?
                .into_iter()
                .map(|cleanup| WorktreeCleanupStatus {
                    resource: cleanup.resource_name,
                    track: cleanup.track,
                    name: cleanup.generated_name,
                    error: cleanup.last_error,
                })
                .collect(),
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
        let system_failure = daemon.failure
            || integrations.failure
            || managed_resources.iter().any(|resource| resource.failure)
            || runtimes.iter().any(|runtime| runtime.failure)
            || !recent_errors.is_empty();
        let has_failure =
            system_failure || projects.iter().any(|project| project.mark == Mark::Failure);
        let overall = if has_failure { "failed" } else { "ok" };
        let target = environment.platform_target()?;

        Ok(Self {
            target,
            overall,
            linux_distribution: (target == PlatformTarget::Linux)
                .then(|| LinuxDistribution::read(environment)),
            system_failure,
            daemon,
            integrations,
            managed_resources,
            runtimes,
            projects,
            worktree_cleanups,
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
        let registration_name = daemon_registration_name(self.target);
        output.line("PV status")?;
        if let Some(distribution) = &self.linux_distribution {
            distribution.write(output)?;
        }
        output.line(&format!("Overall: {}", self.overall))?;
        output.line(&format!("Daemon: {}", self.daemon.state))?;
        output.line(&format!(
            "  {registration_name}: {}",
            self.daemon.registration
        ))?;
        output.line(&format!("  Socket: {}", self.daemon.socket))?;
        #[cfg(target_os = "linux")]
        if let Some(detail) = self.daemon.registration_detail() {
            output.line(&format!("  {detail}"))?;
        }
        output.line("Integrations:")?;
        output.line(&format!("  DNS: {}", self.integrations.dns))?;
        let ports = &self.integrations.ports;
        output.line(&format!("  Ports: {}", ports.state.as_str()))?;
        if ports.is_active()
            && let (Some(http), Some(https)) = (ports.active_http_port, ports.active_https_port)
        {
            output.line(&format!("    HTTP 80 -> {http}, HTTPS 443 -> {https}"))?;
        }
        if !ports.is_active() && self.integrations.ports_mark != Mark::Idle {
            output.line("    repair: pv ports:install")?;
        }
        output.line(&format!("  CA: {}", self.integrations.ca))?;
        for report in &self.integrations.browser_ca {
            output.line(&format!(
                "  Browser CA: {} · {}",
                report.database,
                report.label()
            ))?;
            if let Some(detail) = report.detail() {
                output.line(&format!("    {detail}"))?;
            }
        }
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
        if !self.worktree_cleanups.is_empty() {
            output.line("Worktree cleanup:")?;
            for cleanup in &self.worktree_cleanups {
                output.line(&format!(
                    "  {} {} {} {}",
                    cleanup.resource,
                    cleanup.track,
                    cleanup.name,
                    cleanup.error.as_deref().unwrap_or("pending"),
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
        let registration_name = daemon_registration_name(self.target);
        output.report_heading("status", None)?;
        if let Some(distribution) = &self.linux_distribution {
            distribution.write(output)?;
        }
        output.line("")?;
        let project_attention = self
            .projects
            .iter()
            .filter(|project| matches!(project.mark, Mark::Warning | Mark::Failure))
            .count();
        let mut summary = format!("PV is {}", self.daemon.state);
        if self.system_failure {
            summary.push_str(" · system needs attention");
        }
        if project_attention > 0 {
            summary.push_str(&format!(
                " · {project_attention} Project{} need{} attention",
                if project_attention == 1 { "" } else { "s" },
                if project_attention == 1 { "s" } else { "" },
            ));
        }
        let mark = if self.has_failure() {
            Mark::Failure
        } else if project_attention > 0 {
            Mark::Warning
        } else {
            self.daemon.mark
        };
        output.status(mark, Line::default().toned(Tone::Strong, summary.clone()))?;
        let gateway = self
            .runtimes
            .iter()
            .find(|runtime| runtime.subject == "gateway");
        let daemon_line = if let Some(gateway) = gateway.filter(|_| self.daemon.state == "running")
        {
            format!("Daemon {} · Gateway {}", self.daemon.state, gateway.status)
        } else {
            format!("Daemon {}", self.daemon.state)
        };
        output.status(
            gateway.map_or(self.daemon.mark, |gateway| {
                if self.daemon.state != "running" {
                    self.daemon.mark
                } else {
                    gateway.mark
                }
            }),
            Line::default().toned(Tone::Strong, daemon_line),
        )?;
        if self.daemon.state == "running"
            && let Some(message) = gateway.and_then(|gateway| gateway.message.as_deref())
        {
            output.detail(message)?;
        }
        output.detail(
            Line::field(&format!("{registration_name} "), self.daemon.registration)
                .text("  ·  Socket ")
                .value(self.daemon.socket),
        )?;
        #[cfg(target_os = "linux")]
        if let Some(detail) = self.daemon.registration_detail() {
            output.detail(detail)?;
        }

        output.section("Integrations")?;
        let integration = |name: &str, state: &str| Line::from(format!("{name:5}  ")).text(state);
        let ports = &self.integrations.ports;
        output.status(
            self.integrations.dns_mark,
            integration("DNS", self.integrations.dns),
        )?;
        output.status(
            self.integrations.ports_mark,
            if ports.is_active() {
                Line::from("Ports active · HTTP 80 / HTTPS 443")
            } else {
                integration("Ports", ports.state.as_str())
            },
        )?;
        if ports.is_active()
            && let (Some(http), Some(https)) = (ports.active_http_port, ports.active_https_port)
        {
            output.detail(format!("80→{http}  ·  443→{https}"))?;
        }
        if !ports.is_active() && self.integrations.ports_mark != Mark::Idle {
            output.hint("repair", "pv ports:install")?;
        }
        output.status(
            self.integrations.ca_mark,
            integration("CA", self.integrations.ca),
        )?;

        for report in &self.integrations.browser_ca {
            output.status(
                if report.current() {
                    Mark::Success
                } else {
                    Mark::Failure
                },
                Line::field("Browser CA: ", &report.database)
                    .text(format!(" · {}", report.label())),
            )?;
            if let Some(detail) = report.detail() {
                output.detail(detail)?;
            }
        }

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

        let other_runtimes = self
            .runtimes
            .iter()
            .filter(|runtime| self.daemon.state != "running" || runtime.subject != "gateway")
            .collect::<Vec<_>>();
        if !other_runtimes.is_empty() {
            output.section("Runtimes")?;
            for runtime in other_runtimes {
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

        if !self.worktree_cleanups.is_empty() {
            output.section("Worktree cleanup")?;
            for cleanup in &self.worktree_cleanups {
                let mark = if cleanup.error.is_some() {
                    Mark::Warning
                } else {
                    Mark::Idle
                };
                output.status(
                    mark,
                    Line::from(format!("{} {}  ", cleanup.resource, cleanup.track))
                        .value(cleanup.name.as_str()),
                )?;
                if let Some(error) = &cleanup.error {
                    output.detail(error)?;
                }
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
        output.status(mark, summary)?;
        output.detail(Line::field("Logs: ", &self.log_directory))?;

        Ok(())
    }
}

#[derive(Serialize)]
struct DaemonStatus {
    state: &'static str,
    #[cfg_attr(not(target_os = "linux"), serde(rename = "launch_agent"))]
    registration: &'static str,
    #[cfg(target_os = "linux")]
    #[serde(skip_serializing_if = "Option::is_none")]
    registration_runtime: Option<platform::DaemonRegistrationStatus>,
    #[cfg(target_os = "linux")]
    #[serde(skip_serializing_if = "Option::is_none")]
    registration_error: Option<String>,
    socket: &'static str,
    failure: bool,
    #[serde(skip)]
    mark: Mark,
}

impl DaemonStatus {
    fn read(environment: &impl Environment, paths: &PvPaths) -> Result<Self, ExecuteError> {
        let daemon_registration_path =
            Utf8PathBuf::from_path_buf(environment.daemon_registration_path())
                .map_err(|path| crate::error::CliError::NonUtf8Path { path })?;
        let registration =
            platform::inspect_daemon_registration_file(&daemon_registration_path, None);
        let daemon_registration_file_status = daemon_registration_file_status(&registration);
        let socket_exists = state::fs::path_exists(&paths.daemon_socket());
        let socket = if !socket_exists {
            "missing"
        } else if daemon::health_blocking(paths.clone()).is_ok() {
            "healthy"
        } else {
            "unhealthy"
        };
        let (state, mark) = match &registration {
            DaemonRegistrationFileState::Missing { .. } if socket == "missing" => {
                ("disabled", Mark::Idle)
            }
            DaemonRegistrationFileState::Missing { .. } if socket == "healthy" => {
                ("socket-only", Mark::Warning)
            }
            DaemonRegistrationFileState::Missing { .. } => ("socket-stale", Mark::Failure),
            DaemonRegistrationFileState::Current { .. } if socket == "healthy" => {
                ("running", Mark::Running)
            }
            DaemonRegistrationFileState::Current { .. } => ("down", Mark::Failure),
            DaemonRegistrationFileState::Stale { .. }
            | DaemonRegistrationFileState::Conflict { .. } => ("repair-required", Mark::Failure),
            DaemonRegistrationFileState::Unreadable { .. } => ("unknown", Mark::Warning),
        };

        #[cfg(target_os = "linux")]
        let (registration_runtime, registration_error) =
            match environment.inspect_daemon_registration() {
                Ok(status) => (status, None),
                Err(error) => (None, Some(error.to_string())),
            };
        #[cfg(target_os = "linux")]
        let registration_error = registration_error.or_else(|| {
            registration_runtime.as_ref().and_then(|status| {
                match platform::inspect_daemon_registration_file(&status.fragment_path, None) {
                    DaemonRegistrationFileState::Current { .. }
                    | DaemonRegistrationFileState::Stale { .. } => None,
                    DaemonRegistrationFileState::Unreadable { message, .. } => Some(message),
                    _ => Some(format!(
                        "systemd loaded a service that is not PV-owned: {}",
                        status.fragment_path
                    )),
                }
            })
        });
        #[cfg(target_os = "linux")]
        let (state, mark) = if registration_error.is_some() {
            ("unknown", Mark::Failure)
        } else if matches!(registration, DaemonRegistrationFileState::Current { .. })
            && !registration_runtime.as_ref().is_some_and(|status| {
                status.load_state == "loaded"
                    && status.unit_file_state.as_deref() == Some("enabled")
                    && status.active_state == "active"
                    && status.main_pid != 0
            })
        {
            ("down", Mark::Failure)
        } else {
            (state, mark)
        };

        Ok(Self {
            state,
            registration: daemon_registration_file_status,
            #[cfg(target_os = "linux")]
            registration_runtime,
            #[cfg(target_os = "linux")]
            registration_error,
            socket,
            failure: mark == Mark::Failure,
            mark,
        })
    }

    #[cfg(target_os = "linux")]
    fn registration_detail(&self) -> Option<String> {
        if let Some(error) = &self.registration_error {
            return Some(error.clone());
        }
        self.registration_runtime.as_ref().map(|status| {
            format!(
                "systemd: load={} enablement={} state={}/{} pid={} fragment={}",
                status.load_state,
                status.unit_file_state.as_deref().unwrap_or("unavailable"),
                status.active_state,
                status.sub_state,
                status.main_pid,
                status.fragment_path,
            )
        })
    }
}

#[derive(Serialize)]
struct IntegrationStatuses {
    dns: &'static str,
    ports: PfRoutingDiagnostic,
    ca: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    browser_ca: Vec<super::ca::BrowserCaReport>,
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
        let (ca, mut ca_mark) = ca_status(&local_ca, &trust);
        let browser_ca =
            super::ca::browser_ca_reports(environment, paths, local_metadata.as_ref())?;
        if browser_ca.iter().any(|report| !report.current()) {
            ca_mark = Mark::Failure;
        }

        Ok(Self {
            dns,
            ports,
            ca,
            browser_ca,
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
    let mark = super::runtime_mark(runtime_status);

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
                let mark = super::runtime_mark(Some(state.status));
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

fn daemon_registration_file_status(state: &DaemonRegistrationFileState) -> &'static str {
    match state {
        DaemonRegistrationFileState::Missing { .. } => "missing",
        DaemonRegistrationFileState::Current { .. } => "current",
        DaemonRegistrationFileState::Stale { .. } => "stale",
        DaemonRegistrationFileState::Conflict { .. } => "conflict",
        DaemonRegistrationFileState::Unreadable { .. } => "unreadable",
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

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::super::pf_diagnostics::PfRoutingEvidence;
    use super::*;
    use crate::output::Surface;

    #[test]
    fn running_status_shows_gateway_ports_and_project_attention() -> anyhow::Result<()> {
        let mut snapshot = StatusSnapshot {
            target: PlatformTarget::Macos,
            overall: "ok",
            linux_distribution: None,
            system_failure: false,
            daemon: DaemonStatus {
                state: "running",
                registration: "current",
                #[cfg(target_os = "linux")]
                registration_runtime: None,
                #[cfg(target_os = "linux")]
                registration_error: None,
                socket: "healthy",
                failure: false,
                mark: Mark::Running,
            },
            integrations: IntegrationStatuses {
                dns: "active",
                ports: PfRoutingDiagnostic {
                    state: PfRoutingState::Active,
                    evidence: PfRoutingEvidence::Pfctl,
                    expected_http_port: Some(48080),
                    expected_https_port: Some(48443),
                    active_http_port: Some(48080),
                    active_https_port: Some(48443),
                    observed_at: "2026-09-26T12:00:00Z".to_string(),
                },
                ca: "trusted",
                browser_ca: Vec::new(),
                failure: false,
                dns_mark: Mark::Success,
                ports_mark: Mark::Success,
                ca_mark: Mark::Success,
            },
            managed_resources: Vec::new(),
            runtimes: vec![RuntimeStatus {
                subject: "gateway".to_string(),
                status: "running",
                message: None,
                observed_at: "2026-09-26T12:00:00Z".to_string(),
                failure: false,
                mark: Mark::Running,
            }],
            projects: vec![ProjectStatus {
                mode: "served",
                slug: "acme".to_string(),
                hostname: Some("acme.test".to_string()),
                env_status: "warning",
                message: Some("APP_URL already exists outside the PV block".to_string()),
                observed_at: Some("2026-09-26T12:00:00Z".to_string()),
                mark: Mark::Warning,
            }],
            worktree_cleanups: vec![
                WorktreeCleanupStatus {
                    resource: "mysql".to_string(),
                    track: "8.0".to_string(),
                    name: "acme_feat_app".to_string(),
                    error: Some(
                        "`mysql` track `8.0` isn't installed, so PV can't delete this object; \
                         reinstall the track, or uninstall it with --prune to delete its data"
                            .to_string(),
                    ),
                },
                WorktreeCleanupStatus {
                    resource: "redis".to_string(),
                    track: "8.8".to_string(),
                    name: "acme-feat.cache.".to_string(),
                    error: None,
                },
            ],
            recent_errors: Vec::new(),
            log_directory: "/home/acme/.pv/logs".to_string(),
        };
        let mut bytes = Vec::new();
        snapshot.write(&mut Output::new(&mut bytes, Surface::terminal(false, 100)))?;

        assert_snapshot!(String::from_utf8(bytes)?);

        let mut bytes = Vec::new();
        snapshot.write(&mut Output::new(&mut bytes, Surface::plain()))?;
        assert_snapshot!(
            "running_status_shows_active_ports_in_plain_output",
            String::from_utf8(bytes)?
        );

        snapshot.target = PlatformTarget::Linux;
        let mut bytes = Vec::new();
        snapshot.write(&mut Output::new(&mut bytes, Surface::terminal(false, 100)))?;
        assert_snapshot!(
            "linux_status_shows_registration_in_decorated_output",
            String::from_utf8(bytes)?
        );
        let mut bytes = Vec::new();
        snapshot.write(&mut Output::new(&mut bytes, Surface::plain()))?;
        assert_snapshot!(
            "linux_status_shows_registration_in_plain_output",
            String::from_utf8(bytes)?
        );
        snapshot.target = PlatformTarget::Macos;

        snapshot.overall = "failed";
        snapshot.system_failure = true;
        snapshot.runtimes[0].status = "failed";
        snapshot.runtimes[0].message =
            Some("Gateway failed to start: port 48443 is occupied".to_string());
        snapshot.runtimes[0].failure = true;
        snapshot.runtimes[0].mark = Mark::Failure;
        let mut bytes = Vec::new();
        snapshot.write(&mut Output::new(&mut bytes, Surface::terminal(false, 100)))?;
        assert_snapshot!(
            "running_status_shows_gateway_failure",
            String::from_utf8(bytes)?
        );

        Ok(())
    }
}
