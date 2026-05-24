mod backup;
mod database;
mod error;
pub mod fs;
mod migrations;
mod paths;

pub use database::{
    Database, DatabaseInspection, JobRecord, JobStatus, PortAssignment, PortRequest,
};
pub use error::StateError;
pub use paths::{PathSummaryEntry, PvPaths};

#[doc(hidden)]
pub mod testing {
    use rusqlite::Transaction;

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
        crate::fs::read_to_string(path)
    }

    pub fn transaction<T>(
        database: &mut Database,
        operation: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, StateError> {
        database.transaction(operation)
    }
}
