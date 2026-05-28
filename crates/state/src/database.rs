use std::collections::BTreeSet;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
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
    pub owner: PortOwner,
    pub port: u16,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigWatch {
    pub project_id: String,
    pub config_path: Utf8PathBuf,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ManagedResourceDesiredState {
    Installed,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedResourceTrackRecord {
    pub resource_name: String,
    pub track: String,
    pub desired_state: ManagedResourceDesiredState,
    pub installed_version: Option<String>,
    pub current_artifact_path: Option<Utf8PathBuf>,
    pub usage_count: i64,
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
    owner_kind: String,
    owner_id: String,
    owner_track: String,
    port: u16,
    updated_at: String,
}

struct ManagedResourceTrackRow {
    resource_name: String,
    track: String,
    desired_state: String,
    installed_version: Option<String>,
    current_artifact_path: Option<String>,
    usage_count: i64,
    updated_at: String,
}

struct PortIdentity {
    owner_kind: &'static str,
    owner_id: String,
    owner_track: String,
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

    pub fn record_managed_resource_track_desired(
        &mut self,
        resource_name: &str,
        track: &str,
        desired_state: ManagedResourceDesiredState,
    ) -> Result<ManagedResourceTrackRecord, StateError> {
        validate_managed_resource_identity("name", resource_name)?;
        validate_managed_resource_identity("track", track)?;

        let updated_at = timestamp()?;
        self.connection.execute(
            "INSERT INTO managed_resource_tracks (resource_name, track, desired_state, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(resource_name, track) DO UPDATE SET
                desired_state = excluded.desired_state,
                updated_at = excluded.updated_at",
            params![resource_name, track, desired_state.as_str(), updated_at],
        )?;

        self.managed_resource_track(resource_name, track)
    }

    pub fn record_managed_resource_track_installed(
        &mut self,
        resource_name: &str,
        track: &str,
        installed_version: &str,
        current_artifact_path: &Utf8Path,
    ) -> Result<ManagedResourceTrackRecord, StateError> {
        validate_managed_resource_identity("name", resource_name)?;
        validate_managed_resource_identity("track", track)?;
        validate_managed_resource_identity("artifact version", installed_version)?;

        let updated_at = timestamp()?;
        self.connection.execute(
            "INSERT INTO managed_resource_tracks (
                resource_name,
                track,
                desired_state,
                installed_version,
                current_artifact_path,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(resource_name, track) DO UPDATE SET
                installed_version = excluded.installed_version,
                current_artifact_path = excluded.current_artifact_path,
                updated_at = excluded.updated_at",
            params![
                resource_name,
                track,
                ManagedResourceDesiredState::Installed.as_str(),
                installed_version,
                current_artifact_path.as_str(),
                updated_at,
            ],
        )?;

        self.managed_resource_track(resource_name, track)
    }

    pub fn managed_resource_tracks(&self) -> Result<Vec<ManagedResourceTrackRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT resource_name, track, desired_state, installed_version, current_artifact_path, usage_count, updated_at
            FROM managed_resource_tracks
            ORDER BY resource_name, track",
        )?;
        let rows = statement.query_map([], managed_resource_track_from_row)?;
        let mut tracks = Vec::new();

        for row in rows {
            tracks.push(row?.into_record()?);
        }

        Ok(tracks)
    }

    pub fn assign_port(
        &mut self,
        request: PortRequest,
        mut is_available: impl FnMut(u16) -> bool,
    ) -> Result<PortAssignment, StateError> {
        let identity = request.owner.identity()?;
        let request_name = identity.display_name();
        let candidates = request.candidates();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = port_assignment_in_transaction(&transaction, &identity)?
            && is_available(existing.port)
        {
            transaction.commit()?;

            return Ok(existing);
        }

        let assigned_ports = assigned_port_numbers_except_in_transaction(&transaction, &identity)?;

        for candidate in candidates.iter().copied() {
            if assigned_ports.contains(&candidate) || !is_available(candidate) {
                continue;
            }

            let updated_at = timestamp()?;
            upsert_port_in_transaction(&transaction, &identity, candidate, &updated_at)?;
            transaction.commit()?;

            return Ok(PortAssignment {
                owner: request.owner,
                port: candidate,
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

    pub fn release_port(&mut self, owner: PortOwner) -> Result<bool, StateError> {
        let identity = owner.identity()?;
        let deleted = self.connection.execute(
            "DELETE FROM ports WHERE owner_kind = ?1 AND owner_id = ?2 AND owner_track = ?3",
            params![
                identity.owner_kind,
                identity.owner_id.as_str(),
                identity.owner_track.as_str(),
            ],
        )?;

        Ok(deleted > 0)
    }

    pub fn assigned_ports(&self) -> Result<Vec<PortAssignment>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT owner_kind, owner_id, owner_track, port, updated_at FROM ports ORDER BY owner_kind, owner_id, owner_track",
        )?;
        let rows = statement.query_map([], port_assignment_from_row)?;
        let mut assignments = Vec::new();

        for row in rows {
            assignments.push(row?.into_assignment()?);
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

    fn managed_resource_track(
        &self,
        resource_name: &str,
        track: &str,
    ) -> Result<ManagedResourceTrackRecord, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT resource_name, track, desired_state, installed_version, current_artifact_path, usage_count, updated_at
            FROM managed_resource_tracks
            WHERE resource_name = ?1 AND track = ?2",
        )?;
        let row = statement.query_row(
            params![resource_name, track],
            managed_resource_track_from_row,
        )?;

        row.into_record()
    }
}

impl ManagedResourceDesiredState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Removed => "removed",
        }
    }

    fn from_database(desired_state: String) -> Result<Self, StateError> {
        match desired_state.as_str() {
            "installed" => Ok(Self::Installed),
            "removed" => Ok(Self::Removed),
            _ => Err(StateError::UnknownManagedResourceDesiredState { desired_state }),
        }
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
        self.owner.display_name()
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
    fn identity(&self) -> Result<PortIdentity, StateError> {
        match self {
            Self::Dns => Ok(PortIdentity {
                owner_kind: "dns",
                owner_id: "dns".to_string(),
                owner_track: String::new(),
            }),
            Self::Gateway => Ok(PortIdentity {
                owner_kind: "gateway",
                owner_id: "gateway".to_string(),
                owner_track: String::new(),
            }),
            Self::ProjectWorker {
                project_id,
                php_track,
            } => {
                self.validate_component("project_id", project_id)?;
                self.validate_component("php_track", php_track)?;

                Ok(PortIdentity {
                    owner_kind: "project_worker",
                    owner_id: project_id.clone(),
                    owner_track: php_track.clone(),
                })
            }
            Self::Resource { name, track } => {
                self.validate_component("name", name)?;
                self.validate_component("track", track)?;

                Ok(PortIdentity {
                    owner_kind: "resource",
                    owner_id: name.clone(),
                    owner_track: track.clone(),
                })
            }
        }
    }

    fn from_database(
        owner_kind: String,
        owner_id: String,
        owner_track: String,
    ) -> Result<Self, StateError> {
        match owner_kind.as_str() {
            "dns" if owner_id == "dns" && owner_track.is_empty() => Ok(Self::Dns),
            "dns" => Err(StateError::InvalidPortOwner {
                owner: describe_port_identity(&owner_kind, &owner_id, &owner_track),
                reason: "dns ports must use owner id `dns` and an empty owner track",
            }),
            "gateway" if owner_id == "gateway" && owner_track.is_empty() => Ok(Self::Gateway),
            "gateway" => Err(StateError::InvalidPortOwner {
                owner: describe_port_identity(&owner_kind, &owner_id, &owner_track),
                reason: "gateway ports must use owner id `gateway` and an empty owner track",
            }),
            "project_worker" if !owner_id.is_empty() && !owner_track.is_empty() => {
                Ok(Self::ProjectWorker {
                    project_id: owner_id,
                    php_track: owner_track,
                })
            }
            "project_worker" => Err(StateError::InvalidPortOwner {
                owner: describe_port_identity(&owner_kind, &owner_id, &owner_track),
                reason: "project worker ports must include a project id and php track",
            }),
            "resource" if !owner_id.is_empty() && !owner_track.is_empty() => Ok(Self::Resource {
                name: owner_id,
                track: owner_track,
            }),
            "resource" => Err(StateError::InvalidPortOwner {
                owner: describe_port_identity(&owner_kind, &owner_id, &owner_track),
                reason: "resource ports must include a resource name and track",
            }),
            _ => Err(StateError::UnknownPortOwnerKind { owner_kind }),
        }
    }

    fn validate_component(&self, component: &'static str, value: &str) -> Result<(), StateError> {
        if value.is_empty() {
            return Err(StateError::InvalidPortOwner {
                owner: self.display_name(),
                reason: match component {
                    "project_id" => "project worker port owner project id must not be empty",
                    "php_track" => "project worker port owner php track must not be empty",
                    "name" => "resource port owner name must not be empty",
                    "track" => "resource port owner track must not be empty",
                    _ => "port owner component must not be empty",
                },
            });
        }

        Ok(())
    }

    fn display_name(&self) -> String {
        match self {
            Self::Dns => "dns".to_string(),
            Self::Gateway => "gateway".to_string(),
            Self::ProjectWorker {
                project_id,
                php_track,
            } => format!("project worker {project_id:?} php {php_track:?}"),
            Self::Resource { name, track } => format!("resource {name:?} track {track:?}"),
        }
    }
}

