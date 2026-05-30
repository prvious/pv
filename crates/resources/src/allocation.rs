use crate::error::{ResourcesError, Result};
use crate::identity::ResourceName;
use crate::registry;

const MAX_GENERATED_ALLOCATION_NAME_LEN: usize = 63;

/// Resource-specific allocation object created for a linked Project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceAllocationKind {
    /// SQL database allocation for MySQL and Postgres.
    SqlDatabase,
    /// Redis key prefix allocation.
    RedisPrefix,
    /// RustFS bucket allocation.
    RustfsBucket,
}

/// Generated Resource allocation name and its canonical resource owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAllocationName {
    resource_name: ResourceName,
    allocation_name: String,
    generated_name: String,
}

/// Placeholders PV can provide for resource and allocation env rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvPlaceholderContract {
    resource_placeholders: &'static [&'static str],
    allocation_placeholders: &'static [&'static str],
}

impl EnvPlaceholderContract {
    pub const fn new(
        resource_placeholders: &'static [&'static str],
        allocation_placeholders: &'static [&'static str],
    ) -> Self {
        Self {
            resource_placeholders,
            allocation_placeholders,
        }
    }

    pub fn resource_placeholders(self) -> &'static [&'static str] {
        self.resource_placeholders
    }

    pub fn allocation_placeholders(self) -> &'static [&'static str] {
        self.allocation_placeholders
    }
}

impl ResourceAllocationName {
    pub fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub fn allocation_name(&self) -> &str {
        &self.allocation_name
    }

    pub fn generated_name(&self) -> &str {
        &self.generated_name
    }
}

/// Generates the stable backing object name for a Resource allocation.
pub fn generated_allocation_name(
    resource_name_or_alias: &str,
    primary_hostname: &str,
    allocation_name: &str,
) -> Result<ResourceAllocationName> {
    let descriptor = registry::resolve(resource_name_or_alias)?;
    let Some(allocation_kind) = descriptor.allocation_kind() else {
        return Err(ResourcesError::UnsupportedResourceAllocation {
            resource: descriptor.name().to_string(),
        });
    };
    let generated_name = match allocation_kind {
        ResourceAllocationKind::SqlDatabase => {
            format!(
                "{}_{}",
                sql_hostname_slug(primary_hostname),
                allocation_name.replace('-', "_")
            )
        }
        ResourceAllocationKind::RedisPrefix => {
            format!(
                "{}-{}-",
                dash_hostname_slug(primary_hostname),
                allocation_name.replace('_', "-")
            )
        }
        ResourceAllocationKind::RustfsBucket => {
            format!(
                "{}-{}",
                dash_hostname_slug(primary_hostname),
                allocation_name.replace('_', "-")
            )
        }
    };

    if generated_name.len() > MAX_GENERATED_ALLOCATION_NAME_LEN {
        return Err(ResourcesError::GeneratedAllocationNameTooLong {
            resource: descriptor.name().to_string(),
            allocation: allocation_name.to_string(),
            generated: generated_name,
            max: MAX_GENERATED_ALLOCATION_NAME_LEN,
        });
    }

    Ok(ResourceAllocationName {
        resource_name: ResourceName::new(descriptor.name())?,
        allocation_name: allocation_name.to_string(),
        generated_name,
    })
}

/// Returns resource-level env placeholders for a canonical resource or alias.
pub fn resource_env_placeholders(resource_name_or_alias: &str) -> Result<&'static [&'static str]> {
    Ok(registry::resolve(resource_name_or_alias)?
        .env_placeholder_contract()
        .resource_placeholders())
}

/// Returns allocation-level env placeholders for a canonical resource or alias.
pub fn allocation_env_placeholders(
    resource_name_or_alias: &str,
) -> Result<&'static [&'static str]> {
    Ok(registry::resolve(resource_name_or_alias)?
        .env_placeholder_contract()
        .allocation_placeholders())
}

fn sql_hostname_slug(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '.' | '-' => '_',
            character => character,
        })
        .collect()
}

fn dash_hostname_slug(value: &str) -> String {
    value.replace('.', "-")
}
