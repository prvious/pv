use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use platform::{
    CaFileState, LaunchAgentFileState, LocalCaMetadata, PfConfReference, PfFileState,
    PfRedirectConfig, ResolverConfig, ResolverFileState, TrustDomainState,
};
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
        let launch_agent_path = launch_agent_path(environment)?;
        let launch_agent = platform::inspect_launch_agent_file(&launch_agent_path, None);
        let checks = vec![
            layout_check(&paths),
            database_check(&paths, database.as_ref()),
            launch_agent_check(&launch_agent),
            daemon_socket_check(&paths, &launch_agent),
            dns_check(environment, &paths)?,
            ports_check(environment, &paths)?,
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
    let system = platform::inspect_resolver_file(&system_path, expected.as_ref());

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
) -> Result<DoctorCheck, ExecuteError> {
    let prepared_anchor = platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), None);
    let prepared_reference =
        platform::inspect_pf_conf_reference(&paths.pf_conf_reference_config(), None);
    if let Some(check) = pf_file_failure(
        "Port redirect config",
        "prepared pf config",
        &prepared_anchor,
        &prepared_reference,
    ) {
        return Ok(check);
    }

    let expected_anchor = pf_config_from_anchor_state(&prepared_anchor);
    let expected_reference = pf_reference_from_state(&prepared_reference);
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;
    let system_anchor =
        platform::inspect_pf_anchor_file(&system_anchor_path, expected_anchor.as_ref());
    let system_reference =
        platform::inspect_pf_conf_reference(&system_pf_conf_path, expected_reference.as_ref());
    if let Some(check) = pf_file_failure(
        "Port redirect config",
        "system pf config",
        &system_anchor,
        &system_reference,
    ) {
        return Ok(check);
    }

    let active = match environment.active_pf_redirect_config() {
        Ok(active) => active,
        Err(error) => {
            return Ok(DoctorCheck::fail(
                "Port redirect config",
                "active pf redirects could not be inspected",
                "pv ports:install",
            )
            .with_detail(error.to_string()));
        }
    };
    if active.as_ref() == expected_anchor.as_ref() {
        return Ok(DoctorCheck::pass(
            "Port redirect config",
            "system pf config and active redirects are current",
        ));
    }

    Ok(DoctorCheck::fail(
        "Port redirect config",
        "active pf redirects are not loaded",
        "pv ports:install",
    ))
}

fn pf_file_failure(
    name: &'static str,
    label: &'static str,
    anchor: &PfFileState<PfRedirectConfig>,
    reference: &PfFileState<PfConfReference>,
) -> Option<DoctorCheck> {
    match (anchor, reference) {
        (PfFileState::Current { .. }, PfFileState::Current { .. }) => None,
        (PfFileState::Missing { path }, _) | (_, PfFileState::Missing { path }) => Some(
            DoctorCheck::fail(name, format!("{label} is missing"), "pv ports:install")
                .with_detail(format!("path: {path}")),
        ),
        (PfFileState::Conflict { path }, _) | (_, PfFileState::Conflict { path }) => Some(
            DoctorCheck::fail(name, format!("{label} is not PV-owned"), "pv ports:install")
                .with_detail(format!("path: {path}")),
        ),
        (PfFileState::Unreadable { path, message }, _)
        | (_, PfFileState::Unreadable { path, message }) => Some(
            DoctorCheck::fail(
                name,
                format!("{label} could not be inspected"),
                "pv ports:install",
            )
            .with_detail(format!("{path}: {message}")),
        ),
        (PfFileState::Stale { path, .. }, _) | (_, PfFileState::Stale { path, .. }) => Some(
            DoctorCheck::fail(
                name,
                format!("{label} is PV-owned but stale"),
                "pv ports:install",
            )
            .with_detail(format!("path: {path}")),
        ),
    }
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

fn pf_config_from_anchor_state(state: &PfFileState<PfRedirectConfig>) -> Option<PfRedirectConfig> {
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

fn pf_anchor_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_anchor_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_conf_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_conf_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}