impl PortAssignmentRow {
    fn into_assignment(self) -> Result<PortAssignment, StateError> {
        let owner = PortOwner::from_database(self.owner_kind, self.owner_id, self.owner_track)?;

        Ok(PortAssignment {
            owner,
            port: self.port,
            updated_at: self.updated_at,
        })
    }
}

impl ManagedResourceTrackRow {
    fn into_record(self) -> Result<ManagedResourceTrackRecord, StateError> {
        validate_managed_resource_identity("name", &self.resource_name)?;
        validate_managed_resource_identity("track", &self.track)?;
        if let Some(installed_version) = &self.installed_version {
            validate_managed_resource_identity("artifact version", installed_version)?;
        }

        Ok(ManagedResourceTrackRecord {
            resource_name: self.resource_name,
            track: self.track,
            desired_state: ManagedResourceDesiredState::from_database(self.desired_state)?,
            installed_version: self.installed_version,
            current_artifact_path: self.current_artifact_path.map(Utf8PathBuf::from),
            usage_count: self.usage_count,
            updated_at: self.updated_at,
        })
    }
}

impl PortIdentity {
    fn display_name(&self) -> String {
        describe_port_identity(self.owner_kind, &self.owner_id, &self.owner_track)
    }
}

fn port_assignment_in_transaction(
    transaction: &Transaction<'_>,
    identity: &PortIdentity,
) -> Result<Option<PortAssignment>, StateError> {
    let mut statement = transaction.prepare(
        "SELECT owner_kind, owner_id, owner_track, port, updated_at
        FROM ports
        WHERE owner_kind = ?1 AND owner_id = ?2 AND owner_track = ?3",
    )?;
    let mut rows = statement.query_map(
        params![
            identity.owner_kind,
            identity.owner_id.as_str(),
            identity.owner_track.as_str(),
        ],
        port_assignment_from_row,
    )?;

    match rows.next() {
        Some(row) => Ok(Some(row?.into_assignment()?)),
        None => Ok(None),
    }
}

