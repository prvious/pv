use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use state::{PvPaths, StateError, fs};

use crate::{ResourcesError, Result, php_track_environment};

pub const PHP_EXTENSION_METADATA_PATH: &str = "share/pv/php-extensions.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PhpExtensionLoadKind {
    Extension,
    ZendExtension,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PhpExtensionModule {
    pub name: String,
    pub load_kind: PhpExtensionLoadKind,
    pub relative_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpExtensionResolution {
    pub requested: Vec<String>,
    pub loaded: Vec<PhpExtensionModule>,
    pub ignored: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhpExtensionModule {
    name: String,
    load_kind: String,
    path: String,
}

pub fn read_php_extension_metadata(artifact_root: &Utf8Path) -> Result<Vec<PhpExtensionModule>> {
    let path = artifact_root.join(PHP_EXTENSION_METADATA_PATH);
    if !fs::path_entry_exists(&path).map_err(resources_error_from_state)? {
        return Ok(Vec::new());
    }

    let source = fs::read_to_string(&path).map_err(resources_error_from_state)?;
    let raw_modules =
        serde_json::from_str::<Vec<RawPhpExtensionModule>>(&source).map_err(|error| {
            ResourcesError::InvalidArtifactLayout {
                resource: "php".to_string(),
                reason: format!("invalid PHP extension metadata: {error}"),
            }
        })?;

    let mut modules = Vec::new();
    let mut names = BTreeSet::new();
    for raw in raw_modules {
        let module = PhpExtensionModule::from_raw(raw)?;
        if !names.insert(module.name.clone()) {
            return Err(ResourcesError::InvalidArtifactLayout {
                resource: "php".to_string(),
                reason: format!("duplicate PHP extension `{}`", module.name),
            });
        }
        let module_path = artifact_root.join(&module.relative_path);
        if !fs::path_is_file(&module_path).map_err(resources_error_from_state)? {
            return Err(ResourcesError::InvalidArtifactLayout {
                resource: "php".to_string(),
                reason: format!("PHP extension module `{module_path}` is missing"),
            });
        }
        modules.push(module);
    }

    Ok(modules)
}

pub fn resolve_php_extension_request(
    artifact_root: &Utf8Path,
    requested: &[String],
) -> Result<PhpExtensionResolution> {
    let catalog = read_php_extension_metadata(artifact_root)?;
    let requested = requested.to_vec();
    let mut requested_unique = BTreeSet::new();
    let mut ignored = Vec::new();

    for name in &requested {
        if !requested_unique.insert(name.clone()) {
            continue;
        }
        if !catalog.iter().any(|module| module.name == *name) {
            ignored.push(name.clone());
        }
    }
    let loaded = catalog
        .into_iter()
        .filter(|module| requested_unique.contains(&module.name))
        .collect();

    Ok(PhpExtensionResolution {
        requested,
        loaded,
        ignored,
    })
}

pub fn resolve_persisted_php_extension_modules(
    artifact_root: &Utf8Path,
    loaded_extensions: &[String],
) -> Result<Vec<PhpExtensionModule>> {
    if loaded_extensions.is_empty() {
        return Ok(Vec::new());
    }

    let catalog = read_php_extension_metadata(artifact_root)?;
    let mut loaded_unique = BTreeSet::new();

    for name in loaded_extensions {
        if !loaded_unique.insert(name.clone()) {
            continue;
        }
        if !catalog.iter().any(|module| module.name == *name) {
            return Err(ResourcesError::InvalidArtifactLayout {
                resource: "php".to_string(),
                reason: format!(
                    "persisted PHP extension `{name}` is missing from installed artifact metadata"
                ),
            });
        }
    }
    let loaded = catalog
        .into_iter()
        .filter(|module| loaded_unique.contains(&module.name))
        .collect();

    Ok(loaded)
}

pub fn ensure_php_runtime_overlay(
    paths: &PvPaths,
    runtime_key: &str,
    artifact_root: &Utf8Path,
    modules: &[PhpExtensionModule],
) -> Result<Utf8PathBuf> {
    let conf_dir = paths
        .config()
        .join("php-runtimes")
        .join(runtime_key)
        .join("conf.d");
    match fs::delete_dir_all(&conf_dir) {
        Ok(()) => {}
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(resources_error_from_state(error)),
    }
    fs::ensure_user_dir(&conf_dir).map_err(resources_error_from_state)?;

    for (index, module) in modules.iter().enumerate() {
        let prefix = 10 + (index * 10);
        let directive = match module.load_kind {
            PhpExtensionLoadKind::Extension => "extension",
            PhpExtensionLoadKind::ZendExtension => "zend_extension",
        };
        let module_path = artifact_root.join(&module.relative_path);
        let ini = format!("{directive}={module_path}\n");
        fs::write_sensitive_file(
            &conf_dir.join(format!("{prefix}-{}.ini", module.name)),
            &ini,
        )
        .map_err(resources_error_from_state)?;
    }

    Ok(conf_dir)
}

pub fn php_runtime_environment(
    paths: &PvPaths,
    track: &str,
    runtime_key: &str,
    artifact_root: &Utf8Path,
    modules: &[PhpExtensionModule],
) -> Result<BTreeMap<String, String>> {
    let mut environment =
        php_track_environment(paths, track).map_err(resources_error_from_state)?;
    if !modules.is_empty() {
        let overlay = ensure_php_runtime_overlay(paths, runtime_key, artifact_root, modules)?;
        let scan_dir = environment
            .entry("PHP_INI_SCAN_DIR".to_string())
            .or_default();
        if !scan_dir.is_empty() {
            scan_dir.push(':');
        }
        scan_dir.push_str(overlay.as_str());
    }

    Ok(environment)
}

pub fn php_runtime_exec_environment(
    paths: &PvPaths,
    track: &str,
    runtime_key: &str,
    artifact_root: &Utf8Path,
    modules: &[PhpExtensionModule],
) -> Result<Vec<(OsString, OsString)>> {
    Ok(
        php_runtime_environment(paths, track, runtime_key, artifact_root, modules)?
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect(),
    )
}

impl PhpExtensionModule {
    fn from_raw(raw: RawPhpExtensionModule) -> Result<Self> {
        validate_extension_name(&raw.name)?;
        let relative_path = validate_relative_path(raw.path)?;
        let load_kind = match raw.load_kind.as_str() {
            "extension" => PhpExtensionLoadKind::Extension,
            "zend_extension" => PhpExtensionLoadKind::ZendExtension,
            _ => {
                return Err(ResourcesError::InvalidArtifactLayout {
                    resource: "php".to_string(),
                    reason: format!("invalid PHP extension load kind `{}`", raw.load_kind),
                });
            }
        };

        Ok(Self {
            name: raw.name,
            load_kind,
            relative_path,
        })
    }
}

fn validate_extension_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid {
        return Ok(());
    }

    Err(ResourcesError::InvalidArtifactLayout {
        resource: "php".to_string(),
        reason: format!("invalid PHP extension name `{name}`"),
    })
}

fn validate_relative_path(path: String) -> Result<Utf8PathBuf> {
    let path = Utf8PathBuf::from(path);
    if path.as_str().is_empty()
        || path.as_str().contains('\\')
        || path.as_str().split('/').any(str::is_empty)
        || path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component.as_str(), "." | ".."))
    {
        return Err(ResourcesError::InvalidArtifactLayout {
            resource: "php".to_string(),
            reason: format!("invalid PHP extension path `{path}`"),
        });
    }

    Ok(path)
}

fn resources_error_from_state(error: StateError) -> ResourcesError {
    match error {
        StateError::Filesystem { path, source } => ResourcesError::Filesystem {
            path: path.to_string(),
            reason: source.to_string(),
        },
        error => ResourcesError::InvalidArtifactLayout {
            resource: "php".to_string(),
            reason: error.to_string(),
        },
    }
}
