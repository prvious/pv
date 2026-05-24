use std::time::Duration;

use rusqlite::{Connection, Transaction, params};

use crate::{PvPaths, StateError, fs, migrations};

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
const RECENT_JOB_LIMIT: i64 = 100;
const PORT_CANDIDATE_LIMIT: usize = 10;

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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Running,
    Succeeded,
    Failed,
}

impl JobStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn from_database(status: String) -> Result<Self, StateError> {
        match status.as_str() {
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            _ => Err(StateError::UnknownJobStatus { status }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRecord {
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub status: JobStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortRequest {
    pub name: String,
    pub owner_kind: String,
    pub resource_name: Option<String>,
    pub track: Option<String>,
    pub preferred_port: u16,
    pub fallback_start: u16,
    pub fallback_end: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortAssignment {
    pub name: String,
    pub port: u16,
    pub owner_kind: String,
    pub resource_name: Option<String>,
    pub track: Option<String>,
    pub updated_at: String,
}

struct JobRecordRow {
    id: String,
    kind: String,
    scope: String,
    status: String,
    started_at: String,
    finished_at: Option<String>,
    summary: Option<String>,
    error: Option<String>,
}

struct PortAssignmentRow {
    name: String,
    port: u16,
    owner_kind: String,
    resource_name: Option<String>,
    track: Option<String>,
    updated_at: String,
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

    pub fn start_job(&mut self, kind: &str, scope: &str) -> Result<JobRecord, StateError> {
        let started_at = timestamp()?;

        self.transaction(|transaction| {
            let id = next_job_id(transaction)?;
            transaction.execute(
                "INSERT INTO jobs (id, kind, scope, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, kind, scope, JobStatus::Running.as_str(), started_at],
            )?;

            Ok(JobRecord {
                id,
                kind: kind.to_string(),
                scope: scope.to_string(),
                status: JobStatus::Running,
                started_at,
                finished_at: None,
                summary: None,
                error: None,
            })
        })
    }

    pub fn complete_job(&mut self, id: &str, summary: &str) -> Result<(), StateError> {
        let finished_at = timestamp()?;

        let updated = self.transaction(|transaction| {
            let updated = transaction.execute(
                "UPDATE jobs SET status = ?1, finished_at = ?2, summary = ?3, error = NULL WHERE id = ?4",
                params![JobStatus::Succeeded.as_str(), finished_at, summary, id],
            )?;
            if updated > 0 {
                prune_old_jobs(transaction)?;
            }

            Ok(updated)
        })?;
        if updated == 0 {
            return Err(StateError::JobNotFound { id: id.to_string() });
        }

        Ok(())
    }

    pub fn fail_job(&mut self, id: &str, error: &str) -> Result<(), StateError> {
        let finished_at = timestamp()?;

        let updated = self.transaction(|transaction| {
            let updated = transaction.execute(
                "UPDATE jobs SET status = ?1, finished_at = ?2, error = ?3 WHERE id = ?4",
                params![JobStatus::Failed.as_str(), finished_at, error, id],
            )?;
            if updated > 0 {
                prune_old_jobs(transaction)?;
            }

            Ok(updated)
        })?;
        if updated == 0 {
            return Err(StateError::JobNotFound { id: id.to_string() });
        }

        Ok(())
    }

    pub fn recent_jobs(&self) -> Result<Vec<JobRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, scope, status, started_at, finished_at, summary, error FROM jobs ORDER BY started_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![RECENT_JOB_LIMIT], |row| {
            Ok(JobRecordRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                scope: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                finished_at: row.get(5)?,
                summary: row.get(6)?,
                error: row.get(7)?,
            })
        })?;
        let mut jobs = Vec::new();

        for row in rows {
            let row = row?;
            jobs.push(JobRecord {
                id: row.id,
                kind: row.kind,
                scope: row.scope,
                status: JobStatus::from_database(row.status)?,
                started_at: row.started_at,
                finished_at: row.finished_at,
                summary: row.summary,
                error: row.error,
            });
        }

        Ok(jobs)
    }

    pub fn assign_port(
        &mut self,
        request: PortRequest,
        mut is_available: impl FnMut(u16) -> bool,
    ) -> Result<PortAssignment, StateError> {
        if let Some(existing) = self.port_assignment(&request.name)?
            && is_available(existing.port)
        {
            return Ok(existing);
        }

        let assigned_ports = self.assigned_port_numbers_except(&request.name)?;

        for candidate in request.candidates() {
            if assigned_ports.contains(&candidate) || !is_available(candidate) {
                continue;
            }

            return self.upsert_port(request, candidate);
        }

        Err(StateError::NoAvailablePort {
            name: request.name,
            preferred_port: request.preferred_port,
            fallback_start: request.fallback_start,
            fallback_end: request.fallback_end,
            attempts: PORT_CANDIDATE_LIMIT,
        })
    }

    pub fn release_port(&mut self, name: &str) -> Result<bool, StateError> {
        let deleted = self
            .connection
            .execute("DELETE FROM ports WHERE name = ?1", params![name])?;

        Ok(deleted > 0)
    }

    pub fn assigned_ports(&self) -> Result<Vec<PortAssignment>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT name, port, owner_kind, resource_name, track, updated_at FROM ports ORDER BY name",
        )?;
        let rows = statement.query_map([], port_assignment_from_row)?;
        let mut assignments = Vec::new();

        for row in rows {
            assignments.push(row?.into_assignment());
        }

        Ok(assignments)
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

    fn port_assignment(&self, name: &str) -> Result<Option<PortAssignment>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT name, port, owner_kind, resource_name, track, updated_at FROM ports WHERE name = ?1",
        )?;
        let mut rows = statement.query_map(params![name], port_assignment_from_row)?;

