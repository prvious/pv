use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{CaFileState, LaunchAgentFileState, PfFileState, ResolverFileState};
use state::{Database, JobStatus, PvPaths, RuntimeObservedStatus, StateError};

use crate::args::DoctorArgs;
use crate::environment::Environment;
use crate::error::CliError;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

pub(crate) fn run(
    _args: DoctorArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let report = DoctorReport::read(environment)?;
    let exit_code = if report.has_failures() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };
    let mut output = Output::new(stdout, OutputMode::plain());
    report.write_plain(&mut output)?;

    Ok(exit_code)
}

struct DoctorReport {
    checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    fn read(environment: &impl Environment) -> Result<Self, ExecuteError> {
        let paths = pv_paths(environment)?;
        let database = Database::open_read_only(&paths)?;
        let checks = vec![
            layout_check(&paths),
            database_check(&paths, database.as_ref()),
            launch_agent_check(environment)?,
            daemon_socket_check(&paths, environment)?,
            dns_check(&paths),
            ports_check(&paths),
            ca_check(&paths),
            recent_jobs_check(database.as_ref())?,
            runtime_states_check(database.as_ref())?,
            manifest_cache_check(&paths),
        ];

        Ok(Self { checks })
    }

    fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == CheckStatus::Fail)
    }

    fn write_plain(&self, output: &mut Output<'_, impl Write>) -> Result<(), ExecuteError> {
        output.line("PV doctor")?;
        for check in &self.checks {
            output.line(&format!(
                "[{}] {}: {}",
                check.status.as_str(),
                check.name,
                check.message
            ))?;
            if let Some(detail) = &check.detail {
                output.line(&format!("  {detail}"))?;
            }
            if let Some(repair) = &check.repair {
                output.line(&format!("  repair: `{repair}`"))?;
            }
        }

        let passed = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Pass)
            .count();
        let warnings = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Warn)
            .count();
        let failures = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Fail)
            .count();

        output.line(&format!(
            "Summary: {passed} passed, {warnings} warning(s), {failures} failed"
        ))?;

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

struct DoctorCheck {
    status: CheckStatus,
    name: &'static str,
    message: String,
    detail: Option<String>,
    repair: Option<&'static str>,
}

