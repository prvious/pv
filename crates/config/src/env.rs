use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;

use crate::{ConfigError, ProjectConfig, filesystem};

pub const MANAGED_ENV_START_MARKER: &str = "# >>> PV MANAGED";
pub const MANAGED_ENV_END_MARKER: &str = "# <<< PV MANAGED";
const CREATED_PROJECT_ENV_FILE_MODE: u32 = 0o600;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectEnvContext {
    pub primary_hostname: String,
    pub tls_ca_path: String,
    pub tls_cert_path: String,
    pub tls_key_path: String,
    pub resources: BTreeMap<String, ResourceEnvContext>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceEnvContext {
    pub track: String,
    pub values: BTreeMap<String, String>,
    pub allocations: BTreeMap<String, AllocationEnvContext>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AllocationEnvContext {
    pub generated_name: String,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RenderedProjectEnv {
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedEnvBlockTransform {
    pub content: String,
    pub warnings: Vec<ProjectEnvWarning>,
    pub changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectEnvWarning {
    DuplicateExistingKey { key: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RenderedEnvEntry {
    source: String,
    value: String,
}

pub fn render_project_env(
    config: &ProjectConfig,
    context: &ProjectEnvContext,
) -> Result<RenderedProjectEnv, ConfigError> {
    let project_values = project_context_values(context)?;
    let mut values = render_mapping("env", &config.env, &project_values)?
        .into_iter()
        .map(|(key, entry)| (key, entry.value))
        .collect::<BTreeMap<_, _>>();

    let mut resource_values = BTreeMap::new();
    for (resource, resource_config) in &config.resources {
        if resource_config.env.is_empty() {
            continue;
        }

        let resource_context = context.resources.get(resource).ok_or_else(|| {
            ConfigError::MissingResourceEnvContext {
                resource: resource.clone(),
            }
        })?;
        let context_values = resource_context_values(&project_values, resource_context);
        let rendered = render_mapping(
            &format!("{resource}.env"),
            &resource_config.env,
            &context_values,
        )?;
        insert_same_depth_entries(&mut resource_values, rendered)?;
    }
    values.extend(
        resource_values
            .into_iter()
            .map(|(key, entry)| (key, entry.value)),
    );

    let mut allocation_values = BTreeMap::new();
    for (resource, resource_config) in &config.resources {
        for (allocation, allocation_config) in &resource_config.allocations {
            if allocation_config.env.is_empty() {
                continue;
            }

            let resource_context = context.resources.get(resource).ok_or_else(|| {
                ConfigError::MissingResourceEnvContext {
                    resource: resource.clone(),
                }
            })?;
            let allocation_context =
                resource_context
                    .allocations
                    .get(allocation)
                    .ok_or_else(|| ConfigError::MissingAllocationEnvContext {
                        resource: resource.clone(),
                        allocation: allocation.clone(),
                    })?;
            let context_values =
                allocation_context_values(&project_values, resource_context, allocation_context);
            let rendered = render_mapping(
                &format!("{resource}.allocations.{allocation}.env"),
                &allocation_config.env,
                &context_values,
            )?;
            insert_same_depth_entries(&mut allocation_values, rendered)?;
        }
    }
    values.extend(
        allocation_values
            .into_iter()
            .map(|(key, entry)| (key, entry.value)),
    );

    Ok(RenderedProjectEnv { values })
}

pub fn validate_project_env_shape(config: &ProjectConfig) -> Result<(), ConfigError> {
    let mut resource_values = BTreeMap::new();
    for (resource, resource_config) in &config.resources {
        for key in resource_config.env.keys() {
            insert_same_depth_key(&mut resource_values, key, format!("{resource}.env.{key}"))?;
        }
    }

    let mut allocation_values = BTreeMap::new();
    for (resource, resource_config) in &config.resources {
        for (allocation, allocation_config) in &resource_config.allocations {
            for key in allocation_config.env.keys() {
                insert_same_depth_key(
                    &mut allocation_values,
                    key,
                    format!("{resource}.allocations.{allocation}.env.{key}"),
                )?;
            }
        }
    }

    Ok(())
}

pub fn format_project_env(rendered: &RenderedProjectEnv) -> String {
    if rendered.values.is_empty() {
        return String::new();
    }

    let mut lines = rendered
        .values
        .iter()
        .map(|(key, value)| format!("{key}={}", format_env_value(value)))
        .collect::<Vec<_>>()
        .join("\n");
    lines.push('\n');
    lines
}

pub fn format_env_value(value: &str) -> String {
    if is_safe_unquoted_value(value) {
        return value.to_string();
    }

    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '$' => escaped.push_str("\\$"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }

    format!("\"{escaped}\"")
}

pub fn transform_managed_env_block(
    existing_content: Option<&str>,
    rendered: &RenderedProjectEnv,
) -> Result<ManagedEnvBlockTransform, ConfigError> {
    let existing_content = existing_content.unwrap_or_default();
    if rendered.values.is_empty() {
        return Ok(ManagedEnvBlockTransform {
            content: existing_content.to_string(),
            warnings: Vec::new(),
            changed: false,
        });
    }

    let lines = split_env_lines(existing_content);
    let blocks = managed_blocks(&lines)?;
    let warnings = duplicate_existing_key_warnings(&lines, &blocks, rendered);
    let block_lines = managed_block_lines(rendered);

    let transformed_lines = if blocks.is_empty() {
        append_managed_block(lines, block_lines)
    } else if let [(start, end)] = blocks.as_slice() {
        replace_managed_block(lines, *start, *end, block_lines)
    } else {
        fold_managed_blocks(lines, &blocks, block_lines)
    };
    let content = join_env_lines(&transformed_lines);

    Ok(ManagedEnvBlockTransform {
        changed: content != existing_content,
        content,
        warnings,
    })
}

pub fn validate_managed_env_block(existing_content: Option<&str>) -> Result<(), ConfigError> {
    let existing_content = existing_content.unwrap_or_default();
    let lines = split_env_lines(existing_content);
    managed_blocks(&lines)?;

    Ok(())
}

pub fn write_project_env_file(
    path: &Utf8Path,
    rendered: &RenderedProjectEnv,
) -> Result<ManagedEnvBlockTransform, ConfigError> {
    let existing_content = filesystem::read_optional_to_string(path)?;
    let existing_mode = if existing_content.is_some() {
        Some(filesystem::file_mode(path)?)
    } else {
        None
    };
    let transform = transform_managed_env_block(existing_content.as_deref(), rendered)?;

    if transform.changed {
        let mode = existing_mode.unwrap_or(CREATED_PROJECT_ENV_FILE_MODE);
        filesystem::write_string_atomically_with_mode(path, &transform.content, mode)?;
    }

    Ok(transform)
}

fn project_context_values(
    context: &ProjectEnvContext,
) -> Result<BTreeMap<String, String>, ConfigError> {
    if context.primary_hostname.is_empty() {
        return Err(ConfigError::MissingEnvContext {
            field: "project.primary_hostname".to_string(),
            placeholder: "project_url".to_string(),
        });
    }

    let mut values = BTreeMap::new();
    values.insert(
        "project_url".to_string(),
        format!("https://{}", context.primary_hostname),
    );
    if !context.tls_ca_path.is_empty() {
        values.insert("tls_ca".to_string(), context.tls_ca_path.clone());
    }
    if !context.tls_cert_path.is_empty() {
        values.insert("tls_cert".to_string(), context.tls_cert_path.clone());
    }
    if !context.tls_key_path.is_empty() {
        values.insert("tls_key".to_string(), context.tls_key_path.clone());
    }

    Ok(values)
}

fn resource_context_values(
    project_values: &BTreeMap<String, String>,
    resource_context: &ResourceEnvContext,
) -> BTreeMap<String, String> {
    let mut values = resource_context.values.clone();
    values.extend(project_values.clone());
    values
}

fn allocation_context_values(
    project_values: &BTreeMap<String, String>,
    resource_context: &ResourceEnvContext,
    allocation_context: &AllocationEnvContext,
) -> BTreeMap<String, String> {
    let mut values = resource_context.values.clone();
    if !allocation_context.generated_name.is_empty() {
        values.insert(
            "generated_name".to_string(),
            allocation_context.generated_name.clone(),
        );
    }
    values.extend(allocation_context.values.clone());
    values.extend(project_values.clone());
    values
}

fn render_mapping(
    field: &str,
    mapping: &BTreeMap<String, String>,
    context_values: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, RenderedEnvEntry>, ConfigError> {
    let mut rendered = BTreeMap::new();

    for (key, value) in mapping {
        let field = format!("{field}.{key}");
        let value = render_env_value(&field, value, context_values)?;
        rendered.insert(
            key.clone(),
            RenderedEnvEntry {
                source: field,
                value,
            },
        );
    }

    Ok(rendered)
}

fn render_env_value(
    field: &str,
    value: &str,
    context_values: &BTreeMap<String, String>,
) -> Result<String, ConfigError> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut rendered = String::new();
    let mut index = 0;

    while index < characters.len() {
        if characters[index] != '$' {
            rendered.push(characters[index]);
            index += 1;
            continue;
        }

        if characters.get(index + 1) == Some(&'$') {
            rendered.push('$');
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
        let Some(value) = context_values.get(&placeholder) else {
            return Err(ConfigError::MissingEnvContext {
                field: field.to_string(),
                placeholder,
            });
        };
        rendered.push_str(value);
        index = end_index + 1;
    }

    Ok(rendered)
}

fn insert_same_depth_entries(
    entries: &mut BTreeMap<String, RenderedEnvEntry>,
    new_entries: BTreeMap<String, RenderedEnvEntry>,
) -> Result<(), ConfigError> {
    for (key, entry) in new_entries {
        if let Some(existing) = entries.get(&key) {
            return Err(ConfigError::DuplicateRenderedEnvKey {
                key,
                first: existing.source.clone(),
                second: entry.source,
            });
        }
        entries.insert(key, entry);
    }

    Ok(())
}

fn insert_same_depth_key(
    entries: &mut BTreeMap<String, String>,
    key: &str,
    source: String,
) -> Result<(), ConfigError> {
    if let Some(existing) = entries.get(key) {
        return Err(ConfigError::DuplicateRenderedEnvKey {
            key: key.to_string(),
            first: existing.clone(),
            second: source,
        });
    }
    entries.insert(key.to_string(), source);

    Ok(())
}

fn is_safe_unquoted_value(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            !character.is_whitespace() && !matches!(character, '#' | '"' | '\'' | '\\' | '$')
        })
}

fn split_env_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut lines = content
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn join_env_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut content = lines.join("\n");
    content.push('\n');
    content
}

fn managed_blocks(lines: &[String]) -> Result<Vec<(usize, usize)>, ConfigError> {
    let mut blocks = Vec::new();
    let mut current_start = None;

    for (index, line) in lines.iter().enumerate() {
        if line == MANAGED_ENV_START_MARKER {
            if current_start.is_some() {
                return Err(ConfigError::MalformedManagedEnvBlock {
                    reason: "nested start marker",
                });
            }
            current_start = Some(index);
            continue;
        }

        if line == MANAGED_ENV_END_MARKER {
            let Some(start) = current_start.take() else {
                return Err(ConfigError::MalformedManagedEnvBlock {
                    reason: "end marker without start marker",
                });
            };
            blocks.push((start, index));
        }
    }

    if current_start.is_some() {
        return Err(ConfigError::MalformedManagedEnvBlock {
            reason: "start marker without end marker",
        });
    }

    Ok(blocks)
}

fn managed_block_lines(rendered: &RenderedProjectEnv) -> Vec<String> {
    let mut lines = Vec::with_capacity(rendered.values.len() + 2);
    lines.push(MANAGED_ENV_START_MARKER.to_string());
    lines.extend(
        rendered
            .values
            .iter()
            .map(|(key, value)| format!("{key}={}", format_env_value(value))),
    );
    lines.push(MANAGED_ENV_END_MARKER.to_string());
    lines
}

fn append_managed_block(mut lines: Vec<String>, block_lines: Vec<String>) -> Vec<String> {
    lines.extend(block_lines);
    lines
}

fn replace_managed_block(
    lines: Vec<String>,
    start: usize,
    end: usize,
    block_lines: Vec<String>,
) -> Vec<String> {
    let mut transformed = Vec::new();

    transformed.extend(lines[..start].iter().cloned());
    transformed.extend(block_lines);
    transformed.extend(lines[end + 1..].iter().cloned());

    transformed
}

fn fold_managed_blocks(
    lines: Vec<String>,
    blocks: &[(usize, usize)],
    block_lines: Vec<String>,
) -> Vec<String> {
    let mut transformed = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if blocks
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&index))
        {
            continue;
        }
        transformed.push(line.clone());
    }
    transformed.extend(block_lines);

    transformed
}

fn duplicate_existing_key_warnings(
    lines: &[String],
    blocks: &[(usize, usize)],
    rendered: &RenderedProjectEnv,
) -> Vec<ProjectEnvWarning> {
    let generated_keys = rendered
        .values
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut duplicate_keys = BTreeSet::new();

    for (index, line) in lines.iter().enumerate() {
        if blocks
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&index))
        {
            continue;
        }

        let Some(key) = assignment_key(line) else {
            continue;
        };
        if generated_keys.contains(key) {
            duplicate_keys.insert(key.to_string());
        }
    }

    duplicate_keys
        .into_iter()
        .map(|key| ProjectEnvWarning::DuplicateExistingKey { key })
        .collect()
}

fn assignment_key(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let assignment = line.strip_prefix("export ").unwrap_or(line);
    let (key, _) = assignment.split_once('=')?;
    let key = key.trim();
    if is_env_key(key) { Some(key) } else { None }
}

fn is_env_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };

    let first_valid = first.is_ascii_uppercase() || first == b'_';
    let rest_valid =
        bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');

    first_valid && rest_valid
}
