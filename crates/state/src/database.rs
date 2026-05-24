use std::time::Duration;

use rusqlite::{Connection, Transaction};

use crate::{PvPaths, StateError, fs, migrations};

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);

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
        Self::open_with_migrations(paths, migrations::DEFAULT_MIGRATIONS)
    }

    pub(crate) fn open_with_migrations(
        paths: &PvPaths,
        migrations: &[migrations::Migration],
    ) -> Result<Self, StateError> {
        fs::ensure_layout(paths)?;

        let database_existed = fs::database_exists(paths);
        let mut connection = Connection::open(paths.db())?;
        fs::secure_database_files(paths)?;
        configure_connection(&connection)?;
        migrations::run(paths, &mut connection, migrations, database_existed)?;
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

    pub(crate) fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, StateError> {
        let transaction = self.connection.transaction()?;
        let output = operation(&transaction)?;
        transaction.commit()?;

        Ok(output)
    }

    pub(crate) fn query_i64(&self, sql: &str) -> Result<i64, StateError> {
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
