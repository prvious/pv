use std::io;

use config::ConfigError;
use hickory_proto::ProtoError;
use hickory_proto::serialize::binary::DecodeError;
use protocol::ProtocolError;
use resources::{ManagedResourceCommandError, ManagedResourceUpdate, ResourcesError};
use serde_json::Error as JsonError;
use state::StateError;
use thiserror::Error;
use tokio::task::JoinError;
use tokio_util::codec::LinesCodecError;

use crate::caddy_admin::CaddyAdminError;
use crate::reconciliation::ReconciliationScopeParseError;

#[derive(Debug)]
pub struct ManagedResourceProjectFailure {
    resource_name: String,
    track: String,
    error: Box<DaemonError>,
}

impl ManagedResourceProjectFailure {
    pub(crate) fn new(resource_name: String, track: String, error: DaemonError) -> Self {
        Self {
            resource_name,
            track,
            error: Box::new(error),
        }
    }

    pub fn resource_name(&self) -> &str {
        &self.resource_name
    }

    pub fn track(&self) -> &str {
        &self.track
    }

    pub fn error(&self) -> &DaemonError {
        &self.error
    }

    pub(crate) fn into_error(self) -> DaemonError {
        *self.error
    }
}

#[derive(Debug)]
pub struct RuntimeReconciliationFailure {
    runtime_key: String,
    error: Box<DaemonError>,
}

impl RuntimeReconciliationFailure {
    pub(crate) fn new(runtime_key: String, error: DaemonError) -> Self {
        Self {
            runtime_key,
            error: Box::new(error),
        }
    }

    pub fn runtime_key(&self) -> &str {
        &self.runtime_key
    }