        match rows.next() {
            Some(row) => Ok(Some(row?.into_assignment())),
            None => Ok(None),
        }
    }

    fn assigned_port_numbers_except(&self, name: &str) -> Result<Vec<u16>, StateError> {
        let mut statement = self
            .connection
            .prepare("SELECT port FROM ports WHERE name != ?1 ORDER BY port")?;
        let rows = statement.query_map(params![name], |row| row.get::<_, u16>(0))?;
        let mut ports = Vec::new();

        for row in rows {
            ports.push(row?);
        }

        Ok(ports)
    }

    fn upsert_port(
        &mut self,
        request: PortRequest,
        port: u16,
    ) -> Result<PortAssignment, StateError> {
        let updated_at = timestamp()?;

        self.transaction(|transaction| {
            transaction.execute(
                "INSERT INTO ports (name, port, owner_kind, resource_name, track, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(name) DO UPDATE SET
                    port = excluded.port,
                    owner_kind = excluded.owner_kind,
                    resource_name = excluded.resource_name,
                    track = excluded.track,
                    updated_at = excluded.updated_at",
                params![
                    request.name,
                    port,
                    request.owner_kind,
                    request.resource_name,
                    request.track,
                    updated_at,
                ],
            )?;

            Ok(())
        })?;

        Ok(PortAssignment {
            name: request.name,
            port,
            owner_kind: request.owner_kind,
            resource_name: request.resource_name,
            track: request.track,
            updated_at,
        })
    }
}

impl PortRequest {
    fn candidates(&self) -> Vec<u16> {
        let mut candidates = vec![self.preferred_port];

        for port in self.fallback_start..=self.fallback_end {
            if candidates.len() >= PORT_CANDIDATE_LIMIT {
                break;
            }

            if port != self.preferred_port {
                candidates.push(port);
            }
        }

        candidates
    }
}

impl PortAssignmentRow {
    fn into_assignment(self) -> PortAssignment {
        PortAssignment {
            name: self.name,
            port: self.port,
            owner_kind: self.owner_kind,
            resource_name: self.resource_name,
            track: self.track,
            updated_at: self.updated_at,
        }
    }
}

fn port_assignment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PortAssignmentRow> {
    Ok(PortAssignmentRow {
        name: row.get(0)?,
        port: row.get(1)?,
        owner_kind: row.get(2)?,
        resource_name: row.get(3)?,
        track: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn configure_connection(connection: &Connection) -> Result<(), StateError> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;

    Ok(())
}

fn next_job_id(transaction: &Transaction<'_>) -> rusqlite::Result<String> {
    let next_number = transaction.query_row(
        "SELECT COALESCE(MAX(CAST(SUBSTR(id, 5) AS INTEGER)), 0) + 1 FROM jobs WHERE id GLOB 'job_[0-9]*'",
        [],
        |row| row.get::<_, i64>(0),
    )?;

    Ok(format!("job_{next_number:06}"))
}

fn prune_old_jobs(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute(
        "DELETE FROM jobs WHERE status != ?1 AND id NOT IN (
            SELECT id FROM jobs WHERE status != ?1 ORDER BY started_at DESC, id DESC LIMIT ?2
        )",
        params![JobStatus::Running.as_str(), RECENT_JOB_LIMIT],
    )?;

    Ok(())
}

fn timestamp() -> Result<String, StateError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}
