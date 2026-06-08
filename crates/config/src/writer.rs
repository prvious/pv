use camino::Utf8Path;

use crate::filesystem::{canonicalize_utf8, file_mode, write_string_atomically_with_mode};
use crate::{ConfigError, ProjectConfig, ProjectConfigFile};

const PROJECT_CONFIG_FILE_MODE: u32 = 0o644;

pub fn write_project_php_track(
    project_root: &Utf8Path,
    track: &str,
) -> Result<ProjectConfigFile, ConfigError> {
    let mut config_file = ProjectConfigFile::read_from_root(project_root)?;
    config_file.config.php = Some(track.to_string());

    let content = yaml_serde::to_string(&config_file.config)
        .map_err(|source| ConfigError::Parse { source })?;
    config_file.config = ProjectConfig::parse(&content)?;

    let (write_path, mode) = if config_file.exists {
        let target = canonicalize_utf8(&config_file.path)?;
        let mode = file_mode(&target)?;
        (target, mode)
    } else {
        (config_file.path.clone(), PROJECT_CONFIG_FILE_MODE)
    };
    write_string_atomically_with_mode(&write_path, &content, mode)?;

    config_file.exists = true;
    Ok(config_file)
}
