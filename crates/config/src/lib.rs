use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use resources::{ResourceCapability, ResourceKind, TrackSelector, registry};
use thiserror::Error;
use yaml_serde::{Mapping, Number, Value};

const PREFERRED_CONFIG_FILE: &str = "pv.yml";
const ALTERNATE_CONFIG_FILE: &str = "pv.yaml";
const RESERVED_HOSTNAME: &str = "pv.test";
const MAX_DNS_LABEL_LENGTH: usize = 63;
const MAX_HOSTNAME_LENGTH: usize = 253;
const ALLOWED_ENV_PLACEHOLDERS: &[&str] = &[
    "access_key",
    "bucket",
    "dashboard_url",
    "database",
    "endpoint",
    "host",
    "password",
    "port",
    "prefix",
    "project_url",
    "secret_key",
    "smtp_host",
    "smtp_port",
    "url",
    "username",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectConfig {
    pub php: Option<String>,
    pub document_root: Option<Utf8PathBuf>,
    pub hostnames: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub resources: BTreeMap<String, ResourceConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigFile {
    pub path: Utf8PathBuf,
    pub exists: bool,
    pub config: ProjectConfig,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceConfig {
    pub track: Option<String>,
    pub env: BTreeMap<String, String>,
    pub allocations: BTreeMap<String, AllocationConfig>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AllocationConfig {
    pub env: BTreeMap<String, String>,
}

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
}

impl ProjectConfig {
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        if source.trim().is_empty() {
            return Ok(Self::default());
        }

        let mut value = yaml_serde::from_str::<Value>(source)
            .map_err(|source| ConfigError::Parse { source })?;
        value
            .apply_merge()
            .map_err(|source| ConfigError::Parse { source })?;

        Self::from_value(value)
    }

    fn from_value(value: Value) -> Result<Self, ConfigError> {
        match value {
            Value::Null => Ok(Self::default()),
            Value::Mapping(mapping) => parse_project_mapping(mapping),
            value => Err(ConfigError::RootMustBeMapping {
                found: value_type(&value),
            }),
        }
    }
}

impl ProjectConfigFile {
    pub fn read_from_root(project_root: &Utf8Path) -> Result<Self, ConfigError> {
        let canonical_root = canonicalize_utf8(project_root)?;
        if !is_directory(&canonical_root)? {
            return Err(ConfigError::ProjectRootNotDirectory {
                path: canonical_root,
            });
        }

        let preferred = canonical_root.join(PREFERRED_CONFIG_FILE);
        let alternate = canonical_root.join(ALTERNATE_CONFIG_FILE);
        let preferred_exists = path_present(&preferred)?;
        let alternate_exists = path_present(&alternate)?;

        if preferred_exists && alternate_exists {
            return Err(ConfigError::ConfigFileConflict {
                preferred,
                alternate,
            });
        }

        if preferred_exists || alternate_exists {
            let path = if preferred_exists {
                preferred
            } else {
                alternate
            };
            validate_config_path(&canonical_root, &path)?;
            let source = read_to_string(&path)?;
            let config = ProjectConfig::parse(&source)?;
            let config = validate_project_paths(&canonical_root, config)?;

            return Ok(Self {
                path,
                exists: true,
                config,
            });
        }

        Ok(Self {
            path: preferred,
            exists: false,
            config: ProjectConfig::default(),
        })
    }
}

pub fn normalize_primary_hostname(input: &str) -> Result<String, ConfigError> {
    normalize_hostname(input, true)
}

pub fn normalize_additional_hostname(input: &str) -> Result<String, ConfigError> {
    normalize_hostname(input, false)
}

pub fn hostname_from_project_path(path: &Utf8Path) -> Result<String, ConfigError> {
    let Some(file_name) = path.file_name() else {
        return Err(ConfigError::InvalidHostname {
            hostname: path.to_string(),
            reason: "Project path has no directory name",
        });
    };
    let mut slug = String::new();
    let mut previous_was_hyphen = false;

    for character in file_name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_was_hyphen = false;
        } else if !previous_was_hyphen && !slug.is_empty() {
            slug.push('-');
            previous_was_hyphen = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        return Err(ConfigError::InvalidHostname {
            hostname: file_name.to_string(),
            reason: "Project directory name cannot produce a DNS label",
        });
    }

    normalize_primary_hostname(&slug)
}

fn parse_project_mapping(mapping: Mapping) -> Result<ProjectConfig, ConfigError> {
    let mut config = ProjectConfig::default();

    for (key, value) in mapping {
        let key = string_key(key)?;
        match key.as_str() {
            "php" => {
                config.php = Some(php_track(&value)?);
            }
            "document_root" => {
                let document_root = non_empty_string("document_root", &value)?;
                config.document_root = Some(Utf8PathBuf::from(document_root));
            }
            "hostnames" => {
                config.hostnames = parse_hostnames(&value)?;
            }
            "env" => {
                config.env = parse_env_mapping("env", &value)?;
            }
            resource => {
                let Some(resource_name) = project_config_resource_name(resource) else {
                    return Err(ConfigError::UnknownTopLevelKey { key: key.clone() });
                };
                let resource_config = parse_resource_config(resource_name, &value)?;
                if config
                    .resources
                    .insert(resource_name.to_string(), resource_config)
                    .is_some()
                {
                    return Err(ConfigError::DuplicateResource {
                        resource: resource_name.to_string(),
                    });
                }
            }
        }
    }

    Ok(config)
}

fn parse_resource_config(
    resource: &'static str,
    value: &Value,
) -> Result<ResourceConfig, ConfigError> {
    let mapping = match value {
        Value::Null => return Ok(ResourceConfig::default()),
        Value::Mapping(mapping) => mapping,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: resource.to_string(),
                expected: "a mapping",
                found: value_type(value),
            });
        }
    };
    let mut config = ResourceConfig::default();

    for (key, value) in mapping {
        let key = string_key_ref(key)?;
        match key.as_str() {
            "version" => {
                config.track = Some(resource_track(resource, value)?);
            }
            "env" => {
                config.env = parse_env_mapping(&format!("{resource}.env"), value)?;
            }
            "allocations" => {
                if !resource_supports_allocations(resource) {
                    return Err(ConfigError::UnsupportedResourceAllocations {
                        resource: resource.to_string(),
                    });
                }
                config.allocations = parse_allocations(resource, value)?;
            }
            _ => {
                return Err(ConfigError::UnknownResourceKey {
                    resource: resource.to_string(),
                    key,
                });
            }
        }
    }

    Ok(config)
}

