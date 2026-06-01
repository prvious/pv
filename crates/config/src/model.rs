use std::collections::BTreeMap;

use camino::Utf8PathBuf;

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
