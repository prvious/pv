use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{
    CaFileState, LaunchAgentFileState, LocalCaMetadata, ResolverConfig, ResolverFileState,
    TrustDomainState,
};
use self_update::AppUpdateVersion;
use serde::Serialize;
use state::{Database, JobDiagnosticSubject, PvPaths, RuntimeObservedStatus, StateError};

use crate::args::DoctorArgs;
use crate::environment::Environment;
use crate::error::CliError;
use crate::error::ExecuteError;
use crate::helper_release::HelperReleaseMetadata;
use crate::output::{Output, OutputMode};

use super::pf_diagnostics::{PfRoutingDiagnostic, PfRoutingState};

pub(crate) fn run(
    args: DoctorArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let report = DoctorReport::read(environment)?;
    let exit_code = if report.has_failures() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };
    if args.json {
        serde_json::to_writer(&mut *stdout, &report)?;
        writeln!(stdout)?;

        return Ok(exit_code);
    }
    let mut output = Output::new(stdout, OutputMode::plain());
    report.write_plain(&mut output)?;

    Ok(exit_code)
}

#[derive(Serialize)]
struct DoctorReport {
    checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    fn read(environment: &impl Environment) -> Result<Self, ExecuteError> {
        let paths = pv_paths(environment)?;
        let database = Database::open_read_only(&paths)?;
        let launch_agent_path = launch_agent_path(environment)?;
        let launch_agent = platform::inspect_launch_agent_file(&launch_agent_path, None);
        let checks = vec![
            layout_check(&paths),
            database_check(&paths, database.as_ref()),
            privileged_helper_check(environment, &paths),
            launch_agent_check(&launch_agent),
            daemon_socket_check(&paths, &launch_agent),
            dns_check(environment, &paths)?,
            ports_check(environment, &paths, database.as_ref())?,
            ca_check(environment, &paths),
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

fn privileged_helper_check(environment: &impl Environment, paths: &PvPaths) -> DoctorCheck {
    let version = match AppUpdateVersion::current() {
        Ok(version) => version,
        Err(error) => {
            return DoctorCheck::fail(
                "Privileged helper",
                format!("could not determine the active PV version: {error}"),
                "reinstall PV, then run `pv setup`",
            );
        }
    };
    let expected = match HelperReleaseMetadata::read(&paths.app_release_helper(version.as_str())) {
        Ok(expected) => expected,
        Err(error) => {
            return DoctorCheck::fail(
                "Privileged helper",
                format!("active release helper metadata is invalid: {error}"),
                "reinstall PV, then run `pv setup`",
            );
        }
    };
    let expected_version = expected.version();
    let expected_protocol = expected.protocol_version();
    match environment.privileged_helper_status() {
        Ok(status)
            if status.version == expected_version
                && status.protocol_version == expected_protocol =>
        {
            DoctorCheck::pass(
                "Privileged helper",
                format!(
                    "available at version {} with protocol {}",
                    status.version, status.protocol_version
                ),
            )
        }
        Ok(status) => DoctorCheck::fail(
            "Privileged helper",
            format!(
                "version {} with protocol {} is incompatible",
                status.version, status.protocol_version
            ),
            "pv setup",
        ),
        Err(error) => DoctorCheck::fail(
            "Privileged helper",
            "helper is unavailable or incompatible",
            "pv setup",
        )
        .with_detail(error.to_string()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
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

#[derive(Serialize)]
struct DoctorCheck {
    status: CheckStatus,
    name: &'static str,
    message: String,
    detail: Option<String>,
    repair: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing: Option<PfRoutingDiagnostic>,
}

impl DoctorCheck {
    fn pass(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            name,
            message: message.into(),
            detail: None,
            repair: None,
            routing: None,
        }
    }

    fn warn(name: &'static str, message: impl Into<String>, repair: Option<&'static str>) -> Self {
        Self {
            status: CheckStatus::Warn,
            name,
            message: message.into(),
            detail: None,
            repair: repair.map(str::to_owned),
            routing: None,
        }
    }

    fn fail(name: &'static str, message: impl Into<String>, repair: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            name,
            message: message.into(),
            detail: None,
            repair: Some(repair.into()),
            routing: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn with_routing(mut self, routing: PfRoutingDiagnostic) -> Self {
        self.routing = Some(routing);
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

fn launch_agent_check(state: &LaunchAgentFileState) -> DoctorCheck {
    match state {
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
    }
}

fn daemon_socket_check(paths: &PvPaths, launch_agent: &LaunchAgentFileState) -> DoctorCheck {
    let repair = match launch_agent {
        LaunchAgentFileState::Current { .. } => "pv daemon:restart",
        _ => "pv daemon:enable",
    };

    if !state::fs::path_exists(&paths.daemon_socket()) {
        return DoctorCheck::fail("Daemon socket", "daemon socket is missing", repair)
            .with_detail(format!("path: {}", paths.daemon_socket()));
    }

    match daemon::health_blocking(paths.clone()) {
        Ok(()) => DoctorCheck::pass("Daemon socket", "daemon answered health check")
            .with_detail(format!("path: {}", paths.daemon_socket())),
        Err(error) => DoctorCheck::fail(
            "Daemon socket",
            "daemon socket is present but daemon did not answer health check",
            repair,
        )
        .with_detail(format!("path: {}; error: {error}", paths.daemon_socket())),
    }
}

fn dns_check(environment: &impl Environment, paths: &PvPaths) -> Result<DoctorCheck, ExecuteError> {
    let prepared = platform::inspect_resolver_file(&paths.resolver_config(), None);
    let expected = resolver_config_from_state(&prepared);
    let system_path = resolver_test_path(environment)?;
    let system = environment.inspect_resolver_file(&system_path, expected.as_ref());

    let check = match (&prepared, &system) {
        (ResolverFileState::Current { port, .. }, ResolverFileState::Current { .. }) => {
            DoctorCheck::pass("DNS config", format!("system resolver uses port {port}"))
        }
        (ResolverFileState::Current { .. }, ResolverFileState::Missing { path }) => {
            DoctorCheck::fail(
                "DNS config",
                "system resolver config is missing",
                "pv dns:install",
            )
            .with_detail(format!("path: {path}"))
        }
        (ResolverFileState::Current { .. }, ResolverFileState::Stale { path, .. }) => {
            DoctorCheck::fail(
                "DNS config",
                "system resolver config is PV-owned but stale",
                "pv dns:install",
            )
            .with_detail(format!("path: {path}"))
        }
        (ResolverFileState::Current { .. }, ResolverFileState::Conflict { path }) => {
            DoctorCheck::fail(
                "DNS config",
                "system resolver config is not PV-owned",
                "pv dns:install",
            )
            .with_detail(format!("path: {path}"))
        }
        (ResolverFileState::Current { .. }, ResolverFileState::Unreadable { path, message }) => {
            DoctorCheck::fail(
                "DNS config",
                "system resolver config could not be inspected",
                "pv dns:install",
            )
            .with_detail(format!("{path}: {message}"))
        }
        (ResolverFileState::Missing { path }, _) => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config is missing",
            "pv dns:install",
        )
        .with_detail(format!("path: {path}")),
        (ResolverFileState::Stale { path, .. }, _) => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config is PV-owned but stale",
            "pv dns:install",
        )
        .with_detail(format!("path: {path}")),
        (ResolverFileState::Conflict { path }, _) => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config is not PV-owned",
            "pv dns:install",
        )
        .with_detail(format!("path: {path}")),
        (ResolverFileState::Unreadable { path, message }, _) => DoctorCheck::fail(
            "DNS config",
            "prepared resolver config could not be inspected",
            "pv dns:install",
        )
        .with_detail(format!("{path}: {message}")),
    };

    Ok(check)
}

fn ports_check(
    environment: &impl Environment,
    paths: &PvPaths,
    database: Option<&Database>,
) -> Result<DoctorCheck, ExecuteError> {
    let routing = PfRoutingDiagnostic::read(environment, paths, database)?;
    let message = match routing.state {
        PfRoutingState::Active => "low-port routing is active",
        PfRoutingState::Inactive => "low-port redirects are inactive",
        PfRoutingState::Drifted => "low-port routing has drifted",
        PfRoutingState::Unknown => "low-port routing could not be verified",
    };
    let detail = format!(
        "evidence: {}; expected: HTTP {}, HTTPS {}; active: HTTP {}, HTTPS {}; observed: {}",
        routing.evidence.as_str(),
        display_port(routing.expected_http_port),
        display_port(routing.expected_https_port),
        display_port(routing.active_http_port),
        display_port(routing.active_https_port),
        routing.observed_at,
    );
    let check = if routing.is_active() {
        DoctorCheck::pass("Port redirect config", message)
    } else {
        DoctorCheck::fail("Port redirect config", message, "pv ports:install")
    };

    Ok(check.with_detail(detail).with_routing(routing))
}

fn display_port(port: Option<u16>) -> String {
    port.map_or_else(|| "-".to_owned(), |port| port.to_string())
}

fn ca_check(environment: &impl Environment, paths: &PvPaths) -> DoctorCheck {
    let local_ca =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_ca(&local_ca);
    let trust = ca_trust_state(environment, local_metadata.as_ref());

    match (&local_ca, &trust) {
        (CaFileState::Current { metadata, .. }, TrustDomainState::Current { .. }) => {
            DoctorCheck::pass(
                "Local CA trust",
                format!("system trust matches fingerprint {}", metadata.fingerprint),
            )
        }
        (CaFileState::Current { .. }, TrustDomainState::NotTrusted { fingerprint }) => {
            DoctorCheck::fail(
                "Local CA trust",
                "local CA is not trusted in the System keychain",
                "pv ca:trust",
            )
            .with_detail(format!("fingerprint: {fingerprint}"))
        }
        (
            CaFileState::Current { .. },
            TrustDomainState::Stale {
                actual_fingerprint, ..
            },
        ) => DoctorCheck::fail(
            "Local CA trust",
            "System keychain has stale PV CA trust",
            "pv ca:trust",
        )
        .with_detail(format!("actual fingerprint: {actual_fingerprint}")),
        (CaFileState::Current { .. }, TrustDomainState::Denied { fingerprint }) => {
            DoctorCheck::fail(
                "Local CA trust",
                "System keychain denies PV CA trust",
                "pv ca:trust",
            )
            .with_detail(format!("fingerprint: {fingerprint}"))
        }
        (CaFileState::Current { .. }, TrustDomainState::Unknown { reason }) => DoctorCheck::fail(
            "Local CA trust",
            "System keychain trust could not be determined",
            "pv ca:trust",
        )
        .with_detail(reason.clone()),
        (CaFileState::Current { .. }, TrustDomainState::Unreadable { message }) => {
            DoctorCheck::fail(
                "Local CA trust",
                "System keychain trust could not be inspected",
                "pv ca:trust",
            )
            .with_detail(message.clone())
        }
        (
            CaFileState::Missing {
                certificate_path,
                private_key_path,
            },
            _,
        ) => DoctorCheck::fail(
            "Local CA files",
            "local CA files are missing",
            "pv ca:trust",
        )
        .with_detail(format!(
            "certificate: {certificate_path}; private key: {private_key_path}"
        )),
        (CaFileState::RepairRequired { reason, .. }, _) => DoctorCheck::fail(
            "Local CA files",
            "local CA files require repair",
            "pv ca:trust",
        )
        .with_detail(format!("reason: {reason:?}")),
        (CaFileState::Unreadable { path, message }, _) => DoctorCheck::fail(
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
    let failed = database.unresolved_job_failures()?;

    if failed.is_empty() {
        return Ok(DoctorCheck::pass(
            "Recent jobs",
            "no unresolved failed jobs",
        ));
    }

    let repair = repair_for_job_subject(&failed[0].subject);
    Ok(DoctorCheck::fail(
        "Recent jobs",
        format!("{} unresolved failed job(s)", failed.len()),
        repair,
    )
    .with_detail(
        failed
            .into_iter()
            .map(|failure| {
                let job = failure.job;
                format!(
                    "{} {} {} at {}: {}",
                    job.id,
                    job.kind,
                    job.scope,
                    job.finished_at.as_deref().unwrap_or(&job.started_at),
                    job.error.unwrap_or_else(|| "failed".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    ))
}

fn repair_for_job_subject(subject: &JobDiagnosticSubject) -> String {
    match subject {
        JobDiagnosticSubject::UpdateAssessment => "pv update".to_owned(),
        JobDiagnosticSubject::Resource { name, track: _ } if name == "composer" => {
            "pv composer:install".to_owned()
        }
        JobDiagnosticSubject::Resource { name, track }
            if matches!(
                name.as_str(),
                "mailpit" | "mysql" | "postgres" | "redis" | "rustfs"
            ) =>
        {
            format!("pv {name}:install {track}")
        }
        JobDiagnosticSubject::SystemReconciliation
        | JobDiagnosticSubject::GatewayRuntime
        | JobDiagnosticSubject::Project { .. }
        | JobDiagnosticSubject::Resource { .. }
        | JobDiagnosticSubject::Other { .. } => "pv daemon:restart".to_owned(),
    }
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
        state::RuntimeSubject::PhpRuntimeWorker { php_runtime_key } => {
            format!("worker:{php_runtime_key}")
        }
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

fn launch_agent_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.launch_agent_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
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
