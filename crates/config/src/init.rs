use std::collections::{BTreeMap, BTreeSet};
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value as JsonValue;

use crate::filesystem::{is_directory, path_present, read_to_string};
use crate::{
    AllocationConfig, ConfigError, PhpConfig, ProjectConfig, ProjectConfigFile, ResourceConfig,
};

const SUPPORTED_PHP_TRACKS: &[&str] = &["8.3", "8.4", "8.5"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitDetection {
    pub config_file: ProjectConfigFile,
    pub signals: Vec<ProjectInitSignal>,
    pub suggested_php: String,
    pub suggested_document_root: Option<Utf8PathBuf>,
    pub include_app_url: bool,
    pub include_vite_tls: bool,
    pub resources: BTreeMap<ProjectInitResourceName, ProjectInitResourceDetection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitSignal {
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitResourceDetection {
    pub selected: bool,
    pub reason: String,
    pub default_allocations: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitSelection {
    pub php: String,
    pub document_root: Option<Utf8PathBuf>,
    pub include_app_url: bool,
    pub include_vite_tls: bool,
    pub resources: BTreeMap<ProjectInitResourceName, ProjectInitResourceSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitResourceSelection {
    pub selected: bool,
    pub track: String,
    pub allocations: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ProjectInitResourceName {
    Mailpit,
    Mysql,
    Postgres,
    Redis,
    Rustfs,
}

impl ProjectInitResourceName {
    fn config_key(self) -> &'static str {
        match self {
            Self::Mailpit => "mailpit",
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
            Self::Redis => "redis",
            Self::Rustfs => "rustfs",
        }
    }
}

pub fn detect_project_init(project_root: &Utf8Path) -> Result<ProjectInitDetection, ConfigError> {
    let config_file = ProjectConfigFile::read_from_root(project_root)?;
    let project_root = config_file
        .path
        .parent()
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| ConfigError::ProjectRootNotDirectory {
            path: project_root.to_path_buf(),
        })?;
    let env = read_env_shape(&project_root)?;
    let composer = read_json_file(&project_root.join("composer.json"))?;
    let package = read_json_file(&project_root.join("package.json"))?;
    let laravel_detected = is_laravel_project(&project_root, composer.as_ref())?;
    let vite_detected = package_has_key(package.as_ref(), "devDependencies", "vite")
        || package_has_key(package.as_ref(), "dependencies", "vite");
    let mut signals = Vec::new();

    if laravel_detected {
        signals.push(ProjectInitSignal {
            label: "Laravel".to_string(),
            detail: "Detected composer.json, artisan, and Laravel project files".to_string(),
        });
    }
    if vite_detected {
        signals.push(ProjectInitSignal {
            label: "Vite".to_string(),
            detail: "Detected Vite in package.json".to_string(),
        });
    }

    let suggested_php = composer
        .as_ref()
        .and_then(detect_php_track_from_composer)
        .unwrap_or_else(|| "latest".to_string());
    let public = project_root.join("public");
    let suggested_document_root = if path_present(&public)? && is_directory(&public)? {
        Some(Utf8PathBuf::from("public"))
    } else {
        None
    };
    let include_app_url = env.contains_key("APP_URL") || laravel_detected;
    let resources = detect_resources(&env);

    Ok(ProjectInitDetection {
        config_file,
        signals,
        suggested_php,
        suggested_document_root,
        include_app_url,
        include_vite_tls: vite_detected,
        resources,
    })
}

pub fn default_project_init_selection(detection: &ProjectInitDetection) -> ProjectInitSelection {
    let resources = detection
        .resources
        .iter()
        .map(|(name, detected)| {
            (
                *name,
                ProjectInitResourceSelection {
                    selected: detected.selected,
                    track: "latest".to_string(),
                    allocations: detected.default_allocations.clone(),
                },
            )
        })
        .collect();

    ProjectInitSelection {
        php: detection
            .config_file
            .config
            .php
            .as_ref()
            .and_then(PhpConfig::version_selector)
            .map(str::to_string)
            .unwrap_or_else(|| detection.suggested_php.clone()),
        document_root: detection
            .config_file
            .config
            .document_root
            .clone()
            .or_else(|| detection.suggested_document_root.clone()),
        include_app_url: detection.include_app_url || !detection.config_file.config.env.is_empty(),
        include_vite_tls: detection.include_vite_tls,
        resources,
    }
}

pub fn render_project_init_config(
    detection: &ProjectInitDetection,
    selection: &ProjectInitSelection,
) -> Result<ProjectConfig, ConfigError> {
    let mut config = detection.config_file.config.clone();
    config.php.get_or_insert_with(PhpConfig::default).version = Some(selection.php.clone());
    if config.document_root.is_none() {
        config.document_root = selection.document_root.clone();
    }
    if selection.include_app_url {
        config
            .env
            .entry("APP_URL".to_string())
            .or_insert_with(|| "${project_url}".to_string());
    }
    if selection.include_vite_tls {
        config
            .env
            .entry("VITE_DEV_SERVER_CERT".to_string())
            .or_insert_with(|| "${tls_cert}".to_string());
        config
            .env
            .entry("VITE_DEV_SERVER_KEY".to_string())
            .or_insert_with(|| "${tls_key}".to_string());
    }

    for (name, resource) in &selection.resources {
        if resource.selected {
            merge_resource(&mut config, *name, resource);
        }
    }

    let content = yaml_serde::to_string(&config).map_err(|source| ConfigError::Parse { source })?;
    ProjectConfig::parse(&content)
}

fn read_json_file(path: &Utf8Path) -> Result<Option<JsonValue>, ConfigError> {
    match read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map(Some).map_err(|source| {
            ConfigError::InvalidInitJson {
                path: path.to_path_buf(),
                reason: source.to_string(),
            }
        }),
        Err(ConfigError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn read_env_shape(project_root: &Utf8Path) -> Result<BTreeMap<String, String>, ConfigError> {
    let example = project_root.join(".env.example");
    if path_present(&example)? {
        return read_env_file(&example);
    }

    let env = project_root.join(".env");
    if path_present(&env)? {
        return read_env_file(&env);
    }

    Ok(BTreeMap::new())
}

fn read_env_file(path: &Utf8Path) -> Result<BTreeMap<String, String>, ConfigError> {
    let mut values = BTreeMap::new();
    let contents = read_to_string(path)?;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(
                key.trim().to_string(),
                trim_env_value(value.trim()).to_string(),
            );
        }
    }

    Ok(values)
}

fn trim_env_value(value: &str) -> &str {
    value.trim_matches('"').trim_matches('\'')
}

fn is_laravel_project(
    project_root: &Utf8Path,
    composer: Option<&JsonValue>,
) -> Result<bool, ConfigError> {
    if !package_has_key(composer, "require", "laravel/framework")
        || !path_present(&project_root.join("artisan"))?
    {
        return Ok(false);
    }

    for relative_path in ["bootstrap/app.php", "config/app.php", "public/index.php"] {
        if path_present(&project_root.join(relative_path))? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn package_has_key(package: Option<&JsonValue>, section: &str, key: &str) -> bool {
    package
        .and_then(|package| package.get(section))
        .and_then(JsonValue::as_object)
        .is_some_and(|dependencies| dependencies.contains_key(key))
}

fn detect_php_track_from_composer(composer: &JsonValue) -> Option<String> {
    let constraint = composer.get("require")?.as_object()?.get("php")?.as_str()?;
    let constraint = constraint.trim();

    if constraint.is_empty()
        || constraint.contains(char::is_whitespace)
        || constraint.contains('|')
        || constraint.contains(',')
    {
        return None;
    }

    let version = constraint
        .strip_prefix('^')
        .or_else(|| constraint.strip_prefix('~'))
        .or_else(|| constraint.strip_prefix("=="))
        .or_else(|| constraint.strip_prefix('='))
        .unwrap_or(constraint);
    if version.starts_with(['<', '>', '!']) {
        return None;
    }

    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major != "8"
        || minor.is_empty()
        || !minor.chars().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    if parts.any(|part| {
        !matches!(part, "*" | "x" | "X")
            && (part.is_empty() || !part.chars().all(|character| character.is_ascii_digit()))
    }) {
        return None;
    }

    let track = format!("{major}.{minor}");
    SUPPORTED_PHP_TRACKS
        .contains(&track.as_str())
        .then_some(track)
}

fn detect_resources(
    env: &BTreeMap<String, String>,
) -> BTreeMap<ProjectInitResourceName, ProjectInitResourceDetection> {
    let mut resources = BTreeMap::new();
    let db_connection = env.get("DB_CONNECTION").map(String::as_str);
    let mysql_selected = matches!(db_connection, Some("mysql"));
    let postgres_selected = matches!(db_connection, Some("pgsql" | "postgres"));

    resources.insert(
        ProjectInitResourceName::Mysql,
        ProjectInitResourceDetection {
            selected: mysql_selected,
            reason: if mysql_selected {
                "Detected DB_CONNECTION=mysql".to_string()
            } else {
                "No strong MySQL signal".to_string()
            },
            default_allocations: vec!["app".to_string()],
        },
    );
    resources.insert(
        ProjectInitResourceName::Postgres,
        ProjectInitResourceDetection {
            selected: postgres_selected,
            reason: if postgres_selected {
                "Detected DB_CONNECTION=pgsql".to_string()
            } else {
                "No strong Postgres signal".to_string()
            },
            default_allocations: vec!["app".to_string()],
        },
    );
    resources.insert(
        ProjectInitResourceName::Redis,
        ProjectInitResourceDetection {
            selected: env.contains_key("REDIS_HOST")
                || env.contains_key("REDIS_URL")
                || matches!(env.get("CACHE_STORE").map(String::as_str), Some("redis"))
                || matches!(env.get("CACHE_DRIVER").map(String::as_str), Some("redis"))
                || matches!(env.get("SESSION_DRIVER").map(String::as_str), Some("redis"))
                || matches!(
                    env.get("QUEUE_CONNECTION").map(String::as_str),
                    Some("redis")
                ),
            reason: "Detected Redis-compatible env keys when selected".to_string(),
            default_allocations: vec!["cache".to_string()],
        },
    );
    resources.insert(
        ProjectInitResourceName::Mailpit,
        ProjectInitResourceDetection {
            selected: matches!(env.get("MAIL_MAILER").map(String::as_str), Some("smtp"))
                || env.contains_key("MAIL_HOST")
                || env.contains_key("MAIL_PORT")
                || env.contains_key("MAIL_FROM_ADDRESS"),
            reason: "Detected SMTP mail env keys when selected".to_string(),
            default_allocations: Vec::new(),
        },
    );
    resources.insert(
        ProjectInitResourceName::Rustfs,
        ProjectInitResourceDetection {
            selected: env
                .keys()
                .any(|key| key.starts_with("AWS_") || key.starts_with("S3_")),
            reason: "Detected AWS/S3 env keys when selected".to_string(),
            default_allocations: vec!["uploads".to_string()],
        },
    );

    resources
}

fn merge_resource(
    config: &mut ProjectConfig,
    name: ProjectInitResourceName,
    selection: &ProjectInitResourceSelection,
) {
    let resource = config
        .resources
        .entry(name.config_key().to_string())
        .or_default();
    resource
        .track
        .get_or_insert_with(|| selection.track.clone());

    match name {
        ProjectInitResourceName::Mailpit => merge_mailpit(resource),
        ProjectInitResourceName::Mysql => merge_sql(resource, "mysql", &selection.allocations),
        ProjectInitResourceName::Postgres => merge_sql(resource, "pgsql", &selection.allocations),
        ProjectInitResourceName::Redis => merge_redis(resource, &selection.allocations),
        ProjectInitResourceName::Rustfs => merge_rustfs(resource, &selection.allocations),
    }
}

fn merge_mailpit(resource: &mut ResourceConfig) {
    merge_env_defaults(
        &mut resource.env,
        [
            ("MAIL_MAILER", "smtp"),
            ("MAIL_HOST", "${smtp_host}"),
            ("MAIL_PORT", "${smtp_port}"),
        ],
    );
}

fn merge_sql(resource: &mut ResourceConfig, connection: &str, allocations: &[String]) {
    merge_allocations(resource, allocations, |allocation, prefix| {
        merge_prefixed_env_defaults(
            &mut allocation.env,
            prefix,
            [
                ("DB_CONNECTION", connection),
                ("DB_HOST", "${host}"),
                ("DB_PORT", "${port}"),
                ("DB_DATABASE", "${database}"),
                ("DB_USERNAME", "${username}"),
                ("DB_PASSWORD", "${password}"),
            ],
        );
    });
}

fn merge_redis(resource: &mut ResourceConfig, allocations: &[String]) {
    merge_allocations(resource, allocations, |allocation, prefix| {
        merge_prefixed_env_defaults(
            &mut allocation.env,
            prefix,
            [
                ("REDIS_HOST", "${host}"),
                ("REDIS_PORT", "${port}"),
                ("REDIS_PREFIX", "${prefix}"),
            ],
        );
    });
}

fn merge_rustfs(resource: &mut ResourceConfig, allocations: &[String]) {
    merge_allocations(resource, allocations, |allocation, prefix| {
        merge_prefixed_env_defaults(
            &mut allocation.env,
            prefix,
            [
                ("AWS_ENDPOINT", "${endpoint}"),
                ("AWS_BUCKET", "${bucket}"),
                ("AWS_ACCESS_KEY_ID", "${access_key}"),
                ("AWS_SECRET_ACCESS_KEY", "${secret_key}"),
            ],
        );
    });
}

fn merge_allocations(
    resource: &mut ResourceConfig,
    allocations: &[String],
    mut merge: impl FnMut(&mut AllocationConfig, Option<&str>),
) {
    let mut seen = BTreeSet::new();
    for allocation_name in allocations {
        if !seen.insert(allocation_name) {
            continue;
        }

        let prefix = (seen.len() > 1).then(|| allocation_env_prefix(allocation_name));
        let allocation = resource
            .allocations
            .entry(allocation_name.clone())
            .or_default();
        merge(allocation, prefix.as_deref());
    }
}

fn allocation_env_prefix(allocation: &str) -> String {
    allocation
        .chars()
        .map(|character| {
            if character == '-' {
                '_'
            } else {
                character.to_ascii_uppercase()
            }
        })
        .collect()
}

fn merge_prefixed_env_defaults<const N: usize>(
    env: &mut BTreeMap<String, String>,
    prefix: Option<&str>,
    defaults: [(&str, &str); N],
) {
    for (key, value) in defaults {
        let key = prefix.map_or_else(|| key.to_string(), |prefix| format!("{prefix}_{key}"));
        env.entry(key).or_insert_with(|| value.to_string());
    }
}

fn merge_env_defaults<const N: usize>(
    env: &mut BTreeMap<String, String>,
    defaults: [(&str, &str); N],
) {
    for (key, value) in defaults {
        env.entry(key.to_string())
            .or_insert_with(|| value.to_string());
    }
}
