mod discovery;
mod env;
mod error;
mod filesystem;
mod hostname;
mod model;
mod parser;

pub use env::{
    AllocationEnvContext, MANAGED_ENV_END_MARKER, MANAGED_ENV_START_MARKER,
    ManagedEnvBlockTransform, ProjectEnvContext, ProjectEnvWarning, RenderedProjectEnv,
    ResourceEnvContext, format_env_value, format_project_env, render_project_env,
    transform_managed_env_block, validate_project_env_shape, write_project_env_file,
};
pub use error::ConfigError;
pub use hostname::{
    hostname_from_project_path, normalize_additional_hostname, normalize_primary_hostname,
};
pub use model::{AllocationConfig, ProjectConfig, ProjectConfigFile, ResourceConfig};