fn assigned_port_numbers_except_in_transaction(
    transaction: &Transaction<'_>,
    identity: &PortIdentity,
) -> Result<BTreeSet<u16>, StateError> {
    let mut statement = transaction.prepare(
        "SELECT port
        FROM ports
        WHERE owner_kind != ?1 OR owner_id != ?2 OR owner_track != ?3
        ORDER BY port",
    )?;
    let rows = statement.query_map(
        params![
            identity.owner_kind,
            identity.owner_id.as_str(),
            identity.owner_track.as_str(),
        ],
        |row| row.get::<_, u16>(0),
    )?;
    let mut ports = BTreeSet::new();

    for row in rows {
        ports.insert(row?);
    }

    Ok(ports)
}

fn upsert_port_in_transaction(
    transaction: &Transaction<'_>,
    identity: &PortIdentity,
    port: u16,
    updated_at: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO ports (owner_kind, owner_id, owner_track, port, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(owner_kind, owner_id, owner_track) DO UPDATE SET
            port = excluded.port,
            updated_at = excluded.updated_at",
        params![
            identity.owner_kind,
            identity.owner_id.as_str(),
            identity.owner_track.as_str(),
            port,
            updated_at,
        ],
    )?;

    Ok(())
}

fn port_assignment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PortAssignmentRow> {
    Ok(PortAssignmentRow {
        owner_kind: row.get(0)?,
        owner_id: row.get(1)?,
        owner_track: row.get(2)?,
        port: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn managed_resource_track_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ManagedResourceTrackRow> {
    Ok(ManagedResourceTrackRow {
        resource_name: row.get(0)?,
        track: row.get(1)?,
        desired_state: row.get(2)?,
        installed_version: row.get(3)?,
        current_artifact_path: row.get(4)?,
        usage_count: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn validate_managed_resource_identity(kind: &'static str, value: &str) -> Result<(), StateError> {
    let is_valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));

    if is_valid {
        Ok(())
    } else {
        Err(StateError::InvalidManagedResourceIdentity {
            kind,
            value: value.to_string(),
        })
    }
}

fn describe_port_identity(owner_kind: &str, owner_id: &str, owner_track: &str) -> String {
    if owner_track.is_empty() {
        return format!("{owner_kind} {owner_id:?}");
    }

    format!("{owner_kind} {owner_id:?} track {owner_track:?}")
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
