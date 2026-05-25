use std::collections::BTreeSet;
use std::time::Duration;

use camino::Utf8PathBuf;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

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
    owner: PortOwner,
    preferred_port: u16,
    fallback_start: u16,
    fallback_end: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortOwner {
    Dns,
    Gateway,
    ProjectWorker {
        project_id: String,
        php_track: String,
    },
    Resource {
        name: String,
        track: String,
    },
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigWatch {
    pub project_id: String,
    pub config_path: Utf8PathBuf,
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
        let request_name = request.name();
        let candidates = request.candidates();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = port_assignment_in_transaction(&transaction, &request_name)?
            && is_available(existing.port)
        {
            transaction.commit()?;

            return Ok(existing);
        }

        let assigned_ports =
            assigned_port_numbers_except_in_transaction(&transaction, &request_name)?;

        for candidate in candidates.iter().copied() {
            if assigned_ports.contains(&candidate) || !is_available(candidate) {
                continue;
            }

            let updated_at = timestamp()?;
            upsert_port_in_transaction(&transaction, &request, candidate, &updated_at)?;
            transaction.commit()?;

            return Ok(PortAssignment {
                name: request_name,
                port: candidate,
                owner_kind: request.owner.kind().to_string(),
                resource_name: request.owner.resource_name(),
                track: request.owner.track(),
                updated_at,
            });
        }

        Err(StateError::NoAvailablePort {
            name: request_name,
            preferred_port: request.preferred_port,
            fallback_start: request.fallback_start,
            fallback_end: request.fallback_end,
            attempts: candidates.len(),
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

    pub fn project_config_watches(&self) -> Result<Vec<ProjectConfigWatch>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT id, config_path FROM projects WHERE config_path IS NOT NULL ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProjectConfigWatch {
                project_id: row.get(0)?,
                config_path: Utf8PathBuf::from(row.get::<_, String>(1)?),
            })
        })?;
        let mut watches = Vec::new();

        for row in rows {
            watches.push(row?);
        }

        Ok(watches)
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

impl PortRequest {
    pub fn dns(preferred_port: u16, fallback_start: u16, fallback_end: u16) -> Self {
        Self::new(PortOwner::Dns, preferred_port, fallback_start, fallback_end)
    }

    pub fn gateway(preferred_port: u16, fallback_start: u16, fallback_end: u16) -> Self {
        Self::new(
            PortOwner::Gateway,
            preferred_port,
            fallback_start,
            fallback_end,
        )
    }

    pub fn project_worker(
        project_id: impl Into<String>,
        php_track: impl Into<String>,
        preferred_port: u16,
        fallback_start: u16,
        fallback_end: u16,
    ) -> Self {
        Self::new(
            PortOwner::ProjectWorker {
                project_id: project_id.into(),
                php_track: php_track.into(),
            },
            preferred_port,
            fallback_start,
            fallback_end,
        )
    }

    pub fn resource(
        name: impl Into<String>,
        track: impl Into<String>,
        preferred_port: u16,
        fallback_start: u16,
        fallback_end: u16,
    ) -> Self {
        Self::new(
            PortOwner::Resource {
                name: name.into(),
                track: track.into(),
            },
            preferred_port,
            fallback_start,
            fallback_end,
        )
    }

    pub fn new(
        owner: PortOwner,
        preferred_port: u16,
        fallback_start: u16,
        fallback_end: u16,
    ) -> Self {
        Self {
            owner,
            preferred_port,
            fallback_start,
            fallback_end,
        }
    }

    pub fn name(&self) -> String {
        self.owner.storage_key()
    }

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

impl PortOwner {
    fn storage_key(&self) -> String {
        match self {
            Self::Dns => "dns".to_string(),
            Self::Gateway => "gateway".to_string(),
            Self::ProjectWorker {
                project_id,
                php_track,
            } => format!("project:{project_id}:php:{php_track}"),
            Self::Resource { name, track } => format!("resource:{name}:{track}"),
        }
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Gateway => "gateway",
            Self::ProjectWorker { .. } => "project_worker",
            Self::Resource { .. } => "resource",
        }
    }

    fn resource_name(&self) -> Option<String> {
        match self {
            Self::Resource { name, .. } => Some(name.clone()),
            Self::Dns | Self::Gateway | Self::ProjectWorker { .. } => None,
        }
    }

    fn track(&self) -> Option<String> {
        match self {
            Self::ProjectWorker { php_track, .. } => Some(php_track.clone()),
            Self::Resource { track, .. } => Some(track.clone()),
            Self::Dns | Self::Gateway => None,
        }
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

fn port_assignment_in_transaction(
    transaction: &Transaction<'_>,
    name: &str,
) -> rusqlite::Result<Option<PortAssignment>> {
    let mut statement = transaction.prepare(
        "SELECT name, port, owner_kind, resource_name, track, updated_at FROM ports WHERE name = ?1",
    )?;
    let mut rows = statement.query_map(params![name], port_assignment_from_row)?;

    match rows.next() {
        Some(row) => Ok(Some(row?.into_assignment())),
        None => Ok(None),
    }
}

fn assigned_port_numbers_except_in_transaction(
    transaction: &Transaction<'_>,
    name: &str,
) -> rusqlite::Result<BTreeSet<u16>> {
    let mut statement =
        transaction.prepare("SELECT port FROM ports WHERE name != ?1 ORDER BY port")?;
    let rows = statement.query_map(params![name], |row| row.get::<_, u16>(0))?;
    let mut ports = BTreeSet::new();

    for row in rows {
        ports.insert(row?);
    }

    Ok(ports)
}

fn upsert_port_in_transaction(
    transaction: &Transaction<'_>,
    request: &PortRequest,
    port: u16,
    updated_at: &str,
) -> rusqlite::Result<()> {
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
            request.name(),
            port,
            request.owner.kind(),
            request.owner.resource_name(),
            request.owner.track(),
            updated_at,
        ],
    )?;

    Ok(())
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
