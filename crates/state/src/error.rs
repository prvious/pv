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
