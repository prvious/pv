use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub php: Option<PhpConfig>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_path"
    )]
    pub document_root: Option<Utf8PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hostnames: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, ResourceConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigFile {
    pub path: Utf8PathBuf,
    pub exists: bool,
    pub config: ProjectConfig,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhpConfig {
    pub version: Option<String>,
    pub extensions: Vec<String>,
}

impl PhpConfig {
    pub fn version(version: impl Into<String>) -> Self {
        Self {
            version: Some(version.into()),
            extensions: Vec::new(),
        }
    }

    pub fn version_selector(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn requested_extensions(&self) -> &[String] {
        &self.extensions
    }
}

impl Serialize for PhpConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.extensions.is_empty()
            && let Some(version) = &self.version
        {
            return version.serialize(serializer);
        }

        let field_count =
            usize::from(self.version.is_some()) + usize::from(!self.extensions.is_empty());
        let mut map = serializer.serialize_map(Some(field_count))?;
        if let Some(version) = &self.version {
            map.serialize_entry("version", version)?;
        }
        if !self.extensions.is_empty() {
            map.serialize_entry("extensions", &self.extensions)?;
        }
        map.end()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ResourceConfig {
    #[serde(rename = "version", skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub allocations: BTreeMap<String, AllocationConfig>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct AllocationConfig {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl ProjectConfig {
    pub fn has_env_mappings(&self) -> bool {
        !self.env.is_empty()
            || self.resources.values().any(|resource| {
                !resource.env.is_empty()
                    || resource
                        .allocations
                        .values()
                        .any(|allocation| !allocation.env.is_empty())
            })
    }
}

fn serialize_optional_path<S>(path: &Option<Utf8PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    path.as_ref()
        .map(|path| path.as_str())
        .serialize(serializer)
}
