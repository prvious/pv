use std::io;

use camino::Utf8Path;

use crate::filesystem::{canonicalize_utf8, is_directory, path_present, read_to_string};
use crate::{ConfigError, ProjectConfig, ProjectConfigFile};

const PREFERRED_CONFIG_FILE: &str = "pv.yml";
const ALTERNATE_CONFIG_FILE: &str = "pv.yaml";

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

pub(crate) fn validate_config_for_root(
    project_root: &Utf8Path,
    config: ProjectConfig,
) -> Result<ProjectConfig, ConfigError> {
    validate_project_paths(project_root, config)
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
    let canonical_document_root = match canonicalize_utf8(&absolute_document_root) {
        Ok(path) => path,
        Err(ConfigError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Err(ConfigError::DocumentRootNotDirectory {
                document_root: document_root.clone(),
            });
        }
        Err(error) => return Err(error),
    };

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
