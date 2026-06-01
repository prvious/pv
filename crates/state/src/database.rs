use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{PvPaths, StateError, fs, migrations};

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
const RECENT_JOB_LIMIT: i64 = 100;
const PORT_CANDIDATE_LIMIT: usize = 10;
const PROJECT_ID_LENGTH: usize = 10;
const PROJECT_ID_ATTEMPTS: usize = 16;
const PROJECT_ID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const MAX_DNS_LABEL_LENGTH: usize = 63;
const MAX_HOSTNAME_LENGTH: usize = 253;
const RESERVED_HOSTNAME: &str = "pv.test";
const RESERVED_TRACK_NAME: &str = "latest";
const PROJECT_ENV_OBSERVED_SUBJECT_KIND: &str = "project_env";

pub type EnvContextValues = BTreeMap<String, String>;

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
    pub project_path: Utf8PathBuf,
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
    pub env: EnvContextValues,
    pub usage_count: i64,
    pub removal_prune: bool,
    pub removal_force: bool,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkProjectInput {
    pub path: Utf8PathBuf,
    pub original_path: Utf8PathBuf,
    pub primary_hostname: String,
    pub config_path: Utf8PathBuf,
    pub desired_php_track: Option<String>,
    pub additional_hostnames: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkProjectResult {
    pub status: LinkProjectStatus,
    pub project: ProjectRecord,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkProjectStatus {
    Created,
    Updated,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRecord {
    pub id: String,
    pub path: Utf8PathBuf,
    pub original_path: Utf8PathBuf,
    pub primary_hostname: String,
    pub config_path: Utf8PathBuf,
    pub desired_php_track: Option<String>,
    pub additional_hostnames: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectManagedResourceInput {
    pub resource_name: String,
    pub track: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectManagedResourceRecord {
    pub project_id: String,
    pub resource_name: String,
    pub track: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceAllocationInput {
    pub allocation_name: String,
    pub generated_name: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ResourceAllocationStatus {
    Desired,
    Ready,
    Inactive,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceAllocationRecord {
    pub id: String,
    pub project_id: String,
    pub resource_name: String,
    pub track: String,
    pub allocation_name: String,
    pub generated_name: String,
    pub env: EnvContextValues,
    pub status: ResourceAllocationStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEnvStateContext {
    pub project_id: String,
    pub primary_hostname: String,
    pub resources: BTreeMap<String, ProjectEnvResourceContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEnvResourceContext {
    pub track: String,
    pub values: EnvContextValues,
    pub allocations: BTreeMap<String, ProjectEnvAllocationContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEnvAllocationContext {
    pub generated_name: String,
    pub values: EnvContextValues,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ProjectEnvObservedStatus {
    Pending,
    Rendered,
    Warning,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEnvObservedWarningInput {
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEnvObservedWarningRecord {
    pub project_id: String,
    pub kind: String,
    pub message: String,
    pub observed_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEnvObservedStateRecord {
    pub project_id: String,
    pub status: ProjectEnvObservedStatus,
    pub message: Option<String>,
    pub observed_at: String,
    pub warnings: Vec<ProjectEnvObservedWarningRecord>,
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
    env_json: String,
    usage_count: i64,
    removal_prune: bool,
    removal_force: bool,
    updated_at: String,
}

struct ProjectManagedResourceRow {
    project_id: String,
    resource_name: String,
    track: String,
    created_at: String,
    updated_at: String,
}

struct ResourceAllocationRow {
    id: String,
    project_id: String,
    resource_name: String,
    track: String,
    allocation_name: String,
    generated_name: String,
    env_json: String,
    status: String,
    created_at: String,
    updated_at: String,
}

struct ProjectEnvObservedStateRow {
    project_id: String,
    status: String,
    message: Option<String>,
    observed_at: String,
}

struct ProjectEnvObservedWarningRow {
    project_id: String,
    kind: String,
    message: String,
    observed_at: String,
}

struct ProjectRow {
    id: String,
    path: String,
    original_path: Option<String>,
    primary_hostname: String,
    config_path: Option<String>,
    desired_php_track: Option<String>,
    created_at: String,
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

    pub fn link_project(
        &mut self,
        input: LinkProjectInput,
    ) -> Result<LinkProjectResult, StateError> {
        validate_link_project_input(&input)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = project_by_path_in_transaction(&transaction, &input.path)?;
        let project_id = match &existing {
            Some(project) => project.id.clone(),
            None => generate_project_id(&transaction)?,
        };
        validate_project_hostnames_in_transaction(&transaction, &project_id, &input)?;
        let status = match &existing {
            Some(project) if project_matches_input(project, &input) => LinkProjectStatus::Unchanged,
            Some(_) => LinkProjectStatus::Updated,
            None => LinkProjectStatus::Created,
        };

        match &existing {
            Some(project) => {
                delete_same_project_additional_hostname_in_transaction(
                    &transaction,
                    &project.id,
                    &input.primary_hostname,
                )?;
                update_project_in_transaction(&transaction, &project.id, &input)?;
            }
            None => insert_project_in_transaction(&transaction, &project_id, &input)?,
        }
        replace_project_hostnames_in_transaction(&transaction, &project_id, &input)?;
        let project =
            project_by_id_in_transaction(&transaction, &project_id)?.ok_or_else(|| {
                StateError::ProjectNotFound {
                    target: project_id.clone(),
                }
            })?;
        transaction.commit()?;

        Ok(LinkProjectResult { status, project })
    }

    pub fn unlink_project(&mut self, project_id: &str) -> Result<ProjectRecord, StateError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let project = project_by_id_in_transaction(&transaction, project_id)?.ok_or_else(|| {
            StateError::ProjectNotFound {
                target: project_id.to_string(),
            }
        })?;
        transaction.execute(
            "DELETE FROM observed_states WHERE subject_kind = ?1 AND subject_id = ?2",
            params![PROJECT_ENV_OBSERVED_SUBJECT_KIND, project_id],
        )?;
        let deleted =
            transaction.execute("DELETE FROM projects WHERE id = ?1", params![project_id])?;
        if deleted == 0 {
            return Err(StateError::ProjectNotFound {
                target: project_id.to_string(),
            });
        }
        recalculate_managed_resource_usage_counts_in_transaction(&transaction)?;
        transaction.commit()?;

        Ok(project)
    }

    pub fn validate_project_hostnames(
        &self,
        project_id: &str,
        primary_hostname: &str,
        additional_hostnames: &[String],
    ) -> Result<(), StateError> {
        validate_project_hostname_set(
            &self.connection,
            project_id,
            primary_hostname,
            additional_hostnames,
        )
    }

    pub fn replace_project_additional_hostnames(
        &mut self,
        project_id: &str,
        additional_hostnames: &[String],
    ) -> Result<ProjectRecord, StateError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let project = project_by_id_in_transaction(&transaction, project_id)?.ok_or_else(|| {
            StateError::ProjectNotFound {
                target: project_id.to_string(),
            }
        })?;
        validate_project_hostname_set(
            &transaction,
            project_id,
            &project.primary_hostname,
            additional_hostnames,
        )?;

        if sorted_hostnames(&project.additional_hostnames) != sorted_hostnames(additional_hostnames)
        {
            let input = LinkProjectInput {
                path: project.path.clone(),
                original_path: project.original_path.clone(),
                primary_hostname: project.primary_hostname.clone(),
                config_path: project.config_path.clone(),
                desired_php_track: project.desired_php_track.clone(),
                additional_hostnames: additional_hostnames.to_vec(),
            };
            update_project_in_transaction(&transaction, project_id, &input)?;
            replace_project_hostnames_in_transaction(&transaction, project_id, &input)?;
        }

        let project = project_by_id_in_transaction(&transaction, project_id)?.ok_or_else(|| {
            StateError::ProjectNotFound {
                target: project_id.to_string(),
            }
        })?;
        transaction.commit()?;

        Ok(project)
    }

    pub fn project_by_id(&self, project_id: &str) -> Result<Option<ProjectRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT id, path, original_path, primary_hostname, config_path, desired_php_track, created_at, updated_at
            FROM projects
            WHERE id = ?1",
        )?;
        let mut rows = statement.query_map(params![project_id], project_from_row)?;

        match rows.next() {
            Some(row) => row?.into_record(&self.connection).map(Some),
            None => Ok(None),
        }
    }

    pub fn project_by_path(&self, path: &Utf8Path) -> Result<Option<ProjectRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT id, path, original_path, primary_hostname, config_path, desired_php_track, created_at, updated_at
            FROM projects
            WHERE path = ?1",
        )?;
        let mut rows = statement.query_map(params![path.as_str()], project_from_row)?;

        match rows.next() {
            Some(row) => row?.into_record(&self.connection).map(Some),
            None => Ok(None),
        }
    }

    pub fn project_by_hostname(&self, hostname: &str) -> Result<Option<ProjectRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT projects.id, projects.path, projects.original_path, projects.primary_hostname, projects.config_path, projects.desired_php_track, projects.created_at, projects.updated_at
            FROM projects
            INNER JOIN project_hostnames ON project_hostnames.project_id = projects.id
            WHERE project_hostnames.hostname = ?1",
        )?;
        let mut rows = statement.query_map(params![hostname], project_from_row)?;

        match rows.next() {
            Some(row) => row?.into_record(&self.connection).map(Some),
            None => Ok(None),
        }
    }

    pub fn nearest_project_for_path(
        &self,
        path: &Utf8Path,
    ) -> Result<Option<ProjectRecord>, StateError> {
        Ok(self
            .projects()?
            .into_iter()
            .filter(|project| path.starts_with(&project.path))
            .max_by_key(|project| project.path.as_str().len()))
    }

    pub fn projects(&self) -> Result<Vec<ProjectRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT id, path, original_path, primary_hostname, config_path, desired_php_track, created_at, updated_at
            FROM projects
            ORDER BY primary_hostname",
        )?;
        let rows = statement.query_map([], project_from_row)?;
        let mut projects = Vec::new();

        for row in rows {
            projects.push(row?.into_record(&self.connection)?);
        }

        Ok(projects)
    }

    pub fn replace_project_managed_resources(
        &mut self,
        project_id: &str,
        requirements: &[ProjectManagedResourceInput],
    ) -> Result<Vec<ProjectManagedResourceRecord>, StateError> {
        validate_project_managed_resource_inputs(requirements)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project_exists_in_transaction(&transaction, project_id)?;
        let updated_at = timestamp()?;

        for requirement in requirements {
            upsert_managed_resource_track_desired_in_transaction(
                &transaction,
                &requirement.resource_name,
                &requirement.track,
                &updated_at,
            )?;
            transaction.execute(
                "INSERT INTO project_managed_resources (
                    project_id,
                    resource_name,
                    track,
                    created_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?4)
                ON CONFLICT(project_id, resource_name) DO UPDATE SET
                    track = excluded.track,
                    updated_at = excluded.updated_at",
                params![
                    project_id,
                    requirement.resource_name.as_str(),
                    requirement.track.as_str(),
                    updated_at.as_str(),
                ],
            )?;
        }

        let desired_resources = requirements
            .iter()
            .map(|requirement| requirement.resource_name.as_str())
            .collect::<BTreeSet<_>>();
        let existing_resources =
            project_managed_resource_names_in_transaction(&transaction, project_id)?;
        for resource_name in existing_resources {
            if !desired_resources.contains(resource_name.as_str()) {
                transaction.execute(
                    "DELETE FROM project_managed_resources
                    WHERE project_id = ?1
                    AND resource_name = ?2",
                    params![project_id, resource_name.as_str()],
                )?;
            }
        }

        recalculate_managed_resource_usage_counts_in_transaction(&transaction)?;
        let records = project_managed_resources_in_transaction(&transaction, project_id)?;
        transaction.commit()?;

        Ok(records)
    }

    pub fn project_managed_resources(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProjectManagedResourceRecord>, StateError> {
        project_managed_resources_in_connection(&self.connection, project_id)
    }

    pub fn record_managed_resource_track_desired(
        &mut self,
        resource_name: &str,
        track: &str,
        desired_state: ManagedResourceDesiredState,
    ) -> Result<ManagedResourceTrackRecord, StateError> {
        validate_managed_resource_identity("name", resource_name)?;
        validate_concrete_track(track)?;

        let updated_at = timestamp()?;
        self.connection.execute(
            "INSERT INTO managed_resource_tracks (
                resource_name,
                track,
                desired_state,
                removal_prune,
                removal_force,
                updated_at
            )
            VALUES (?1, ?2, ?3, 0, 0, ?4)
            ON CONFLICT(resource_name, track) DO UPDATE SET
                desired_state = excluded.desired_state,
                removal_prune = 0,
                removal_force = 0,
                updated_at = excluded.updated_at",
            params![resource_name, track, desired_state.as_str(), updated_at],
        )?;

        self.managed_resource_track(resource_name, track)
    }

    pub fn record_managed_resource_track_removal_intent(
        &mut self,
        resource_name: &str,
        track: &str,
        prune: bool,
        force: bool,
    ) -> Result<ManagedResourceTrackRecord, StateError> {
        validate_managed_resource_identity("name", resource_name)?;
        validate_concrete_track(track)?;

        let updated_at = timestamp()?;
        self.connection.execute(
            "INSERT INTO managed_resource_tracks (
                resource_name,
                track,
                desired_state,
                removal_prune,
                removal_force,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(resource_name, track) DO UPDATE SET
                desired_state = excluded.desired_state,
                removal_prune = excluded.removal_prune,
                removal_force = excluded.removal_force,
                updated_at = excluded.updated_at",
            params![
                resource_name,
                track,
                ManagedResourceDesiredState::Removed.as_str(),
                prune,
                force,
                updated_at
            ],
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
        validate_concrete_track(track)?;
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

    pub fn record_managed_resource_track_env_context(
        &mut self,
        resource_name: &str,
        track: &str,
        env: &EnvContextValues,
    ) -> Result<ManagedResourceTrackRecord, StateError> {
        validate_managed_resource_identity("name", resource_name)?;
        validate_concrete_track(track)?;
        let context = format!("managed resource {resource_name:?} track {track:?}");
        let env_json = serialize_env_context(&context, env)?;
        let updated_at = timestamp()?;

        self.connection.execute(
            "INSERT INTO managed_resource_tracks (
                resource_name,
                track,
                desired_state,
                env_json,
                removal_prune,
                removal_force,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, 0, 0, ?5)
            ON CONFLICT(resource_name, track) DO UPDATE SET
                env_json = excluded.env_json,
                updated_at = excluded.updated_at",
            params![
                resource_name,
                track,
                ManagedResourceDesiredState::Installed.as_str(),
                env_json,
                updated_at,
            ],
        )?;

        self.managed_resource_track(resource_name, track)
    }

    pub fn managed_resource_tracks(&self) -> Result<Vec<ManagedResourceTrackRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT resource_name, track, desired_state, installed_version, current_artifact_path, env_json, usage_count, removal_prune, removal_force, updated_at
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

    pub fn replace_project_resource_allocations(
        &mut self,
        project_id: &str,
        resource_name: &str,
        track: &str,
        allocations: &[ResourceAllocationInput],
    ) -> Result<Vec<ResourceAllocationRecord>, StateError> {
        validate_resource_allocation_group(resource_name, track, allocations)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project_exists_in_transaction(&transaction, project_id)?;
        let updated_at = timestamp()?;
        let existing_allocations = resource_allocations_for_project_resource_in_transaction(
            &transaction,
            project_id,
            resource_name,
        )?;
        let existing_by_name = existing_allocations
            .into_iter()
            .map(|allocation| (allocation.allocation_name.clone(), allocation))
            .collect::<BTreeMap<_, _>>();
        let desired_allocation_names = allocations
            .iter()
            .map(|allocation| allocation.allocation_name.as_str())
            .collect::<BTreeSet<_>>();

        for allocation in allocations {
            if let Some(existing) = existing_by_name.get(&allocation.allocation_name) {
                let status = if existing.track == track
                    && existing.status == ResourceAllocationStatus::Ready
                {
                    ResourceAllocationStatus::Ready
                } else {
                    ResourceAllocationStatus::Desired
                };
                transaction.execute(
                    "UPDATE resource_allocations
                    SET track = ?1,
                        status = ?2,
                        updated_at = ?3
                    WHERE id = ?4",
                    params![
                        track,
                        status.as_str(),
                        updated_at.as_str(),
                        existing.id.as_str()
                    ],
                )?;
            } else {
                let id = next_resource_allocation_id(&transaction)?;
                transaction.execute(
                    "INSERT INTO resource_allocations (
                        id,
                        project_id,
                        resource_name,
                        track,
                        allocation_name,
                        generated_name,
                        env_json,
                        status,
                        created_at,
                        updated_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}', ?7, ?8, ?8)",
                    params![
                        id,
                        project_id,
                        resource_name,
                        track,
                        allocation.allocation_name.as_str(),
                        allocation.generated_name.as_str(),
                        ResourceAllocationStatus::Desired.as_str(),
                        updated_at.as_str(),
                    ],
                )?;
            }
        }

        for existing in existing_by_name.values() {
            if !desired_allocation_names.contains(existing.allocation_name.as_str())
                && existing.status != ResourceAllocationStatus::Inactive
            {
                transaction.execute(
                    "UPDATE resource_allocations
                    SET status = ?1,
                        updated_at = ?2
                    WHERE id = ?3",
                    params![
                        ResourceAllocationStatus::Inactive.as_str(),
                        updated_at.as_str(),
                        existing.id.as_str(),
                    ],
                )?;
            }
        }

        let records = resource_allocations_for_project_resource_in_transaction(
            &transaction,
            project_id,
            resource_name,
        )?
        .into_iter()
        .filter(|allocation| desired_allocation_names.contains(allocation.allocation_name.as_str()))
        .collect::<Vec<_>>();
        transaction.commit()?;

        Ok(records)
    }

    pub fn mark_resource_allocation_ready(
        &mut self,
        project_id: &str,
        resource_name: &str,
        track: &str,
        allocation_name: &str,
        env: &EnvContextValues,
    ) -> Result<ResourceAllocationRecord, StateError> {
        validate_resource_allocation_identity("resource", resource_name)?;
        validate_concrete_track(track)?;
        validate_resource_allocation_name(allocation_name)?;
        let context = format!(
            "resource allocation {project_id:?}/{resource_name:?}/{track:?}/{allocation_name:?}"
        );
        let env_json = serialize_env_context(&context, env)?;
        let updated_at = timestamp()?;
        let updated = self.connection.execute(
            "UPDATE resource_allocations
            SET status = ?1,
                env_json = ?2,
                updated_at = ?3
            WHERE project_id = ?4
            AND resource_name = ?5
            AND track = ?6
            AND allocation_name = ?7
            AND status = ?8",
            params![
                ResourceAllocationStatus::Ready.as_str(),
                env_json,
                updated_at,
                project_id,
                resource_name,
                track,
                allocation_name,
                ResourceAllocationStatus::Desired.as_str(),
            ],
        )?;
        if updated == 0 {
            resource_allocation_by_key(
                &self.connection,
                project_id,
                resource_name,
                allocation_name,
            )?;
            return Err(StateError::ResourceAllocationNotDesired {
                project_id: project_id.to_string(),
                resource: resource_name.to_string(),
                track: track.to_string(),
                allocation: allocation_name.to_string(),
            });
        }

        resource_allocation_by_key(&self.connection, project_id, resource_name, allocation_name)
    }

    pub fn resource_allocations(
        &self,
        project_id: &str,
        resource_name: &str,
    ) -> Result<Vec<ResourceAllocationRecord>, StateError> {
        resource_allocations_for_project_resource_in_connection(
            &self.connection,
            project_id,
            resource_name,
        )
    }

    pub fn project_env_context(
        &self,
        project_id: &str,
    ) -> Result<ProjectEnvStateContext, StateError> {
        let project =
            self.project_by_id(project_id)?
                .ok_or_else(|| StateError::ProjectNotFound {
                    target: project_id.to_string(),
                })?;
        let requirements = self.project_managed_resources(project_id)?;
        let mut resources = BTreeMap::new();

        for requirement in requirements {
            let track =
                self.managed_resource_track(&requirement.resource_name, &requirement.track)?;
            let allocations = ready_allocation_env_contexts(
                &self.connection,
                project_id,
                &requirement.resource_name,
                &requirement.track,
            )?;

            resources.insert(
                requirement.resource_name,
                ProjectEnvResourceContext {
                    track: requirement.track,
                    values: track.env,
                    allocations,
                },
            );
        }

        Ok(ProjectEnvStateContext {
            project_id: project.id,
            primary_hostname: project.primary_hostname,
            resources,
        })
    }

    pub fn record_project_env_observed_snapshot(
        &mut self,
        project_id: &str,
        status: ProjectEnvObservedStatus,
        message: Option<&str>,
        warnings: &[ProjectEnvObservedWarningInput],
    ) -> Result<ProjectEnvObservedStateRecord, StateError> {
        validate_project_env_observed_warnings(warnings)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_project_exists_in_transaction(&transaction, project_id)?;
        let observed_at = timestamp()?;
        transaction.execute(
            "INSERT INTO observed_states (
                subject_kind,
                subject_id,
                status,
                message,
                observed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(subject_kind, subject_id) DO UPDATE SET
                status = excluded.status,
                message = excluded.message,
                observed_at = excluded.observed_at",
            params![
                PROJECT_ENV_OBSERVED_SUBJECT_KIND,
                project_id,
                status.as_str(),
                message,
                observed_at,
            ],
        )?;
        transaction.execute(
            "DELETE FROM project_env_observed_warnings WHERE project_id = ?1",
            params![project_id],
        )?;

        for warning in warnings {
            transaction.execute(
                "INSERT INTO project_env_observed_warnings (
                    project_id,
                    warning_kind,
                    message,
                    observed_at
                )
                VALUES (?1, ?2, ?3, ?4)",
                params![
                    project_id,
                    warning.kind.as_str(),
                    warning.message.as_str(),
                    observed_at.as_str(),
                ],
            )?;
        }

        transaction.commit()?;

        self.project_env_observed_state(project_id)?
            .ok_or_else(|| StateError::ProjectNotFound {
                target: project_id.to_string(),
            })
    }

    pub fn project_env_observed_state(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectEnvObservedStateRecord>, StateError> {
        let row = self
            .connection
            .query_row(
                "SELECT subject_id, status, message, observed_at
                FROM observed_states
                WHERE subject_kind = ?1
                AND subject_id = ?2",
                params![PROJECT_ENV_OBSERVED_SUBJECT_KIND, project_id],
                project_env_observed_state_from_row,
            )
            .optional()?;

        match row {
            Some(row) => row.into_record(&self.connection).map(Some),
            None => Ok(None),
        }
    }

    pub fn project_env_observed_warnings(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProjectEnvObservedWarningRecord>, StateError> {
        project_env_observed_warnings_in_connection(&self.connection, project_id)
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
        let mut statement = self
            .connection
            .prepare("SELECT id, path FROM projects ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok(ProjectConfigWatch {
                project_id: row.get(0)?,
                project_path: Utf8PathBuf::from(row.get::<_, String>(1)?),
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
            "SELECT resource_name, track, desired_state, installed_version, current_artifact_path, env_json, usage_count, removal_prune, removal_force, updated_at
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
        let context = format!(
            "managed resource {:?} track {:?}",
            self.resource_name, self.track
        );

        Ok(ManagedResourceTrackRecord {
            resource_name: self.resource_name,
            track: self.track,
            desired_state: ManagedResourceDesiredState::from_database(self.desired_state)?,
            installed_version: self.installed_version,
            current_artifact_path: self.current_artifact_path.map(Utf8PathBuf::from),
            env: parse_env_context(&context, &self.env_json)?,
            usage_count: self.usage_count,
            removal_prune: self.removal_prune,
            removal_force: self.removal_force,
            updated_at: self.updated_at,
        })
    }
}

impl ProjectManagedResourceRow {
    fn into_record(self) -> Result<ProjectManagedResourceRecord, StateError> {
        validate_managed_resource_identity("name", &self.resource_name)?;
        validate_managed_resource_identity("track", &self.track)?;

        Ok(ProjectManagedResourceRecord {
            project_id: self.project_id,
            resource_name: self.resource_name,
            track: self.track,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

impl ResourceAllocationStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Desired => "desired",
            Self::Ready => "ready",
            Self::Inactive => "inactive",
            Self::Failed => "failed",
        }
    }

    fn from_database(status: String) -> Result<Self, StateError> {
        match status.as_str() {
            "desired" => Ok(Self::Desired),
            "ready" => Ok(Self::Ready),
            "inactive" => Ok(Self::Inactive),
            "failed" => Ok(Self::Failed),
            _ => Err(StateError::UnknownResourceAllocationStatus { status }),
        }
    }
}

impl ResourceAllocationRow {
    fn into_record(self) -> Result<ResourceAllocationRecord, StateError> {
        validate_resource_allocation_identity("resource", &self.resource_name)?;
        validate_concrete_track(&self.track)?;
        validate_resource_allocation_name(&self.allocation_name)?;
        validate_resource_allocation_identity("generated name", &self.generated_name)?;
        let context = format!(
            "resource allocation {:?}/{:?}/{:?}",
            self.project_id, self.resource_name, self.allocation_name
        );

        Ok(ResourceAllocationRecord {
            id: self.id,
            project_id: self.project_id,
            resource_name: self.resource_name,
            track: self.track,
            allocation_name: self.allocation_name,
            generated_name: self.generated_name,
            env: parse_env_context(&context, &self.env_json)?,
            status: ResourceAllocationStatus::from_database(self.status)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

impl ProjectEnvObservedStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Rendered => "rendered",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }

    fn from_database(status: String) -> Result<Self, StateError> {
        match status.as_str() {
            "pending" => Ok(Self::Pending),
            "rendered" => Ok(Self::Rendered),
            "warning" => Ok(Self::Warning),
            "failed" => Ok(Self::Failed),
            _ => Err(StateError::UnknownProjectEnvObservedStatus { status }),
        }
    }
}

impl ProjectEnvObservedStateRow {
    fn into_record(
        self,
        connection: &Connection,
    ) -> Result<ProjectEnvObservedStateRecord, StateError> {
        let warnings = project_env_observed_warnings_in_connection(connection, &self.project_id)?;

        Ok(ProjectEnvObservedStateRecord {
            project_id: self.project_id,
            status: ProjectEnvObservedStatus::from_database(self.status)?,
            message: self.message,
            observed_at: self.observed_at,
            warnings,
        })
    }
}

impl ProjectEnvObservedWarningRow {
    fn into_record(self) -> Result<ProjectEnvObservedWarningRecord, StateError> {
        validate_project_env_observed_warning_component("kind", &self.kind)?;
        validate_project_env_observed_warning_component("message", &self.message)?;

        Ok(ProjectEnvObservedWarningRecord {
            project_id: self.project_id,
            kind: self.kind,
            message: self.message,
            observed_at: self.observed_at,
        })
    }
}

impl ProjectRow {
    fn into_record(self, connection: &Connection) -> Result<ProjectRecord, StateError> {
        let additional_hostnames = additional_hostnames_for_project(connection, &self.id)?;
        let path = Utf8PathBuf::from(self.path);
        let original_path = self
            .original_path
            .map(Utf8PathBuf::from)
            .unwrap_or_else(|| path.clone());
        let config_path = self
            .config_path
            .map(Utf8PathBuf::from)
            .unwrap_or_else(|| path.join("pv.yml"));

        Ok(ProjectRecord {
            id: self.id,
            path,
            original_path,
            primary_hostname: self.primary_hostname,
            config_path,
            desired_php_track: self.desired_php_track,
            additional_hostnames,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

impl PortIdentity {
    fn display_name(&self) -> String {
        describe_port_identity(self.owner_kind, &self.owner_id, &self.owner_track)
    }
}

fn project_by_path_in_transaction(
    transaction: &Transaction<'_>,
    path: &Utf8Path,
) -> Result<Option<ProjectRecord>, StateError> {
    let row = transaction
        .query_row(
            "SELECT id, path, original_path, primary_hostname, config_path, desired_php_track, created_at, updated_at
            FROM projects
            WHERE path = ?1",
            params![path.as_str()],
            project_from_row,
        )
        .optional()?;

    match row {
        Some(row) => row.into_record(transaction).map(Some),
        None => Ok(None),
    }
}

fn project_by_id_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<Option<ProjectRecord>, StateError> {
    let row = transaction
        .query_row(
            "SELECT id, path, original_path, primary_hostname, config_path, desired_php_track, created_at, updated_at
            FROM projects
            WHERE id = ?1",
            params![project_id],
            project_from_row,
        )
        .optional()?;

    match row {
        Some(row) => row.into_record(transaction).map(Some),
        None => Ok(None),
    }
}

fn ensure_project_exists_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<(), StateError> {
    let count = transaction.query_row(
        "SELECT COUNT(*) FROM projects WHERE id = ?1",
        params![project_id],
        |row| row.get::<_, i64>(0),
    )?;
    if count == 0 {
        return Err(StateError::ProjectNotFound {
            target: project_id.to_string(),
        });
    }

    Ok(())
}

fn validate_project_managed_resource_inputs(
    requirements: &[ProjectManagedResourceInput],
) -> Result<(), StateError> {
    let mut resource_names = BTreeSet::new();

    for requirement in requirements {
        validate_managed_resource_identity("name", &requirement.resource_name)?;
        validate_concrete_track(&requirement.track)?;
        if !resource_names.insert(requirement.resource_name.as_str()) {
            return Err(StateError::InvalidManagedResourceIdentity {
                kind: "name",
                value: requirement.resource_name.clone(),
            });
        }
    }

    Ok(())
}

fn upsert_managed_resource_track_desired_in_transaction(
    transaction: &Transaction<'_>,
    resource_name: &str,
    track: &str,
    updated_at: &str,
) -> Result<(), StateError> {
    transaction.execute(
        "INSERT INTO managed_resource_tracks (
            resource_name,
            track,
            desired_state,
            removal_prune,
            removal_force,
            updated_at
        )
        VALUES (?1, ?2, ?3, 0, 0, ?4)
        ON CONFLICT(resource_name, track) DO UPDATE SET
            desired_state = excluded.desired_state,
            removal_prune = 0,
            removal_force = 0,
            updated_at = excluded.updated_at",
        params![
            resource_name,
            track,
            ManagedResourceDesiredState::Installed.as_str(),
            updated_at,
        ],
    )?;

    Ok(())
}

fn project_managed_resource_names_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<Vec<String>, StateError> {
    let mut statement = transaction.prepare(
        "SELECT resource_name
        FROM project_managed_resources
        WHERE project_id = ?1
        ORDER BY resource_name",
    )?;
    let rows = statement.query_map(params![project_id], |row| row.get::<_, String>(0))?;
    let mut resource_names = Vec::new();

    for row in rows {
        resource_names.push(row?);
    }

    Ok(resource_names)
}

fn project_managed_resources_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<Vec<ProjectManagedResourceRecord>, StateError> {
    project_managed_resources_in_connection(transaction, project_id)
}

fn project_managed_resources_in_connection(
    connection: &Connection,
    project_id: &str,
) -> Result<Vec<ProjectManagedResourceRecord>, StateError> {
    let mut statement = connection.prepare(
        "SELECT project_id, resource_name, track, created_at, updated_at
        FROM project_managed_resources
        WHERE project_id = ?1
        ORDER BY resource_name",
    )?;
    let rows = statement.query_map(params![project_id], project_managed_resource_from_row)?;
    let mut records = Vec::new();

    for row in rows {
        records.push(row?.into_record()?);
    }

    Ok(records)
}

fn recalculate_managed_resource_usage_counts_in_transaction(
    transaction: &Transaction<'_>,
) -> Result<(), StateError> {
    transaction.execute(
        "UPDATE managed_resource_tracks
        SET usage_count = (
            SELECT COUNT(*)
            FROM project_managed_resources
            WHERE project_managed_resources.resource_name = managed_resource_tracks.resource_name
            AND project_managed_resources.track = managed_resource_tracks.track
        )",
        [],
    )?;

    Ok(())
}

fn validate_resource_allocation_group(
    resource_name: &str,
    track: &str,
    allocations: &[ResourceAllocationInput],
) -> Result<(), StateError> {
    validate_resource_allocation_identity("resource", resource_name)?;
    validate_concrete_track(track)?;
    let mut allocation_names = BTreeSet::new();
    let mut generated_names = BTreeSet::new();

    for allocation in allocations {
        validate_resource_allocation_name(&allocation.allocation_name)?;
        validate_resource_allocation_identity("generated name", &allocation.generated_name)?;
        if !allocation_names.insert(allocation.allocation_name.as_str()) {
            return Err(StateError::InvalidResourceAllocationIdentity {
                kind: "allocation",
                value: allocation.allocation_name.clone(),
            });
        }
        if !generated_names.insert(allocation.generated_name.as_str()) {
            return Err(StateError::InvalidResourceAllocationIdentity {
                kind: "generated name",
                value: allocation.generated_name.clone(),
            });
        }
    }

    Ok(())
}

fn resource_allocations_for_project_resource_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
    resource_name: &str,
) -> Result<Vec<ResourceAllocationRecord>, StateError> {
    resource_allocations_for_project_resource_in_connection(transaction, project_id, resource_name)
}

fn resource_allocations_for_project_resource_in_connection(
    connection: &Connection,
    project_id: &str,
    resource_name: &str,
) -> Result<Vec<ResourceAllocationRecord>, StateError> {
    let mut statement = connection.prepare(
        "SELECT id, project_id, resource_name, track, allocation_name, generated_name, env_json, status, created_at, updated_at
        FROM resource_allocations
        WHERE project_id = ?1
        AND resource_name = ?2
        ORDER BY allocation_name",
    )?;
    let rows = statement.query_map(
        params![project_id, resource_name],
        resource_allocation_from_row,
    )?;
    let mut allocations = Vec::new();

    for row in rows {
        allocations.push(row?.into_record()?);
    }

    Ok(allocations)
}

fn ready_resource_allocations_for_project_resource_track_in_connection(
    connection: &Connection,
    project_id: &str,
    resource_name: &str,
    track: &str,
) -> Result<Vec<ResourceAllocationRecord>, StateError> {
    let mut statement = connection.prepare(
        "SELECT id, project_id, resource_name, track, allocation_name, generated_name, env_json, status, created_at, updated_at
        FROM resource_allocations
        WHERE project_id = ?1
        AND resource_name = ?2
        AND track = ?3
        AND status = ?4
        ORDER BY allocation_name",
    )?;
    let rows = statement.query_map(
        params![
            project_id,
            resource_name,
            track,
            ResourceAllocationStatus::Ready.as_str()
        ],
        resource_allocation_from_row,
    )?;
    let mut allocations = Vec::new();

    for row in rows {
        allocations.push(row?.into_record()?);
    }

    Ok(allocations)
}

fn resource_allocation_by_key(
    connection: &Connection,
    project_id: &str,
    resource_name: &str,
    allocation_name: &str,
) -> Result<ResourceAllocationRecord, StateError> {
    let row = connection
        .query_row(
            "SELECT id, project_id, resource_name, track, allocation_name, generated_name, env_json, status, created_at, updated_at
            FROM resource_allocations
            WHERE project_id = ?1
            AND resource_name = ?2
            AND allocation_name = ?3",
            params![project_id, resource_name, allocation_name],
            resource_allocation_from_row,
        )
        .optional()?;

    row.ok_or_else(|| StateError::ResourceAllocationNotFound {
        project_id: project_id.to_string(),
        resource: resource_name.to_string(),
        allocation: allocation_name.to_string(),
    })?
    .into_record()
}

fn ready_allocation_env_contexts(
    connection: &Connection,
    project_id: &str,
    resource_name: &str,
    track: &str,
) -> Result<BTreeMap<String, ProjectEnvAllocationContext>, StateError> {
    let allocations = ready_resource_allocations_for_project_resource_track_in_connection(
        connection,
        project_id,
        resource_name,
        track,
    )?;
    let mut contexts = BTreeMap::new();

    for allocation in allocations {
        contexts.insert(
            allocation.allocation_name,
            ProjectEnvAllocationContext {
                generated_name: allocation.generated_name,
                values: allocation.env,
            },
        );
    }

    Ok(contexts)
}

fn project_env_observed_warnings_in_connection(
    connection: &Connection,
    project_id: &str,
) -> Result<Vec<ProjectEnvObservedWarningRecord>, StateError> {
    let mut statement = connection.prepare(
        "SELECT project_id, warning_kind, message, observed_at
        FROM project_env_observed_warnings
        WHERE project_id = ?1
        ORDER BY warning_kind, message",
    )?;
    let rows = statement.query_map(params![project_id], project_env_observed_warning_from_row)?;
    let mut warnings = Vec::new();

    for row in rows {
        warnings.push(row?.into_record()?);
    }

    Ok(warnings)
}

fn validate_project_env_observed_warnings(
    warnings: &[ProjectEnvObservedWarningInput],
) -> Result<(), StateError> {
    let mut identities = BTreeSet::new();

    for warning in warnings {
        validate_project_env_observed_warning_component("kind", &warning.kind)?;
        validate_project_env_observed_warning_component("message", &warning.message)?;
        if !identities.insert((warning.kind.as_str(), warning.message.as_str())) {
            return Err(StateError::InvalidProjectEnvObservedWarning {
                kind: "warning",
                value: format!("{}:{}", warning.kind, warning.message),
            });
        }
    }

    Ok(())
}

fn validate_link_project_input(input: &LinkProjectInput) -> Result<(), StateError> {
    validate_project_path("path", &input.path)?;
    validate_original_project_path(&input.original_path)?;
    validate_project_path("config path", &input.config_path)?;
    validate_project_hostname(&input.primary_hostname)?;
    for hostname in &input.additional_hostnames {
        validate_project_hostname(hostname)?;
    }
    if let Some(track) = &input.desired_php_track
        && track.trim().is_empty()
    {
        return Err(StateError::InvalidProjectTrack {
            track: track.clone(),
        });
    }

    Ok(())
}

fn validate_original_project_path(path: &Utf8Path) -> Result<(), StateError> {
    if !path.is_absolute() {
        return Err(StateError::InvalidProjectPath {
            kind: "original path",
            path: path.to_path_buf(),
            reason: "path must be absolute",
        });
    }

    Ok(())
}

fn validate_project_path(kind: &'static str, path: &Utf8Path) -> Result<(), StateError> {
    if !path.is_absolute() {
        return Err(StateError::InvalidProjectPath {
            kind,
            path: path.to_path_buf(),
            reason: "path must be absolute",
        });
    }

    if path
        .components()
        .any(|component| matches!(component, Utf8Component::CurDir | Utf8Component::ParentDir))
    {
        return Err(StateError::InvalidProjectPath {
            kind,
            path: path.to_path_buf(),
            reason: "path must be canonical",
        });
    }

    Ok(())
}

fn validate_project_hostname(hostname: &str) -> Result<(), StateError> {
    if hostname.is_empty() {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname must not be empty",
        });
    }
    if hostname != hostname.to_ascii_lowercase() {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname must be normalized lowercase",
        });
    }
    if hostname == RESERVED_HOSTNAME {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "`pv.test` is reserved",
        });
    }
    if !hostname.ends_with(".test") {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname must end in `.test`",
        });
    }
    if hostname.len() > MAX_HOSTNAME_LENGTH {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname must be at most 253 bytes",
        });
    }

    for label in hostname.split('.') {
        validate_project_hostname_label(hostname, label)?;
    }

    Ok(())
}

fn validate_project_hostname_label(hostname: &str, label: &str) -> Result<(), StateError> {
    if label.is_empty() {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname labels must not be empty",
        });
    }
    if label.len() > MAX_DNS_LABEL_LENGTH {
        return Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname labels must be at most 63 bytes",
        });
    }

    let is_valid = label
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-');

    if is_valid {
        Ok(())
    } else {
        Err(StateError::InvalidProjectHostname {
            hostname: hostname.to_string(),
            reason: "hostname labels must contain only lowercase letters, numbers, or interior hyphens",
        })
    }
}