fn parse_allocations(
    resource: &str,
    value: &Value,
) -> Result<BTreeMap<String, AllocationConfig>, ConfigError> {
    let mapping = match value {
        Value::Null => return Ok(BTreeMap::new()),
        Value::Mapping(mapping) => mapping,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: format!("{resource}.allocations"),
                expected: "a mapping",
                found: value_type(value),
            });
        }
    };
    let mut allocations = BTreeMap::new();
    let mut normalized_allocations = BTreeMap::new();

    for (key, value) in mapping {
        let allocation = string_key_ref(key)?;
        validate_allocation_name(&allocation)?;
        let normalized = normalized_allocation_name(resource, &allocation);
        if normalized_allocations
            .insert(normalized.clone(), allocation.clone())
            .is_some()
        {
            return Err(ConfigError::DuplicateNormalizedAllocation {
                resource: resource.to_string(),
                allocation,
                normalized,
            });
        }

        let config = parse_allocation_config(resource, &allocation, value)?;
        allocations.insert(allocation, config);
    }

    Ok(allocations)
}

fn parse_allocation_config(
    resource: &str,
    allocation: &str,
    value: &Value,
) -> Result<AllocationConfig, ConfigError> {
    let mapping = match value {
        Value::Null => return Ok(AllocationConfig::default()),
        Value::Mapping(mapping) => mapping,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: format!("{resource}.allocations.{allocation}"),
                expected: "a mapping",
                found: value_type(value),
            });
        }
    };
    let mut config = AllocationConfig::default();

    for (key, value) in mapping {
        let key = string_key_ref(key)?;
        match key.as_str() {
            "env" => {
                config.env =
                    parse_env_mapping(&format!("{resource}.allocations.{allocation}.env"), value)?;
            }
            _ => {
                return Err(ConfigError::UnknownAllocationKey {
                    resource: resource.to_string(),
                    allocation: allocation.to_string(),
                    key,
                });
            }
        }
    }

    Ok(config)
}

