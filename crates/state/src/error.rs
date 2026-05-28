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

    #[error("Project `{target}` was not found")]
    ProjectNotFound { target: String },

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
