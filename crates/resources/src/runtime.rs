use camino::{Utf8Path, Utf8PathBuf};

use crate::fs;
use crate::{ResourceAdapter, ResourceName, ResourcesError, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeArtifactAdapter {
    resource_name: ResourceName,
    executable_relative_path: Utf8PathBuf,
}

impl RuntimeArtifactAdapter {
    fn new(resource_name: ResourceName, executable_relative_path: impl Into<Utf8PathBuf>) -> Self {
        Self {
            resource_name,
            executable_relative_path: executable_relative_path.into(),
        }
    }

    pub fn executable_path(&self, release: &Utf8Path) -> Utf8PathBuf {
        release.join(&self.executable_relative_path)
    }
}

impl ResourceAdapter for RuntimeArtifactAdapter {
    fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    fn validate_installation(&self, root: &Utf8Path) -> Result<()> {
        let executable_path = self.executable_path(root);
        if fs::path_is_file(&executable_path)? {
            return Ok(());
        }

        Err(ResourcesError::InvalidArtifactLayout {
            resource: self.resource_name.as_str().to_string(),
            reason: format!("missing executable `{}`", self.executable_relative_path),
        })
    }
}

pub fn php_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("php")?,
        "bin/php",
    ))
}

pub fn frankenphp_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("frankenphp")?,
        "bin/frankenphp",
    ))
}

pub fn composer_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("composer")?,
        "composer.phar",
    ))
}

pub fn mailpit_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("mailpit")?,
        "bin/mailpit",
    ))
}