fn parse_hostnames(value: &Value) -> Result<Vec<String>, ConfigError> {
    let sequence = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Sequence(sequence) => sequence,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: "hostnames".to_string(),
                expected: "a sequence",
                found: value_type(value),
            });
        }
    };
    let mut hostnames = Vec::new();

    for value in sequence {
        let hostname = non_empty_string("hostnames", value)?;
        let hostname = normalize_additional_hostname(&hostname)?;
        if hostnames.contains(&hostname) {
            return Err(ConfigError::DuplicateHostname { hostname });
        }

        hostnames.push(hostname);
    }

    Ok(hostnames)
}

fn parse_env_mapping(field: &str, value: &Value) -> Result<BTreeMap<String, String>, ConfigError> {
    let mapping = match value {
        Value::Null => return Ok(BTreeMap::new()),
        Value::Mapping(mapping) => mapping,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: field.to_string(),
                expected: "a mapping",
                found: value_type(value),
            });
        }
    };
    let mut env = BTreeMap::new();

    for (key, value) in mapping {
        let key = string_key_ref(key)?;
        validate_env_key(&key)?;
        let value_field = format!("{field}.{key}");
        let value = env_scalar_to_string(&value_field, value)?;
        validate_env_placeholders(&value_field, &value)?;
        env.insert(key, value);
    }

    Ok(env)
}

fn validate_project_paths(
    project_root: &Utf8Path,
    config: ProjectConfig,
) -> Result<ProjectConfig, ConfigError> {
    let Some(document_root) = &config.document_root else {
        return Ok(config);
    };

    if document_root.is_absolute() {
        return Err(ConfigError::AbsoluteDocumentRoot {
            document_root: document_root.clone(),
        });
    }

    let absolute_document_root = project_root.join(document_root);
    if !absolute_document_root.exists() {
        return Err(ConfigError::DocumentRootNotDirectory {
            document_root: document_root.clone(),
        });
    }

    let canonical_document_root = canonicalize_utf8(&absolute_document_root)?;
    if !canonical_document_root.starts_with(project_root) {
        return Err(ConfigError::DocumentRootEscapesProject {
            document_root: document_root.clone(),
        });
    }

    if !is_directory(&canonical_document_root)? {
        return Err(ConfigError::DocumentRootNotDirectory {
            document_root: document_root.clone(),
        });
    }

    Ok(config)
}

fn normalize_hostname(input: &str, allow_bare_label: bool) -> Result<String, ConfigError> {
    let original = input.trim();
    if original.is_empty() {
        return Err(ConfigError::InvalidHostname {
            hostname: input.to_string(),
            reason: "hostname must not be empty",
        });
    }

    let trimmed = original.strip_suffix('.').unwrap_or(original);
    let mut hostname = trimmed.to_ascii_lowercase();
    if allow_bare_label && !hostname.contains('.') {
        hostname.push_str(".test");
    }

    validate_hostname(&hostname, input, allow_bare_label)?;

    Ok(hostname)
}

fn validate_hostname(
    hostname: &str,
    original: &str,
    allow_bare_label: bool,
) -> Result<(), ConfigError> {
    if hostname == RESERVED_HOSTNAME {
        return Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: "`pv.test` is reserved",
        });
    }

    if hostname.contains('*') {
        return Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: "wildcard hostnames are not supported",
        });
    }

    if !hostname.ends_with(".test") {
        return Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: if allow_bare_label {
                "hostname must be a bare label or end in `.test`"
            } else {
                "additional hostnames must be full `.test` hostnames"
            },
        });
    }

    if hostname.len() > MAX_HOSTNAME_LENGTH {
        return Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: "hostname must be at most 253 bytes",
        });
    }

    for label in hostname.split('.') {
        validate_dns_label(label, original)?;
    }

    Ok(())
}