impl DoctorCheck {
    fn pass(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            name,
            message: message.into(),
            detail: None,
            repair: None,
        }
    }

    fn warn(name: &'static str, message: impl Into<String>, repair: Option<&'static str>) -> Self {
        Self {
            status: CheckStatus::Warn,
            name,
            message: message.into(),
            detail: None,
            repair,
        }
    }

    fn fail(name: &'static str, message: impl Into<String>, repair: &'static str) -> Self {
        Self {
            status: CheckStatus::Fail,
            name,
            message: message.into(),
            detail: None,
            repair: Some(repair),
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

fn layout_check(paths: &PvPaths) -> DoctorCheck {
    if !state::fs::path_exists(paths.root()) {
        return DoctorCheck::fail("State layout", "missing ~/.pv state directory", "pv setup")
            .with_detail(format!("path: {}", paths.root()));
    }

    match state::fs::inspect_layout(paths) {
        Ok(entries) => DoctorCheck::pass(
            "State layout",
            format!(
                "{} PV-owned directories have user-only permissions",
                entries.len()
            ),
        ),
        Err(error) => DoctorCheck::fail(
            "State layout",
            "state layout could not be inspected safely",
            "pv setup",
        )
        .with_detail(error.to_string()),
    }
}

fn database_check(paths: &PvPaths, database: Option<&Database>) -> DoctorCheck {
    let Some(database) = database else {
        return DoctorCheck::fail("Database", "pv.db is missing", "pv setup")
            .with_detail(format!("path: {}", paths.db()));
    };

    match database.inspect() {
        Ok(inspection) => DoctorCheck::pass(
            "Database",
            format!(
                "read-only open succeeded; {} migrations applied",
                inspection.migrations.len()
            ),
        ),
        Err(error) => DoctorCheck::fail("Database", "pv.db could not be inspected", "pv setup")
            .with_detail(error.to_string()),
    }
}

fn launch_agent_check(environment: &impl Environment) -> Result<DoctorCheck, ExecuteError> {
    let launch_agent_path = launch_agent_path(environment)?;
    let state = platform::inspect_launch_agent_file(&launch_agent_path, None);
    let check = match state {
        LaunchAgentFileState::Current { path, .. } => {
            DoctorCheck::pass("Daemon LaunchAgent", "PV-owned LaunchAgent is installed")
                .with_detail(format!("path: {path}"))
        }
        LaunchAgentFileState::Missing { path } => DoctorCheck::fail(
            "Daemon LaunchAgent",
            "LaunchAgent is missing",
            "pv daemon:enable",
        )
        .with_detail(format!("path: {path}")),
        LaunchAgentFileState::Stale { path, .. } => DoctorCheck::fail(
            "Daemon LaunchAgent",
            "LaunchAgent is PV-owned but stale",
            "pv daemon:restart",
        )
        .with_detail(format!("path: {path}")),
        LaunchAgentFileState::Conflict { path } => DoctorCheck::fail(
            "Daemon LaunchAgent",
            "LaunchAgent file is not PV-owned",
            "pv daemon:enable",
        )
        .with_detail(format!("path: {path}")),
        LaunchAgentFileState::Unreadable { path, message } => DoctorCheck::fail(
            "Daemon LaunchAgent",
            "LaunchAgent file could not be inspected",
            "pv daemon:enable",
        )
        .with_detail(format!("{path}: {message}")),
    };

    Ok(check)
}

fn daemon_socket_check(
    paths: &PvPaths,
    environment: &impl Environment,
) -> Result<DoctorCheck, ExecuteError> {
    let repair = match platform::inspect_launch_agent_file(&launch_agent_path(environment)?, None) {
        LaunchAgentFileState::Current { .. } => "pv daemon:restart",
        _ => "pv daemon:enable",
    };

    if state::fs::path_exists(&paths.daemon_socket()) {
        return Ok(
            DoctorCheck::pass("Daemon socket", "daemon socket path is present")
                .with_detail(format!("path: {}", paths.daemon_socket())),
        );
    }

    Ok(
        DoctorCheck::fail("Daemon socket", "daemon socket is missing", repair)
            .with_detail(format!("path: {}", paths.daemon_socket())),
    )
}

fn dns_check(paths: &PvPaths) -> DoctorCheck {
    match platform::inspect_resolver_file(&paths.resolver_config(), None) {
        ResolverFileState::Current { port, .. } => {
            DoctorCheck::pass("DNS config", format!("prepared resolver uses port {port}"))
        }
        ResolverFileState::Missing { path } => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config is missing",
            "pv dns:install",
        )
        .with_detail(format!("path: {path}")),
        ResolverFileState::Stale { path, .. } => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config is PV-owned but stale",
            "pv dns:install",
        )
        .with_detail(format!("path: {path}")),
        ResolverFileState::Conflict { path } => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config is not PV-owned",
            "pv dns:install",
        )
        .with_detail(format!("path: {path}")),
        ResolverFileState::Unreadable { path, message } => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config could not be inspected",
            "pv dns:install",
        )
        .with_detail(format!("{path}: {message}")),
    }
}

fn ports_check(paths: &PvPaths) -> DoctorCheck {
    let anchor = platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), None);
    let reference = platform::inspect_pf_conf_reference(&paths.pf_conf_reference_config(), None);

    match (&anchor, &reference) {
        (PfFileState::Current { .. }, PfFileState::Current { .. }) => {
            DoctorCheck::pass("Port redirect config", "prepared pf config is current")
        }
        (PfFileState::Missing { path }, _) | (_, PfFileState::Missing { path }) => {
            DoctorCheck::fail(
                "Port redirect config",
                "prepared pf config is missing",
                "pv ports:install",
            )
            .with_detail(format!("path: {path}"))
        }
        (PfFileState::Conflict { path }, _) | (_, PfFileState::Conflict { path }) => {
            DoctorCheck::fail(
                "Port redirect config",
                "prepared pf config is not PV-owned",
                "pv ports:install",
            )
            .with_detail(format!("path: {path}"))
        }
        (PfFileState::Unreadable { path, message }, _)
        | (_, PfFileState::Unreadable { path, message }) => DoctorCheck::fail(
            "Port redirect config",
            "prepared pf config could not be inspected",
            "pv ports:install",
        )
        .with_detail(format!("{path}: {message}")),
        (PfFileState::Stale { path, .. }, _) | (_, PfFileState::Stale { path, .. }) => {
            DoctorCheck::fail(
                "Port redirect config",
                "prepared pf config is PV-owned but stale",
                "pv ports:install",
            )
            .with_detail(format!("path: {path}"))
        }
    }
}

