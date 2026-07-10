use camino::Utf8Path;

use crate::discovery::validate_config_for_root;
use crate::filesystem::{canonicalize_utf8, file_mode, write_string_atomically_with_mode};
use crate::{ConfigError, PhpConfig, ProjectConfig, ProjectConfigFile};

const PROJECT_CONFIG_FILE_MODE: u32 = 0o644;

pub fn write_project_config(
    project_root: &Utf8Path,
    config: &ProjectConfig,
) -> Result<ProjectConfigFile, ConfigError> {
    let mut config_file = ProjectConfigFile::read_from_root(project_root)?;
    let canonical_root = config_file
        .path
        .parent()
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| ConfigError::ProjectRootNotDirectory {
            path: project_root.to_path_buf(),
        })?;
    let content = yaml_serde::to_string(config).map_err(|source| ConfigError::Parse { source })?;
    let parsed = ProjectConfig::parse(&content)?;
    let parsed = validate_config_for_root(&canonical_root, parsed)?;

    let (write_path, mode) = if config_file.exists {
        let target = canonicalize_utf8(&config_file.path)?;
        let mode = file_mode(&target)?;
        (target, mode)
    } else {
        (config_file.path.clone(), PROJECT_CONFIG_FILE_MODE)
    };
    write_string_atomically_with_mode(&write_path, &content, mode)?;

    config_file.exists = true;
    config_file.config = parsed;
    Ok(config_file)
}

pub fn update_project_config(
    project_root: &Utf8Path,
    update: impl FnOnce(&mut ProjectConfig),
) -> Result<ProjectConfigFile, ConfigError> {
    let mut config_file = ProjectConfigFile::read_from_root(project_root)?;
    update(&mut config_file.config);

    write_project_config(project_root, &config_file.config)
}

pub fn write_project_php_track(
    project_root: &Utf8Path,
    track: &str,
) -> Result<ProjectConfigFile, ConfigError> {
    update_project_config(project_root, |config| {
        let php = config.php.get_or_insert_with(PhpConfig::default);
        php.version = Some(track.to_string());
    })
}