fn validate_dns_label(label: &str, original: &str) -> Result<(), ConfigError> {
    if label.is_empty() {
        return Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: "hostname labels must not be empty",
        });
    }

    if label.len() > MAX_DNS_LABEL_LENGTH {
        return Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: "hostname labels must be at most 63 bytes",
        });
    }

    let is_valid = label
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-');

    if is_valid {
        Ok(())
    } else {
        Err(ConfigError::InvalidHostname {
            hostname: original.to_string(),
            reason: "hostname labels must contain only letters, numbers, or interior hyphens",
        })
    }
}

fn non_empty_string(field: &str, value: &Value) -> Result<String, ConfigError> {
    let scalar = string_scalar(field, value)?;
    if scalar.trim().is_empty() {
        return Err(ConfigError::EmptyField {
            field: field.to_string(),
        });
    }

    Ok(scalar)
}

fn non_empty_string_or_number(field: &str, value: &Value) -> Result<String, ConfigError> {
    let scalar = string_or_number_to_string(field, value)?;
    if scalar.trim().is_empty() {
        return Err(ConfigError::EmptyField {
            field: field.to_string(),
        });
    }

    Ok(scalar)
}

fn php_track(value: &Value) -> Result<String, ConfigError> {
    let track = non_empty_string_or_number("php", value)?;
    TrackSelector::parse(track.clone()).map_err(|source| ConfigError::InvalidPhpTrack {
        track: track.clone(),
        reason: source.to_string(),
    })?;

    Ok(track)
}

fn resource_track(resource: &str, value: &Value) -> Result<String, ConfigError> {
    let field = format!("{resource}.version");
    let track = non_empty_string_or_number(&field, value)?;
    TrackSelector::parse(track.clone()).map_err(|source| ConfigError::InvalidResourceTrack {
        resource: resource.to_string(),
        track: track.clone(),
        reason: source.to_string(),
    })?;

    Ok(track)
}

fn string_scalar(field: &str, value: &Value) -> Result<String, ConfigError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        value => Err(ConfigError::InvalidFieldType {
            field: field.to_string(),
            expected: "a string",
            found: value_type(value),
        }),
    }
}

fn string_or_number_to_string(field: &str, value: &Value) -> Result<String, ConfigError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(number) => Ok(number_to_string(number)),
        value => Err(ConfigError::InvalidFieldType {
            field: field.to_string(),
            expected: "a string or number",
            found: value_type(value),
        }),
    }
}

fn env_scalar_to_string(field: &str, value: &Value) -> Result<String, ConfigError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(number) => Ok(number_to_string(number)),
        Value::Bool(value) => Ok(value.to_string()),
        value => Err(ConfigError::InvalidFieldType {
            field: field.to_string(),
            expected: "a string, number, or boolean",
            found: value_type(value),
        }),
    }
}

fn number_to_string(number: &Number) -> String {
    format!("{number}")
}

fn string_key(value: Value) -> Result<String, ConfigError> {
    match value {
        Value::String(key) => Ok(key),
        value => Err(ConfigError::InvalidFieldType {
            field: "mapping key".to_string(),
            expected: "a string",
            found: value_type(&value),
        }),
    }
}

fn string_key_ref(value: &Value) -> Result<String, ConfigError> {
    match value {
        Value::String(key) => Ok(key.clone()),
        value => Err(ConfigError::InvalidFieldType {
            field: "mapping key".to_string(),
            expected: "a string",
            found: value_type(value),
        }),
    }
}

fn project_config_resource_name(name: &str) -> Option<&'static str> {
    registry::resolve(name)
        .ok()
        .filter(|descriptor| descriptor.kind() == ResourceKind::BackingService)
        .map(|descriptor| descriptor.name())
}

fn resource_supports_allocations(resource: &str) -> bool {
    matches!(
        registry::resolve_canonical(resource),
        Ok(descriptor) if descriptor.capabilities().contains(&ResourceCapability::Allocation)
    )
}

fn normalized_allocation_name(resource: &str, allocation: &str) -> String {
    match resource {
        "mysql" | "postgres" => allocation.replace('-', "_"),
        "redis" | "rustfs" => allocation.replace('_', "-"),
        _ => allocation.to_string(),
    }
}

