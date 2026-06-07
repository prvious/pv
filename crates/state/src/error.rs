use camino::Utf8PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("home directory could not be determined")]
    MissingHome,

    #[error("home directory is not valid UTF-8: {path:?}")]
    NonUtf8Home { path: std::path::PathBuf },

    #[error("filesystem error at {path}: {source}")]
    Filesystem {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unsafe permissions for {path}: expected {expected:o}, found {actual:o}")]
    UnsafePermissions {
        path: Utf8PathBuf,
        expected: u32,
        actual: u32,
    },

    #[error("unexpected owner for {path}: expected uid {expected}, found uid {actual}")]
    UnexpectedOwner {
        path: Utf8PathBuf,
        expected: u32,
        actual: u32,
    },

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration {version} ({name}) failed: {source}")]
    MigrationFailed {
        version: i64,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    #[error("migration {version} name mismatch: expected {expected}, found {actual}")]
    MigrationNameMismatch {
        version: i64,
        expected: &'static str,
        actual: String,
    },

    #[error("unknown daemon job status `{status}`")]
    UnknownJobStatus { status: String },

    #[error("unknown managed resource desired state `{desired_state}`")]
    UnknownManagedResourceDesiredState { desired_state: String },

    #[error("unknown resource allocation status `{status}`")]
    UnknownResourceAllocationStatus { status: String },

    #[error("unknown Project env observed status `{status}`")]
    UnknownProjectEnvObservedStatus { status: String },

    #[error("unknown runtime observed status `{status}`")]
    UnknownRuntimeObservedStatus { status: String },

    #[error("invalid runtime subject {kind} `{value}`")]
    InvalidRuntimeSubject { kind: &'static str, value: String },

    #[error("Project `{target}` was not found")]
    ProjectNotFound { target: String },

    #[error("invalid Project {kind} `{path}`: {reason}")]
    InvalidProjectPath {
        kind: &'static str,
        path: Utf8PathBuf,
        reason: &'static str,
    },

    #[error("invalid Project hostname `{hostname}`: {reason}")]
    InvalidProjectHostname {
        hostname: String,
        reason: &'static str,
    },

    #[error("invalid Project PHP track `{track}`")]
    InvalidProjectTrack { track: String },

    #[error("Project hostname `{hostname}` is already owned by Project `{project_id}`")]
    ProjectHostnameCollision {
        hostname: String,
        project_id: String,
    },

    #[error("Project hostname `{hostname}` appears more than once for the same Project")]
    DuplicateProjectHostname { hostname: String },

    #[error("could not allocate a unique Project ID after {attempts} attempts")]
    ProjectIdExhausted { attempts: usize },

    #[error("invalid managed resource {kind} `{value}`")]
    InvalidManagedResourceIdentity { kind: &'static str, value: String },

    #[error("reserved concrete track `{track}` must be resolved before state storage")]
    ReservedConcreteTrack { track: String },

    #[error("invalid resource allocation {kind} `{value}`")]
    InvalidResourceAllocationIdentity { kind: &'static str, value: String },

    #[error(
        "Resource allocation generated name `{generated}` for resource `{resource}` track `{track}` is already in use"
    )]
    ResourceAllocationGeneratedNameCollision {
        resource: String,
        track: String,
        generated: String,
    },

    #[error("invalid env JSON for {context}: {reason}")]
    InvalidEnvJson { context: String, reason: String },

    #[error("invalid env context for {context}: {reason}")]
    InvalidEnvContext { context: String, reason: String },

    #[error(
        "Resource allocation `{allocation}` for Project `{project_id}` resource `{resource}` was not found"
    )]
    ResourceAllocationNotFound {
        project_id: String,
        resource: String,
        allocation: String,
    },

    #[error(
        "Resource allocation `{allocation}` for Project `{project_id}` resource `{resource}` track `{track}` is not in desired state"
    )]
    ResourceAllocationNotDesired {
        project_id: String,
        resource: String,
        track: String,
        allocation: String,
    },

    #[error("invalid Project env observed warning {kind} `{value}`")]
    InvalidProjectEnvObservedWarning { kind: &'static str, value: String },

    #[error("daemon job `{id}` was not found")]
    JobNotFound { id: String },

    #[error("invalid port owner `{owner}`: {reason}")]
    InvalidPortOwner { owner: String, reason: &'static str },

    #[error("unknown port owner kind `{owner_kind}`")]
    UnknownPortOwnerKind { owner_kind: String },

    #[error(
        "no available port for {name}; tried preferred {preferred_port} and up to {attempts} candidates in {fallback_start}-{fallback_end}"
    )]
    NoAvailablePort {
        name: String,
        preferred_port: u16,
        fallback_start: u16,
        fallback_end: u16,
        attempts: usize,
    },

    #[error("time formatting failed: {0}")]
    TimeFormat(#[from] time::error::Format),

    #[error("could not allocate a unique migration backup name under {path}")]
    BackupNameExhausted { path: Utf8PathBuf },
}

impl StateError {
    pub(crate) fn filesystem(path: impl Into<Utf8PathBuf>, source: std::io::Error) -> Self {
        Self::Filesystem {
            path: path.into(),
            source,
        }
    }
}
