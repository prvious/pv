use std::collections::BTreeSet;

use rusqlite::{Connection, Transaction, params};

use crate::{PvPaths, StateError, backup};

const CORE_SCHEMA_SQL: &str = include_str!("sql/001_core_state_schema.sql");

pub(crate) const DEFAULT_MIGRATIONS: &[Migration] =
    &[Migration::new(1, "core_state_schema", CORE_SCHEMA_SQL)];

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

impl Migration {
    pub const fn new(version: i64, name: &'static str, sql: &'static str) -> Self {
        Self { version, name, sql }
    }
}

pub(crate) fn run(
    paths: &PvPaths,
    connection: &mut Connection,
    migrations: &[Migration],
    database_existed: bool,
) -> Result<(), StateError> {
    let applied_versions = applied_versions(connection)?;
    let pending = migrations
        .iter()
        .filter(|migration| !applied_versions.contains(&migration.version))
        .copied()
        .collect::<Vec<_>>();

    if pending.is_empty() {
        ensure_migration_table(connection)?;
        return Ok(());
    }

    if database_existed {
        backup::database(paths, |backup_path| {
            connection.execute("VACUUM main INTO ?1", params![backup_path.as_str()])?;
            Ok(())
        })?;
    }

    let transaction = connection.transaction()?;
    transaction.execute_batch(MIGRATION_TABLE_SQL)?;

    for migration in pending {
        apply_migration(&transaction, migration)?;
    }

    transaction.commit()?;

    Ok(())
}

fn ensure_migration_table(connection: &Connection) -> Result<(), StateError> {
    connection.execute_batch(MIGRATION_TABLE_SQL)?;

    Ok(())
}

const MIGRATION_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS pv_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
);
"#;

fn apply_migration(transaction: &Transaction<'_>, migration: Migration) -> Result<(), StateError> {
    transaction
        .execute_batch(migration.sql)
        .map_err(|source| StateError::MigrationFailed {
            version: migration.version,
            name: migration.name,
            source,
        })?;
    transaction
        .execute(
            "INSERT INTO pv_migrations (version, name, applied_at) VALUES (?1, ?2, datetime('now'))",
            params![migration.version, migration.name],
        )
        .map_err(|source| StateError::MigrationFailed {
            version: migration.version,
            name: migration.name,
            source,
        })?;

    Ok(())
}

fn applied_versions(connection: &Connection) -> Result<BTreeSet<i64>, StateError> {
    if !table_exists(connection, "pv_migrations")? {
        return Ok(BTreeSet::new());
    }

    let mut statement = connection.prepare("SELECT version FROM pv_migrations")?;
    let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
    let mut versions = BTreeSet::new();

    for row in rows {
        versions.insert(row?);
    }

    Ok(versions)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, StateError> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, i64>(0),
    )?;

    Ok(count > 0)
}
