use camino::Utf8Path;

use crate::filesystem::write_string_atomically_with_mode;
use crate::{ConfigError, ProjectConfigFile};

const PROJECT_CONFIG_FILE_MODE: u32 = 0o644;

pub fn write_project_php_track(
    project_root: &Utf8Path,
    track: &str,
) -> Result<ProjectConfigFile, ConfigError> {
    let mut config_file = ProjectConfigFile::read_from_root(project_root)?;
    config_file.config.php = Some(track.to_string());

    let content = yaml_serde::to_string(&config_file.config)
        .map_err(|source| ConfigError::Parse { source })?;
    write_string_atomically_with_mode(&config_file.path, &content, PROJECT_CONFIG_FILE_MODE)?;

    config_file.exists = true;
    Ok(config_file)
}