fn ca_check(paths: &PvPaths) -> DoctorCheck {
    match platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key()) {
        CaFileState::Current { metadata, .. } => DoctorCheck::pass(
            "Local CA files",
            format!("current certificate fingerprint {}", metadata.fingerprint),
        ),
        CaFileState::Missing {
            certificate_path,
            private_key_path,
        } => DoctorCheck::fail(
            "Local CA files",
            "local CA files are missing",
            "pv ca:trust",
        )
        .with_detail(format!(
            "certificate: {certificate_path}; private key: {private_key_path}"
        )),
        CaFileState::RepairRequired { reason, .. } => DoctorCheck::fail(
            "Local CA files",
            "local CA files require repair",
            "pv ca:trust",
        )
        .with_detail(format!("reason: {reason:?}")),
        CaFileState::Unreadable { path, message } => DoctorCheck::fail(
            "Local CA files",
            "local CA files could not be inspected",
            "pv ca:trust",
        )
        .with_detail(format!("{path}: {message}")),
    }
}

fn recent_jobs_check(database: Option<&Database>) -> Result<DoctorCheck, ExecuteError> {
    let Some(database) = database else {
        return Ok(DoctorCheck::warn(
            "Recent jobs",
            "skipped because pv.db is missing",
            Some("pv setup"),
        ));
    };
    let failed = database
        .recent_jobs()?
        .into_iter()
        .filter(|job| job.status == JobStatus::Failed)
        .collect::<Vec<_>>();

    if failed.is_empty() {
        return Ok(DoctorCheck::pass(
            "Recent jobs",
            "no failed jobs in recent history",
        ));
    }

    Ok(DoctorCheck::fail(
        "Recent jobs",
        format!("{} failed job(s) in recent history", failed.len()),
        "pv setup",
    )
    .with_detail(
        failed
            .into_iter()
            .map(|job| {
                format!(
                    "{} {} {}: {}",
                    job.id,
                    job.kind,
                    job.scope,
                    job.error.unwrap_or_else(|| "failed".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    ))
}

fn runtime_states_check(database: Option<&Database>) -> Result<DoctorCheck, ExecuteError> {
    let Some(database) = database else {
        return Ok(DoctorCheck::warn(
            "Runtime states",
            "skipped because pv.db is missing",
            Some("pv setup"),
        ));
    };
    let failed = database
        .runtime_observed_states()?
        .into_iter()
        .filter(|state| {
            matches!(
                state.status,
                RuntimeObservedStatus::Degraded | RuntimeObservedStatus::Failed
            )
        })
        .collect::<Vec<_>>();

    if failed.is_empty() {
        return Ok(DoctorCheck::pass(
            "Runtime states",
            "no degraded or failed runtime observations",
        ));
    }

    Ok(DoctorCheck::fail(
        "Runtime states",
        format!("{} degraded or failed runtime observation(s)", failed.len()),
        "pv daemon:restart",
    )
    .with_detail(
        failed
            .into_iter()
            .map(|state| {
                format!(
                    "{} {}",
                    runtime_subject_label(&state.subject),
                    state
                        .message
                        .unwrap_or_else(|| runtime_status_label(state.status).to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    ))
}

fn manifest_cache_check(paths: &PvPaths) -> DoctorCheck {
    let manifest_path = paths.downloads().join("manifest.json");
    if state::fs::path_exists(&manifest_path) {
        return DoctorCheck::pass("Artifact manifest cache", "cached manifest is present")
            .with_detail(format!("path: {manifest_path}"));
    }

    DoctorCheck::warn(
        "Artifact manifest cache",
        "cached artifact manifest is missing",
        Some("pv setup"),
    )
    .with_detail(format!("path: {manifest_path}"))
}

fn runtime_subject_label(subject: &state::RuntimeSubject) -> String {
    match subject {
        state::RuntimeSubject::Gateway => "gateway".to_string(),
        state::RuntimeSubject::PhpWorker { php_track } => format!("worker:{php_track}"),
        state::RuntimeSubject::Resource { name, track } => format!("{name}:{track}"),
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

fn launch_agent_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.launch_agent_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
