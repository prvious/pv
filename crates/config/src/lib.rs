mod discovery;
mod env;
mod error;
mod filesystem;
mod hostname;
mod init;
mod model;
mod parser;
mod writer;

pub use env::{
    AllocationEnvContext, MANAGED_ENV_END_MARKER, MANAGED_ENV_START_MARKER,
    ManagedEnvBlockTransform, ProjectEnvContext, ProjectEnvWarning, RenderedProjectEnv,
    ResourceEnvContext, format_env_value, format_project_env, render_project_env,
    transform_managed_env_block, validate_managed_env_block, validate_project_env_shape,
    write_project_env_file,
};
pub use error::{ConfigCapability, ConfigError};
pub use hostname::{
    hostname_from_project_path, normalize_additional_hostname, normalize_primary_hostname,
};
pub use init::{
    ProjectInitDetection, ProjectInitResourceDetection, ProjectInitResourceName,
    ProjectInitResourceSelection, ProjectInitSelection, ProjectInitSignal,
    default_project_init_selection, detect_project_init, render_project_init_config,
};
pub use model::{AllocationConfig, PhpConfig, ProjectConfig, ProjectConfigFile, ResourceConfig};
pub use writer::{update_project_config, write_project_config, write_project_php_track};
