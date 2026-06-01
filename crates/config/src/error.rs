use std::io;

use camino::Utf8PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Project path is not valid UTF-8: {path:?}")]
    NonUtf8Path { path: std::path::PathBuf },

    #[error("Project config file conflict: both {preferred} and {alternate} exist")]
    ConfigFileConflict {
        preferred: Utf8PathBuf,
        alternate: Utf8PathBuf,
    },

    #[error("filesystem error at {path}: {source}")]
    Filesystem {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Project root must be an existing directory: {path}")]
    ProjectRootNotDirectory { path: Utf8PathBuf },

    #[error("Project config symlink escapes the Project root: {path}")]
    ConfigPathEscapesRoot { path: Utf8PathBuf },

    #[error("Project config YAML parse error: {source}")]
    Parse {
        #[source]
        source: yaml_serde::Error,
    },

    #[error("Project config root must be a mapping, found {found}")]
    RootMustBeMapping { found: &'static str },

    #[error("unknown Project config key `{key}`")]
    UnknownTopLevelKey { key: String },

    #[error("unknown Project config key `{key}` under resource `{resource}`")]
    UnknownResourceKey { resource: String, key: String },

    #[error("unknown Project config key `{key}` under allocation `{resource}.{allocation}`")]
    UnknownAllocationKey {
        resource: String,
        allocation: String,
        key: String,
    },

    #[error("Project config field `{field}` must be {expected}, found {found}")]
    InvalidFieldType {
        field: String,
        expected: &'static str,
        found: &'static str,
    },

    #[error("Project config field `{field}` must not be empty")]
    EmptyField { field: String },

    #[error("invalid Project config PHP track `{track}`: {reason}")]
    InvalidPhpTrack { track: String, reason: String },

    #[error("invalid Project config resource `{resource}` version `{track}`: {reason}")]
    InvalidResourceTrack {
        resource: String,
        track: String,
        reason: String,
    },

    #[error("invalid Project hostname `{hostname}`: {reason}")]
    InvalidHostname {
        hostname: String,
        reason: &'static str,
    },

    #[error("duplicate Project config hostname `{hostname}`")]
    DuplicateHostname { hostname: String },

    #[error("Project config document_root must be relative to the Project root: {document_root}")]
    AbsoluteDocumentRoot { document_root: Utf8PathBuf },

    #[error("Project config document_root escapes the Project root: {document_root}")]
    DocumentRootEscapesProject { document_root: Utf8PathBuf },

    #[error("Project config document_root must be an existing directory: {document_root}")]
    DocumentRootNotDirectory { document_root: Utf8PathBuf },

    #[error("invalid Project config env key `{key}`")]
    InvalidEnvKey { key: String },

    #[error("invalid Project config allocation name `{allocation}`")]
    InvalidAllocationName { allocation: String },

    #[error("duplicate Project config resource `{resource}`")]
    DuplicateResource { resource: String },

    #[error("Project config resource `{resource}` does not support allocations")]
    UnsupportedResourceAllocations { resource: String },

    #[error(
        "duplicate Project config allocation `{allocation}` for resource `{resource}` after normalizing to `{normalized}`"
    )]
    DuplicateNormalizedAllocation {
        resource: String,
        allocation: String,
        normalized: String,
    },

    #[error("invalid Project config env placeholder `{placeholder}` in `{field}`: {reason}")]
    InvalidEnvPlaceholder {
        field: String,
        placeholder: String,
        reason: &'static str,
    },

    #[error("unknown Project config env placeholder `{placeholder}` in `{field}`")]
    UnknownEnvPlaceholder { field: String, placeholder: String },

    #[error(
        "failed to load env placeholder contract for Project config resource `{resource}`: {reason}"
    )]
    EnvPlaceholderContract { resource: String, reason: String },

    #[error("missing Project env context for resource `{resource}`")]
    MissingResourceEnvContext { resource: String },

    #[error("missing Project env context for allocation `{resource}.{allocation}`")]
    MissingAllocationEnvContext {
        resource: String,
        allocation: String,
    },

    #[error("missing Project env context value `{placeholder}` for `{field}`")]
    MissingEnvContext { field: String, placeholder: String },

    #[error(
        "duplicate rendered Project env key `{key}` from same-depth mappings `{first}` and `{second}`"
    )]
    DuplicateRenderedEnvKey {
        key: String,
        first: String,
        second: String,
    },

    #[error("malformed PV-managed .env block: {reason}")]
    MalformedManagedEnvBlock { reason: &'static str },
}
