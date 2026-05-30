use camino::Utf8Path;

use crate::ConfigError;

const RESERVED_HOSTNAME: &str = "pv.test";
const MAX_DNS_LABEL_LENGTH: usize = 63;
const MAX_HOSTNAME_LENGTH: usize = 253;

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