fn project_matches_input(project: &ProjectRecord, input: &LinkProjectInput) -> bool {
    project.path == input.path
        && project.original_path == input.original_path
        && project.primary_hostname == input.primary_hostname
        && project.config_path == input.config_path
        && project.desired_php_track == input.desired_php_track
        && sorted_hostnames(&project.additional_hostnames)
            == sorted_hostnames(&input.additional_hostnames)
}

fn sorted_hostnames(hostnames: &[String]) -> Vec<String> {
    let mut hostnames = hostnames.to_vec();
    hostnames.sort();

    hostnames
}

fn insert_project_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
    input: &LinkProjectInput,
) -> Result<(), StateError> {
    let created_at = timestamp()?;
    transaction.execute(
        "INSERT INTO projects (id, path, original_path, primary_hostname, config_path, desired_php_track, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            project_id,
            input.path.as_str(),
            input.original_path.as_str(),
            input.primary_hostname.as_str(),
            input.config_path.as_str(),
            input.desired_php_track.as_deref(),
            created_at,
        ],
    )?;

    Ok(())
}

fn update_project_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
    input: &LinkProjectInput,
) -> Result<(), StateError> {
    let updated_at = timestamp()?;
    transaction.execute(
        "UPDATE projects
        SET primary_hostname = ?1,
            original_path = ?2,
            config_path = ?3,
            desired_php_track = ?4,
            updated_at = ?5
        WHERE id = ?6",
        params![
            input.primary_hostname.as_str(),
            input.original_path.as_str(),
            input.config_path.as_str(),
            input.desired_php_track.as_deref(),
            updated_at,
            project_id,
        ],
    )?;

    Ok(())
}

