use std::collections::BTreeSet;
use std::time::Duration;

use rusqlite::{Connection, Transaction, params};

use crate::{PvPaths, StateError, fs};

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);

const CORE_SCHEMA_SQL: &str = r#"
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    primary_hostname TEXT NOT NULL UNIQUE,
    config_path TEXT,
    desired_php_track TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE project_hostnames (
    hostname TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    is_primary INTEGER NOT NULL CHECK (is_primary IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE TABLE managed_resource_tracks (
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    desired_state TEXT NOT NULL,
    installed_version TEXT,
    current_artifact_path TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (resource_name, track)
);

CREATE TABLE resource_allocations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    resource_name TEXT NOT NULL,
    track TEXT NOT NULL,
    allocation_name TEXT NOT NULL,
    generated_name TEXT NOT NULL,
    env_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, resource_name, allocation_name)
);

CREATE TABLE ports (
    name TEXT PRIMARY KEY,
    port INTEGER NOT NULL UNIQUE,
    owner_kind TEXT NOT NULL,
    resource_name TEXT,
    track TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE observed_states (
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    status TEXT NOT NULL,
    message TEXT,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (subject_kind, subject_id)
);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    summary TEXT,
    error TEXT
);
"#;

const DEFAULT_MIGRATIONS: &[Migration] = &[Migration::new(1, "core_state_schema", CORE_SCHEMA_SQL)];

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

#[derive(Debug)]
pub struct Database {
    connection: Connection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseInspection {
    pub busy_timeout_ms: i64,
    pub foreign_keys_enabled: bool,
    pub journal_mode: String,
    pub migrations: Vec<String>,
    pub tables: Vec<String>,
}

impl Database {
    pub fn open(paths: &PvPaths) -> Result<Self, StateError> {
        Self::open_with_migrations(paths, DEFAULT_MIGRATIONS)
    }

    pub fn open_with_migrations(
        paths: &PvPaths,
        migrations: &[Migration],
    ) -> Result<Self, StateError> {
        fs::ensure_layout(paths)?;

        let database_existed = fs::database_exists(paths);
        let mut connection = Connection::open(paths.db())?;
        fs::secure_database_files(paths)?;
        configure_connection(&connection)?;
        run_migrations(paths, &mut connection, migrations, database_existed)?;
        fs::secure_database_files(paths)?;

        Ok(Self { connection })
    }

    pub fn inspect(&self) -> Result<DatabaseInspection, StateError> {
        Ok(DatabaseInspection {
            busy_timeout_ms: self.pragma_i64("busy_timeout")?,
            foreign_keys_enabled: self.pragma_bool("foreign_keys")?,
            journal_mode: self.pragma_string("journal_mode")?,
            migrations: self.applied_migration_names()?,
            tables: self.table_names()?,
        })
    }

    pub fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, StateError> {
        let transaction = self.connection.transaction()?;
        let output = operation(&transaction)?;
        transaction.commit()?;

        Ok(output)
    }

    pub fn query_i64(&self, sql: &str) -> Result<i64, StateError> {
        Ok(self.connection.query_row(sql, [], |row| row.get(0))?)
    }

    fn pragma_bool(&self, pragma: &str) -> Result<bool, StateError> {
        Ok(self.pragma_i64(pragma)? == 1)
    }

    fn pragma_i64(&self, pragma: &str) -> Result<i64, StateError> {
        Ok(self
            .connection
            .pragma_query_value(None, pragma, |row| row.get(0))?)
    }

    fn pragma_string(&self, pragma: &str) -> Result<String, StateError> {
        Ok(self
            .connection
            .pragma_query_value(None, pragma, |row| row.get(0))?)
    }

    fn applied_migration_names(&self) -> Result<Vec<String>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT printf('%03d:%s', version, name) FROM pv_migrations ORDER BY version",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut migrations = Vec::new();

        for row in rows {
            migrations.push(row?);
        }

        Ok(migrations)
    }

    fn table_names(&self) -> Result<Vec<String>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut tables = Vec::new();

        for row in rows {
            tables.push(row?);
        }

        Ok(tables)
    }
}

fn configure_connection(connection: &Connection) -> Result<(), StateError> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;

    Ok(())
}

fn run_migrations(
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
        backup_database(paths, connection)?;
    }

    ensure_migration_table(connection)?;

    for migration in pending {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO pv_migrations (version, name, applied_at) VALUES (?1, ?2, datetime('now'))",
            params![migration.version, migration.name],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

fn backup_database(paths: &PvPaths, connection: &Connection) -> Result<(), StateError> {
    fs::backup_database(paths, |backup_path| {
        connection.execute("VACUUM main INTO ?1", params![backup_path.as_str()])?;
        Ok(())
    })
}

fn ensure_migration_table(connection: &Connection) -> Result<(), StateError> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS pv_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );
        "#,
    )?;

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