fn validate_env_placeholders(field: &str, value: &str) -> Result<(), ConfigError> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < characters.len() {
        if characters[index] != '$' {
            index += 1;
            continue;
        }

        if characters.get(index + 1) == Some(&'$') {
            index += 2;
            continue;
        }

        if characters.get(index + 1) != Some(&'{') {
            return Err(ConfigError::InvalidEnvPlaceholder {
                field: field.to_string(),
                placeholder: "$".to_string(),
                reason: "literal dollar signs must be escaped as `$$`",
            });
        }

        let Some(end_index) = characters[index + 2..]
            .iter()
            .position(|character| *character == '}')
            .map(|offset| index + 2 + offset)
        else {
            return Err(ConfigError::InvalidEnvPlaceholder {
                field: field.to_string(),
                placeholder: characters[index..].iter().collect(),
                reason: "placeholder must end with `}`",
            });
        };
        let placeholder = characters[index + 2..end_index].iter().collect::<String>();
        validate_placeholder_name(field, &placeholder)?;
        if !ALLOWED_ENV_PLACEHOLDERS.contains(&placeholder.as_str()) {
            return Err(ConfigError::UnknownEnvPlaceholder {
                field: field.to_string(),
                placeholder,
            });
        }

        index = end_index + 1;
    }

    Ok(())
}

fn validate_placeholder_name(field: &str, placeholder: &str) -> Result<(), ConfigError> {
    let mut bytes = placeholder.bytes();
    let Some(first) = bytes.next() else {
        return Err(ConfigError::InvalidEnvPlaceholder {
            field: field.to_string(),
            placeholder: placeholder.to_string(),
            reason: "placeholder name must not be empty",
        });
    };
    let first_valid = first.is_ascii_lowercase();
    let rest_valid =
        bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');

    if first_valid && rest_valid {
        Ok(())
    } else {
        Err(ConfigError::InvalidEnvPlaceholder {
            field: field.to_string(),
            placeholder: placeholder.to_string(),
            reason: "placeholder names must use lowercase snake_case",
        })
    }
}

fn validate_env_key(key: &str) -> Result<(), ConfigError> {
    let mut bytes = key.bytes();
    let Some(first) = bytes.next() else {
        return Err(ConfigError::InvalidEnvKey {
            key: key.to_string(),
        });
    };

    let first_valid = first.is_ascii_uppercase() || first == b'_';
    let rest_valid =
        bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');

    if first_valid && rest_valid {
        Ok(())
    } else {
        Err(ConfigError::InvalidEnvKey {
            key: key.to_string(),
        })
    }
}

fn validate_allocation_name(allocation: &str) -> Result<(), ConfigError> {
    let mut bytes = allocation.bytes();
    let Some(first) = bytes.next() else {
        return Err(ConfigError::InvalidAllocationName {
            allocation: allocation.to_string(),
        });
    };

    let first_valid = first.is_ascii_lowercase();
    let rest_valid = bytes.all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    });

    if first_valid && rest_valid {
        Ok(())
    } else {
        Err(ConfigError::InvalidAllocationName {
            allocation: allocation.to_string(),
        })
    }
}

fn validate_config_path(
    project_root: &Utf8Path,
    config_path: &Utf8Path,
) -> Result<(), ConfigError> {
    let canonical_config_path = canonicalize_utf8(config_path)?;
    if canonical_config_path.starts_with(project_root) {
        Ok(())
    } else {
        Err(ConfigError::ConfigPathEscapesRoot {
            path: config_path.to_path_buf(),
        })
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "sequence",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged value",
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config discovery owns symlink-aware config file probing"
)]
fn path_present(path: &Utf8Path) -> Result<bool, ConfigError> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Filesystem {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config parser owns canonical path validation"
)]
fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, ConfigError> {
    let path = std::fs::canonicalize(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;

    Utf8PathBuf::from_path_buf(path).map_err(|path| ConfigError::NonUtf8Path { path })
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config parser owns config file reads"
)]
fn read_to_string(path: &Utf8Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config parser owns project root and document root validation"
)]
fn is_directory(path: &Utf8Path) -> Result<bool, ConfigError> {
    let metadata = std::fs::metadata(path).map_err(|source| ConfigError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(metadata.is_dir())
}