    pub fn error(&self) -> &DaemonError {
        &self.error
    }
}

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error(
        "{source}; additionally failed to remove daemon socket during startup cleanup: {cleanup}"
    )]
    StartupCleanupFailed {
        #[source]
        source: Box<DaemonError>,
        cleanup: Box<DaemonError>,
    },

    #[error("{source}; additionally failed to clean up runtime `{runtime}` transaction: {cleanup}")]
    RuntimeCleanupFailed {
        runtime: String,
        #[source]
        source: Box<DaemonError>,
        cleanup: Box<DaemonError>,
    },

    #[error("Caddy update failed with `{source}`; compensation also failed: {compensation}")]
    CaddyUpdateCompensationFailed {
        #[source]
        source: Box<DaemonError>,
        compensation: Box<DaemonError>,
    },

    #[error(
        "Managed Resource update failed with `{source}`; reconciliation also failed: {reconciliation}"
    )]
    PartialUpdateReconciliationFailed {
        #[source]
        source: Box<DaemonError>,
        reconciliation: Box<DaemonError>,
    },

    #[error(
        "Managed Resource update partially completed: {}; remaining update failed: {source}",
        managed_resource_partial_update_summary(.update)
    )]
    ManagedResourcePartialUpdateFailed {
        update: ManagedResourceUpdate,
        #[source]
        source: Box<DaemonError>,
    },

    #[error("daemon socket is already in use at {path}")]
    SocketInUse { path: String },

    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] JsonError),

    #[error("daemon protocol frame error: {0}")]
    Frame(#[from] LinesCodecError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error("daemon protocol error: daemon {phase} timed out")]
    ProtocolTimedOut { phase: &'static str },

    #[error("daemon protocol error: daemon protocol mismatch; run `pv daemon:restart`")]
    ProtocolMismatch { expected: u16, actual: u16 },

    #[error("daemon protocol error: {reason}")]
    UnexpectedProtocolResponse { reason: String },

    #[error("daemon protocol error: {message}")]
    DaemonRejected { message: String },

    #[error("Caddy admin error: {0}")]
    CaddyAdmin(#[from] CaddyAdminError),

    #[error("DNS request decode error: {0}")]
    DnsDecode(#[from] DecodeError),

    #[error("DNS response encode error: {0}")]
    DnsEncode(#[from] ProtoError),

    #[error("DNS resolver failed to bind {protocol} on 127.0.0.1:{port}: {source}")]
    DnsBind {
        protocol: &'static str,
        port: u16,
        #[source]
        source: io::Error,
    },

    #[error("state error: {0}")]
    State(#[from] StateError),

    #[error("reconciliation scope error: {0}")]
    ReconciliationScope(#[from] ReconciliationScopeParseError),

    #[error("Project config error: {0}")]
    Config(#[from] ConfigError),

    #[error("Project `{project_id}` env dependencies do not match their last applied state")]
    ProjectEnvDependenciesNotApplied { project_id: String },

    #[error(
        "Project `{project_id}` Managed Resource allocation failed with `{allocation}`; additionally failed to record the Project failure: {recording}"
    )]
    ProjectAllocationFailureRecordingFailed {
        project_id: String,
        allocation: Box<DaemonError>,
        #[source]
        recording: Box<DaemonError>,
    },

    #[error(
        "Project `{project_id}` env reconciliation failed with `{reconciliation}`; additionally failed to record the Project failure: {recording}"
    )]
    ProjectEnvFailureRecordingFailed {
        project_id: String,
        reconciliation: Box<DaemonError>,
        #[source]
        recording: Box<DaemonError>,
    },

    #[error(
        "Managed Resource `{resource_name}` track `{track}` reconciliation failed with `{reconciliation}`; additionally failed to record the runtime failure: {recording}"
    )]
    ManagedResourceRuntimeFailureRecordingFailed {
        resource_name: String,
        track: String,
        reconciliation: Box<DaemonError>,
        #[source]
        recording: Box<DaemonError>,
    },

    #[error("Managed Resource error: {0}")]
    Resources(#[from] ResourcesError),

    #[error("Managed Resource command failed: {0}")]
    ManagedResourceCommand(#[from] ManagedResourceCommandError),

    #[error("platform error: {0}")]
    Platform(#[from] platform::PlatformError),

    #[error("Managed Resource default installs failed: {}", default_install_failures(.failures))]
    ManagedResourceDefaultInstallFailures { failures: Vec<String> },

    #[error(
        "Project Managed Resource reconciliation failed: {}",
        project_resource_failures(.failures)
    )]
    ManagedResourceProjectFailures {
        failures: Vec<ManagedResourceProjectFailure>,
    },

    #[error("runtime reconciliation failed: {}", runtime_reconciliation_failures(.failures))]
    RuntimeReconciliationFailures {
        failures: Vec<RuntimeReconciliationFailure>,
    },

    #[error("Redis readiness failed: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("daemon task failed: {0}")]
    Task(#[from] JoinError),

    #[error("process `{name}` started without an observable pid")]
    MissingProcessId { name: String },

    #[error("process `{name}` started without observable identity for pid {pid}")]
    MissingProcessIdentity { name: String, pid: u32 },

    #[error("readiness check `{check}` timed out after {timeout_ms}ms; last error: {last_error:?}")]
    ReadinessTimedOut {
        check: String,
        timeout_ms: u128,
        last_error: Option<String>,
    },

    #[error("Managed Resource runtime `{resource}` is not supported yet")]
    UnsupportedManagedResourceRuntime { resource: String },

    #[error(
        "Managed Resource runtime `{name}` is listening but no PV-owned process could be verified"
    )]
    NonPvManagedResourceRuntimeListener { name: String },

    #[error(
        "Managed Resource runtime `{resource}` track `{track}` is missing installed artifact path"
    )]
    ManagedResourceArtifactMissing { resource: String, track: String },

    #[error("Managed Resource runtime `{resource}` track `{track}` is marked removed")]
    ManagedResourceTrackRemoved { resource: String, track: String },

    #[error("Managed Resource runtime `{resource}` track `{track}` is missing port `{port}`")]
    ManagedResourcePortMissing {
        resource: String,
        track: String,
        port: String,
    },

    #[error(
        "Managed Resource runtime `{resource}` track `{track}` uses reserved port name `{port}`"
    )]
    ManagedResourcePortNameReserved {
        resource: String,
        track: String,
        port: String,
    },

    #[error(
        "PostgreSQL track `{track}` cannot preload `{library}` because its module is missing at {path}; install an artifact that supplies the library or remove it from the track preload desired state"
    )]
    PostgresPreloadLibraryMissing {
        track: String,
        library: String,
        path: camino::Utf8PathBuf,
    },

    #[error(
        "PostgreSQL track `{track}` cannot preload `timescaledb` with `pg_duckdb` until their compatibility is validated; remove one library from the track preload desired state"
    )]
    UnsafePostgresPreloadCombination { track: String },

    #[error(
        "invalid SQL database identifier `{identifier}`: identifiers must use ASCII alphanumeric characters and underscores"
    )]
    InvalidSqlIdentifier { identifier: String },

    #[error("SQL admin error: {0}")]
    SqlAdmin(#[from] sqlx::Error),

    #[error("time formatting failed: {0}")]
    TimeFormat(#[from] time::error::Format),
}

fn default_install_failures(failures: &[String]) -> String {
    failures.join("; ")
}

fn project_resource_failures(failures: &[ManagedResourceProjectFailure]) -> String {
    failures
        .iter()
        .map(|failure| {
            format!(
                "{} {}: {}",
                failure.resource_name, failure.track, failure.error
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn runtime_reconciliation_failures(failures: &[RuntimeReconciliationFailure]) -> String {
    failures
        .iter()
        .map(|failure| format!("{}: {}", failure.runtime_key, failure.error))
        .collect::<Vec<_>>()
        .join("; ")
}

fn managed_resource_partial_update_summary(update: &ManagedResourceUpdate) -> String {
    let installs = update
        .installs()
        .iter()
        .map(|install| {
            format!(
                "{} track {} to {}",
                install.resource_name(),
                install.track(),
                install.artifact_version()
            )
        })
        .collect::<Vec<_>>();

    format!(
        "updated {} artifact(s) ({})",
        installs.len(),
        installs.join(", ")
    )
}