fn delete_same_project_additional_hostname_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
    hostname: &str,
) -> Result<(), StateError> {
    transaction.execute(
        "DELETE FROM project_hostnames WHERE project_id = ?1 AND hostname = ?2 AND is_primary = 0",
        params![project_id, hostname],
    )?;

    Ok(())
}

fn replace_project_hostnames_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
    input: &LinkProjectInput,
) -> Result<(), StateError> {
    let created_at = timestamp()?;
    transaction.execute(
        "INSERT OR IGNORE INTO project_hostnames (hostname, project_id, is_primary, created_at)
        VALUES (?1, ?2, 1, ?3)",
        params![input.primary_hostname.as_str(), project_id, created_at],
    )?;
    transaction.execute(
        "DELETE FROM project_hostnames WHERE project_id = ?1 AND is_primary = 0",
        params![project_id],
    )?;

    for hostname in &input.additional_hostnames {
        transaction.execute(
            "INSERT INTO project_hostnames (hostname, project_id, is_primary, created_at)
            VALUES (?1, ?2, 0, ?3)",
            params![hostname.as_str(), project_id, created_at],
        )?;
    }

    Ok(())
}

fn validate_project_hostnames_in_transaction(
    transaction: &Transaction<'_>,
    project_id: &str,
    input: &LinkProjectInput,
) -> Result<(), StateError> {
    validate_project_hostname_set(
        transaction,
        project_id,
        &input.primary_hostname,
        &input.additional_hostnames,
    )
}

