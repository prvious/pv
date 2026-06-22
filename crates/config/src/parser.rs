use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use resources::{
    ResourceCapability, ResourceKind, TrackSelector, allocation_env_placeholders, registry,
    resource_env_placeholders,
};
use yaml_serde::{Mapping, Number, Value};

use crate::hostname::normalize_additional_hostname;
use crate::{AllocationConfig, ConfigError, PhpConfig, ProjectConfig, ResourceConfig};

const PROJECT_ENV_PLACEHOLDERS: &[&str] = &["project_url"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvPlaceholderScope<'a> {
    Project,
    Resource { resource: &'a str },
    Allocation { resource: &'a str },
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

fn parse_project_mapping(mapping: Mapping) -> Result<ProjectConfig, ConfigError> {
    let mut config = ProjectConfig::default();

    for (key, value) in mapping {
        let key = string_key(key)?;
        match key.as_str() {
            "php" => {
                config.php = Some(php_config(&value)?);
            }
            "document_root" => {
                let document_root = non_empty_string("document_root", &value)?;
                config.document_root = Some(Utf8PathBuf::from(document_root));
            }
            "hostnames" => {
                config.hostnames = parse_hostnames(&value)?;
            }
            "env" => {
                config.env = parse_env_mapping("env", EnvPlaceholderScope::Project, &value)?;
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
                config.env = parse_env_mapping(
                    &format!("{resource}.env"),
                    EnvPlaceholderScope::Resource { resource },
                    value,
                )?;
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
                config.env = parse_env_mapping(
                    &format!("{resource}.allocations.{allocation}.env"),
                    EnvPlaceholderScope::Allocation { resource },
                    value,
                )?;
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

fn parse_env_mapping(
    field: &str,
    scope: EnvPlaceholderScope<'_>,
    value: &Value,
) -> Result<BTreeMap<String, String>, ConfigError> {
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
        validate_env_placeholders(&value_field, scope, &value)?;
        env.insert(key, value);
    }

    Ok(env)
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

fn php_config(value: &Value) -> Result<PhpConfig, ConfigError> {
    match value {
        Value::Mapping(mapping) => php_config_mapping(mapping),
        value => php_track(value).map(PhpConfig::version),
    }
}

fn php_config_mapping(mapping: &Mapping) -> Result<PhpConfig, ConfigError> {
    let mut config = PhpConfig::default();

    for (key, value) in mapping {
        let key = string_key_ref(key)?;
        match key.as_str() {
            "version" => {
                config.version = Some(php_track_field("php.version", value)?);
            }
            "extensions" => {
                config.extensions = php_extensions(value)?;
            }
            _ => {
                return Err(ConfigError::UnknownPhpKey { key });
            }
        }
    }

    Ok(config)
}

fn php_track(value: &Value) -> Result<String, ConfigError> {
    php_track_field("php", value)
}

fn php_track_field(field: &str, value: &Value) -> Result<String, ConfigError> {
    let track = non_empty_string_or_number(field, value)?;
    TrackSelector::parse(track.clone()).map_err(|source| ConfigError::InvalidPhpTrack {
        track: track.clone(),
        reason: source.to_string(),
    })?;

    Ok(track)
}

fn php_extensions(value: &Value) -> Result<Vec<String>, ConfigError> {
    let sequence = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Sequence(sequence) => sequence,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: "php.extensions".to_string(),
                expected: "a sequence",
                found: value_type(value),
            });
        }
    };

    sequence
        .iter()
        .map(|value| non_empty_string("php.extensions", value))
        .collect()
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

fn validate_env_placeholders(
    field: &str,
    scope: EnvPlaceholderScope<'_>,
    value: &str,
) -> Result<(), ConfigError> {
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
        if !scope.allows_placeholder(&placeholder)? {
            return Err(ConfigError::UnknownEnvPlaceholder {
                field: field.to_string(),
                placeholder,
            });
        }

        index = end_index + 1;
    }

    Ok(())
}

impl<'a> EnvPlaceholderScope<'a> {
    fn allows_placeholder(self, placeholder: &str) -> Result<bool, ConfigError> {
        if PROJECT_ENV_PLACEHOLDERS.contains(&placeholder) {
            return Ok(true);
        }

        match self {
            Self::Project => Ok(false),
            Self::Resource { resource } => {
                Ok(resource_placeholders(resource)?.contains(&placeholder))
            }
            Self::Allocation { resource } => Ok(resource_placeholders(resource)?
                .contains(&placeholder)
                || allocation_placeholders(resource)?.contains(&placeholder)),
        }
    }
}

fn resource_placeholders(resource: &str) -> Result<&'static [&'static str], ConfigError> {
    resource_env_placeholders(resource).map_err(|source| ConfigError::EnvPlaceholderContract {
        resource: resource.to_string(),
        reason: source.to_string(),
    })
}

fn allocation_placeholders(resource: &str) -> Result<&'static [&'static str], ConfigError> {
    allocation_env_placeholders(resource).map_err(|source| ConfigError::EnvPlaceholderContract {
        resource: resource.to_string(),
        reason: source.to_string(),
    })
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
