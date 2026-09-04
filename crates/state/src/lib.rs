mod app_release;
mod backup;
mod database;
mod error;
pub mod fs;
mod migrations;
mod paths;
mod update_lock;

pub use app_release::{AppReleaseInstall, AppReleaseLayout};
pub use database::{
    DNS_PREFERRED_PORT, Database, DatabaseInspection, EnvContextValues,
    GATEWAY_HTTP_PREFERRED_PORT, GATEWAY_HTTPS_PREFERRED_PORT, GatewayPort, GatewayPortAssignments,
    JobDiagnosticSubject, JobRecord, JobStatus, LinkProjectInput, LinkProjectResult,
    LinkProjectStatus, ManagedResourceDesiredState, ManagedResourceTrackInstallInput,
    ManagedResourceTrackRecord, ManagedResourceTrackRemovalInput, PortAssignment, PortOwner,
    PortRequest, PostgresPreloadLibrary, ProjectConfigWatch, ProjectEnvAllocationContext,
    ProjectEnvObservedStateRecord, ProjectEnvObservedStatus, ProjectEnvObservedWarningInput,
    ProjectEnvObservedWarningRecord, ProjectEnvResourceContext, ProjectEnvStateContext,
    ProjectManagedResourceInput, ProjectManagedResourceRecord, ProjectMode, ProjectPhpRuntimeInput,
    ProjectPhpRuntimeRecord, ProjectReconciliationStateInput, ProjectRecord,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, ResourceAllocationInput,
    ResourceAllocationRecord, ResourceAllocationStatus, RuntimeObservedStateRecord,
    RuntimeObservedStatus, RuntimeSubject, UnresolvedJobFailure, php_runtime_key,
};
pub use error::{StateCapability, StateError};
pub use paths::{PathSummaryEntry, PvPaths};
pub use update_lock::{HelperLifecycleLock, JobsLock, UpdateLock};

#[doc(hidden)]
pub mod testing {
    #[cfg(any(test, feature = "test-support"))]
    use std::sync::mpsc::{Receiver, channel};

    use rusqlite::Transaction;

    use crate::fs::read_to_string as read_file_to_string;
    pub use crate::migrations::Migration;
    use crate::{Database, PvPaths, StateError};

    pub fn open_with_migrations(
        paths: &PvPaths,
        migrations: &[Migration],
    ) -> Result<Database, StateError> {
        Database::open_with_migrations(paths, migrations)
    }

    pub fn query_i64(database: &Database, sql: &str) -> Result<i64, StateError> {
        database.query_i64(sql)
    }

    pub fn read_to_string(path: &camino::Utf8Path) -> Result<String, StateError> {
        read_file_to_string(path)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_sensitive_write(path: camino::Utf8PathBuf) {
        crate::fs::fail_next_sensitive_write(path);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn remove_database_auxiliary_file_before_hardening(
        path: camino::Utf8PathBuf,
    ) -> Receiver<()> {
        let (sender, receiver) = channel();
        crate::fs::remove_database_auxiliary_file_before_hardening(path, sender);

        receiver
    }

    pub fn transaction<T>(
        database: &mut Database,
        operation: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, StateError> {
        database.transaction(operation)
    }
}