fn validate_project_hostname_set(
    connection: &Connection,
    project_id: &str,
    primary_hostname: &str,
    additional_hostnames: &[String],
) -> Result<(), StateError> {
    let mut hostnames = BTreeMap::new();
    for hostname in
        std::iter::once(primary_hostname).chain(additional_hostnames.iter().map(String::as_str))
    {
        validate_project_hostname(hostname)?;
        if hostnames.insert(hostname, ()).is_some() {
            return Err(StateError::DuplicateProjectHostname {
                hostname: hostname.to_string(),
            });
        }

        if let Some(owner) = project_owner_for_hostname(connection, hostname)?
            && owner != project_id
        {
            return Err(StateError::ProjectHostnameCollision {
                hostname: hostname.to_string(),
                project_id: owner,
            });
        }
    }

    Ok(())
}

fn project_owner_for_hostname(
    connection: &Connection,
    hostname: &str,
) -> Result<Option<String>, StateError> {
    Ok(connection
        .query_row(
            "SELECT project_id FROM project_hostnames WHERE hostname = ?1",
            params![hostname],
            |row| row.get(0),
        )
        .optional()?)
}

fn generate_project_id(transaction: &Transaction<'_>) -> Result<String, StateError> {
    let mut rng = fastrand::Rng::new();

    for _attempt in 0..PROJECT_ID_ATTEMPTS {
        let id = random_project_id(&mut rng);
        let count = transaction.query_row(
            "SELECT COUNT(*) FROM projects WHERE id = ?1",
            params![id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;

        if count == 0 {
            return Ok(id);
        }
    }

    Err(StateError::ProjectIdExhausted {
        attempts: PROJECT_ID_ATTEMPTS,
    })
}

fn random_project_id(rng: &mut fastrand::Rng) -> String {
    let mut id = String::with_capacity(PROJECT_ID_LENGTH);

    for _index in 0..PROJECT_ID_LENGTH {
        let character_index = rng.usize(..PROJECT_ID_ALPHABET.len());
        id.push(PROJECT_ID_ALPHABET[character_index] as char);
    }

    id
}

fn additional_hostnames_for_project(
    connection: &Connection,
    project_id: &str,
) -> Result<Vec<String>, StateError> {
    let mut statement = connection.prepare(
        "SELECT hostname
        FROM project_hostnames
        WHERE project_id = ?1 AND is_primary = 0
        ORDER BY hostname",
    )?;
    let rows = statement.query_map(params![project_id], |row| row.get::<_, String>(0))?;
    let mut hostnames = Vec::new();

    for row in rows {
        hostnames.push(row?);
    }

    Ok(hostnames)
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
        env_json: row.get(5)?,
        usage_count: row.get(6)?,
        removal_prune: row.get(7)?,
        removal_force: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn project_managed_resource_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProjectManagedResourceRow> {
    Ok(ProjectManagedResourceRow {
        project_id: row.get(0)?,
        resource_name: row.get(1)?,
        track: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn resource_allocation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResourceAllocationRow> {
    Ok(ResourceAllocationRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        resource_name: row.get(2)?,
        track: row.get(3)?,
        allocation_name: row.get(4)?,
        generated_name: row.get(5)?,
        env_json: row.get(6)?,
        status: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn project_env_observed_state_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProjectEnvObservedStateRow> {
    Ok(ProjectEnvObservedStateRow {
        project_id: row.get(0)?,
        status: row.get(1)?,
        message: row.get(2)?,
        observed_at: row.get(3)?,
    })
}

fn project_env_observed_warning_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProjectEnvObservedWarningRow> {
    Ok(ProjectEnvObservedWarningRow {
        project_id: row.get(0)?,
        kind: row.get(1)?,
        message: row.get(2)?,
        observed_at: row.get(3)?,
    })
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: row.get(0)?,
        path: row.get(1)?,
        original_path: row.get(2)?,
        primary_hostname: row.get(3)?,
        config_path: row.get(4)?,
        desired_php_track: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
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

fn validate_concrete_track(track: &str) -> Result<(), StateError> {
    validate_managed_resource_identity("track", track)?;
    if track == RESERVED_TRACK_NAME {
        return Err(StateError::ReservedConcreteTrack {
            track: track.to_string(),
        });
    }

    Ok(())
}

fn validate_resource_allocation_name(value: &str) -> Result<(), StateError> {
    let mut bytes = value.bytes();
    let Some(first_byte) = bytes.next() else {
        return Err(StateError::InvalidResourceAllocationIdentity {
            kind: "allocation",
            value: value.to_string(),
        });
    };
    let is_valid = first_byte.is_ascii_lowercase()
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });

    if is_valid {
        Ok(())
    } else {
        Err(StateError::InvalidResourceAllocationIdentity {
            kind: "allocation",
            value: value.to_string(),
        })
    }
}

fn validate_resource_allocation_identity(
    kind: &'static str,
    value: &str,
) -> Result<(), StateError> {
    let is_valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));

    if is_valid {
        Ok(())
    } else {
        Err(StateError::InvalidResourceAllocationIdentity {
            kind,
            value: value.to_string(),
        })
    }
}

fn validate_project_env_observed_warning_component(
    kind: &'static str,
    value: &str,
) -> Result<(), StateError> {
    if value.trim().is_empty() {
        return Err(StateError::InvalidProjectEnvObservedWarning {
            kind,
            value: value.to_string(),
        });
    }

    Ok(())
}

fn serialize_env_context(context: &str, env: &EnvContextValues) -> Result<String, StateError> {
    validate_env_context(context, env)?;
    serde_json::to_string(env).map_err(|source| StateError::InvalidEnvJson {
        context: context.to_string(),
        reason: source.to_string(),
    })
}

fn parse_env_context(context: &str, env_json: &str) -> Result<EnvContextValues, StateError> {
    let env = serde_json::from_str::<EnvContextValues>(env_json).map_err(|source| {
        StateError::InvalidEnvJson {
            context: context.to_string(),
            reason: source.to_string(),
        }
    })?;
    validate_env_context(context, &env)?;

    Ok(env)
}

fn validate_env_context(context: &str, env: &EnvContextValues) -> Result<(), StateError> {
    for key in env.keys() {
        if key.is_empty() {
            return Err(StateError::InvalidEnvContext {
                context: context.to_string(),
                reason: "env keys must not be empty".to_string(),
            });
        }
    }

    Ok(())
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

fn next_resource_allocation_id(transaction: &Transaction<'_>) -> rusqlite::Result<String> {
    let next_number = transaction.query_row(
        "SELECT COALESCE(MAX(CAST(SUBSTR(id, 12) AS INTEGER)), 0) + 1 FROM resource_allocations WHERE id GLOB 'allocation_[0-9]*'",
        [],
        |row| row.get::<_, i64>(0),
    )?;

    Ok(format!("allocation_{next_number:06}"))
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
