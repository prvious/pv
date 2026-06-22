# PHP Extension Opt-Ins Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Project-level PHP extension opt-ins through `php.extensions`, with bundled optional shared modules loaded by PV-generated runtime overlays.

**Architecture:** The config crate normalizes scalar and object PHP config into a single `PhpConfig` model. Release tooling publishes optional PHP extension metadata into manifests and an artifact-local `share/pv/php-extensions.json`, and runtime code reads the installed artifact metadata so daemon restarts and shims work offline. State stores the last valid PHP runtime assignment, and the daemon/CLI use a runtime identity of resolved PHP track plus sorted loaded extension names.

**Tech Stack:** Rust, `yaml_serde`, `serde`, `rusqlite`, StaticPHP v3 recipe shell scripts, PV artifact manifests, `insta` snapshots, and `cargo nextest`.

## Global Constraints

- Keep existing scalar Project config valid: `php: 8.4`.
- Support object Project config: `php.version` and `php.extensions`.
- `php.extensions` must be a YAML array of strings.
- Unsupported extension names are not Project config errors.
- Ignored extension names must be visible as non-blocking diagnostics.
- No named profiles, presets, local PECL, `phpize`, `php-config`, arbitrary `.so`, or custom Project PHP ini.
- Default loaded extensions: `bcmath`, `ctype`, `curl`, `dom`, `fileinfo`, `filter`, `hash`, `iconv`, `intl`, `json`, `libxml`, `mbstring`, `openssl`, `pcntl`, `pcre`, `pdo`, `pdo_mysql`, `pdo_pgsql`, `pdo_sqlite`, `phar`, `posix`, `session`, `simplexml`, `sodium`, `sqlite3`, `tokenizer`, `xml`, `xmlreader`, `xmlwriter`, `zip`, `zlib`.
- Initial optional catalog: `redis`, `sqlsrv`, `pdo_sqlsrv`, `xdebug`, `apcu`, `pcov`, `imagick`, `mongodb`, `yaml`.
- Runtime identity is resolved PHP track plus sorted available loaded extension names.
- Standalone PHP, Composer-through-PHP, and FrankenPHP workers for a Project must use the same loaded extension set.
- PHP and FrankenPHP remain paired artifacts per PHP track; optional extension combinations do not create separate downloaded artifact flavors.

---

## References

- Approved spec: `docs/superpowers/specs/2026-06-22-php-extension-opt-ins-design.md`
- Canonical product design: `DESIGN.md`, section `Multi-version PHP`
- Superseding ADR: `docs/adr/0014-project-level-php-extension-opt-ins.md`
- Existing PHP defaults helper: `crates/resources/src/php_defaults.rs`
- Existing PHP runtime planner: `crates/daemon/src/gateway.rs`
- Existing PHP and Composer shims: `crates/cli/src/commands/php.rs`, `crates/cli/src/commands/composer.rs`
- Existing PHP recipe metadata: `release/artifacts/recipes/php/tracks.toml`

## File Structure

- Modify `crates/config/src/model.rs`: add `PhpConfig`, custom serialization, and helper accessors.
- Modify `crates/config/src/parser.rs`: parse scalar and object PHP config.
- Modify `crates/config/src/writer.rs`: preserve `php.extensions` while updating `php.version`.
- Modify `crates/config/src/error.rs`: add unknown PHP key errors if needed.
- Modify `crates/config/tests/project_config.rs`: cover config shapes and writer preservation.
- Create `crates/resources/src/php_extensions.rs`: artifact-local extension metadata, request resolution, ini overlay generation, and runtime env helpers.
- Modify `crates/resources/src/manifest.rs`: parse optional PHP extension metadata from artifact entries.
- Modify `crates/resources/src/runtime.rs`: require artifact-local extension metadata files when metadata advertises modules.
- Modify `crates/resources/src/lib.rs`: export PHP extension runtime helpers.
- Modify `crates/resources/tests/*`: add manifest and runtime adapter coverage.
- Modify `crates/pv-release/src/recipe.rs`: split default loaded extensions from optional shared extensions in PHP recipes.
- Modify `crates/pv-release/src/record.rs`, `record_writer.rs`, and `manifest.rs`: carry optional PHP extension metadata through release records and generated manifests.
- Modify `release/artifacts/recipes/php/tracks.toml`: define default and optional extension sets.
- Modify `release/artifacts/recipes/php/build.sh`: build optional modules shared, stage modules, write `share/pv/php-extensions.json`, and pass metadata to release records.
- Modify `release/artifacts/recipes/php/smoke.sh`: smoke default runtime without optional modules and smoke optional modules through a temporary scan dir.
- Modify `crates/state/src/sql/008_project_php_runtime_extensions.sql`: add persisted Project runtime extension fields.
- Modify `crates/state/src/migrations.rs`: register migration 8.
- Modify `crates/state/src/database.rs`: add `ProjectPhpRuntimeInput`, runtime key validation, PHP worker runtime identity support, and persisted extension fields.
- Modify `crates/state/src/paths.rs`: treat worker path arguments as runtime keys.
- Modify `crates/state/tests/state_foundation.rs`: cover migration, runtime keys, and port identities.
- Modify `crates/daemon/src/project_env.rs`: resolve requested/loaded/ignored extensions and persist last valid runtime.
- Modify `crates/daemon/src/gateway.rs`: group workers by runtime identity, generate overlays, and use runtime-key paths.
- Modify `crates/daemon/tests/project_env_reconciliation.rs` and `gateway_reconciliation.rs`: cover runtime persistence and worker grouping.
- Modify `crates/cli/src/commands/php.rs`: resolve Project runtime for the PHP shim and use overlay env.
- Modify `crates/cli/src/commands/composer.rs`: inherit the PHP shim runtime overlay.
- Modify `crates/cli/src/commands/project.rs` and `status.rs`: surface ignored-extension warnings.
- Modify `crates/cli/tests/php.rs`, `composer.rs`, `status.rs`, and `it/cli.rs`: snapshot CLI behavior.

## Interfaces

Task 1 produces:

```rust
pub struct PhpConfig {
    pub version: Option<String>,
    pub extensions: Vec<String>,
}

impl PhpConfig {
    pub fn version(version: impl Into<String>) -> Self;
    pub fn version_selector(&self) -> Option<&str>;
    pub fn requested_extensions(&self) -> &[String];
}
```

Task 2 produces:

```rust
pub enum PhpExtensionLoadKind {
    Extension,
    ZendExtension,
}

pub struct PhpExtensionModule {
    pub name: String,
    pub load_kind: PhpExtensionLoadKind,
    pub relative_path: Utf8PathBuf,
}

pub struct PhpExtensionResolution {
    pub requested: Vec<String>,
    pub loaded: Vec<PhpExtensionModule>,
    pub ignored: Vec<String>,
}

pub fn read_php_extension_metadata(artifact_root: &Utf8Path) -> Result<Vec<PhpExtensionModule>>;
pub fn resolve_php_extension_request(
    artifact_root: &Utf8Path,
    requested: &[String],
) -> Result<PhpExtensionResolution>;
pub fn ensure_php_runtime_overlay(
    paths: &PvPaths,
    runtime_key: &str,
    artifact_root: &Utf8Path,
    modules: &[PhpExtensionModule],
) -> Result<Utf8PathBuf>;
pub fn php_runtime_environment(
    paths: &PvPaths,
    track: &str,
    runtime_key: &str,
    artifact_root: &Utf8Path,
    modules: &[PhpExtensionModule],
) -> Result<BTreeMap<String, String>>;
```

Task 4 produces:

```rust
pub struct ProjectPhpRuntimeInput {
    pub track: String,
    pub requested_extensions: Vec<String>,
    pub loaded_extensions: Vec<String>,
    pub ignored_extensions: Vec<String>,
}

pub struct ProjectPhpRuntimeRecord {
    pub track: Option<String>,
    pub requested_extensions: Vec<String>,
    pub loaded_extensions: Vec<String>,
    pub ignored_extensions: Vec<String>,
}

pub fn php_runtime_key(track: &str, loaded_extensions: &[String]) -> Result<String, StateError>;
```

Task 6 consumes all prior interfaces and produces runtime plans keyed by `PhpWorkerRuntimePlan.runtime_key`.

## Task 1: Project Config PHP Object Form

**Files:**
- Modify: `crates/config/src/model.rs`
- Modify: `crates/config/src/parser.rs`
- Modify: `crates/config/src/writer.rs`
- Modify: `crates/config/src/lib.rs`
- Test: `crates/config/tests/project_config.rs`

**Interfaces:**
- Produces: `PhpConfig`, `PhpConfig::version`, `PhpConfig::version_selector`, `PhpConfig::requested_extensions`.
- Consumes: existing `TrackSelector::parse`, existing `ProjectConfig::parse`, existing `write_project_php_track`.

- [ ] **Step 1: Write failing parser tests**

Add these tests near the existing PHP config tests in `crates/config/tests/project_config.rs`:

```rust
#[test]
fn project_config_accepts_php_object_with_version_and_extensions() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
php:
  version: 8.4
  extensions:
    - redis
    - xdebug
"#,
    )?;

    let php = config.php.as_ref().ok_or_else(|| anyhow!("missing php config"))?;
    assert_eq!(php.version_selector(), Some("8.4"));
    assert_eq!(php.requested_extensions(), ["redis", "xdebug"]);

    Ok(())
}

#[test]
fn project_config_accepts_php_object_with_extensions_only() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
php:
  extensions:
    - xdebug
"#,
    )?;

    let php = config.php.as_ref().ok_or_else(|| anyhow!("missing php config"))?;
    assert_eq!(php.version_selector(), None);
    assert_eq!(php.requested_extensions(), ["xdebug"]);

    Ok(())
}

#[test]
fn project_config_rejects_invalid_php_extensions_shape() -> Result<()> {
    assert!(matches!(
        ProjectConfig::parse("php:\n  extensions: redis\n"),
        Err(ConfigError::InvalidFieldType { field, .. }) if field == "php.extensions"
    ));
    assert!(matches!(
        ProjectConfig::parse("php:\n  extensions:\n    - true\n"),
        Err(ConfigError::InvalidFieldType { field, .. }) if field == "php.extensions"
    ));

    Ok(())
}
```

- [ ] **Step 2: Run parser tests to verify failure**

Run:

```bash
cargo nextest run -p config -E 'test(project_config_accepts_php_object_with_version_and_extensions) or test(project_config_accepts_php_object_with_extensions_only) or test(project_config_rejects_invalid_php_extensions_shape)'
```

Expected: FAIL to compile because `PhpConfig` and its accessors do not exist.

- [ ] **Step 3: Implement `PhpConfig` model and serialization**

In `crates/config/src/model.rs`, replace the `php: Option<String>` field with `php: Option<PhpConfig>` and add:

```rust
use serde::ser::SerializeMap;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhpConfig {
    pub version: Option<String>,
    pub extensions: Vec<String>,
}

impl PhpConfig {
    pub fn version(version: impl Into<String>) -> Self {
        Self {
            version: Some(version.into()),
            extensions: Vec::new(),
        }
    }

    pub fn version_selector(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn requested_extensions(&self) -> &[String] {
        &self.extensions
    }
}

impl Serialize for PhpConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.extensions.is_empty()
            && let Some(version) = &self.version
        {
            return version.serialize(serializer);
        }

        let field_count = usize::from(self.version.is_some()) + usize::from(!self.extensions.is_empty());
        let mut map = serializer.serialize_map(Some(field_count))?;
        if let Some(version) = &self.version {
            map.serialize_entry("version", version)?;
        }
        if !self.extensions.is_empty() {
            map.serialize_entry("extensions", &self.extensions)?;
        }
        map.end()
    }
}
```

Update the public export in `crates/config/src/lib.rs`:

```rust
pub use model::{AllocationConfig, PhpConfig, ProjectConfig, ProjectConfigFile, ResourceConfig};
```

- [ ] **Step 4: Implement PHP object parsing**

In `crates/config/src/parser.rs`, import `PhpConfig` and replace `php_track` with this shape:

```rust
fn php_config(value: &Value) -> Result<PhpConfig, ConfigError> {
    match value {
        Value::Mapping(mapping) => php_config_mapping(mapping),
        value => php_track(value).map(PhpConfig::version),
    }
}

fn php_config_mapping(mapping: &Mapping) -> Result<PhpConfig, ConfigError> {
    let mut config = PhpConfig::default();

    for (key, value) in mapping {
        let key = string_key_ref(key)?;
        match key.as_str() {
            "version" => {
                config.version = Some(php_track_field("php.version", value)?);
            }
            "extensions" => {
                config.extensions = php_extensions(value)?;
            }
            _ => {
                return Err(ConfigError::UnknownPhpKey { key });
            }
        }
    }

    Ok(config)
}

fn php_track_field(field: &str, value: &Value) -> Result<String, ConfigError> {
    let track = non_empty_string_or_number(field, value)?;
    TrackSelector::parse(track.clone()).map_err(|source| ConfigError::InvalidPhpTrack {
        track: track.clone(),
        reason: source.to_string(),
    })?;

    Ok(track)
}

fn php_extensions(value: &Value) -> Result<Vec<String>, ConfigError> {
    let sequence = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Sequence(sequence) => sequence,
        value => {
            return Err(ConfigError::InvalidFieldType {
                field: "php.extensions".to_string(),
                expected: "a sequence",
                found: value_type(value),
            });
        }
    };

    sequence
        .iter()
        .map(|value| non_empty_string("php.extensions", value))
        .collect()
}
```

Change the top-level parser branch to:

```rust
"php" => {
    config.php = Some(php_config(&value)?);
}
```

Add the error variant to `crates/config/src/error.rs`:

```rust
#[error("unknown Project config key `php.{key}`")]
UnknownPhpKey { key: String },
```

- [ ] **Step 5: Preserve extensions in `php:use` writer**

In `crates/config/src/writer.rs`, replace direct assignment with:

```rust
let php = config_file
    .config
    .php
    .get_or_insert_with(config::PhpConfig::default);
php.version = Some(track.to_string());
```

Use `crate::PhpConfig::default` if the module cannot refer to `config::PhpConfig` internally.

- [ ] **Step 6: Run parser and writer tests**

Run:

```bash
cargo nextest run -p config -E 'test(project_config_accepts_php_object_with_version_and_extensions) or test(project_config_accepts_php_object_with_extensions_only) or test(project_config_rejects_invalid_php_extensions_shape) or test(project_config_writer_updates_php_in_discovered_file)'
```

Expected: PASS.

- [ ] **Step 7: Commit Task 1**

```bash
git add crates/config/src/model.rs crates/config/src/parser.rs crates/config/src/writer.rs crates/config/src/lib.rs crates/config/src/error.rs crates/config/tests/project_config.rs
git commit -m "feat(config): parse PHP extension requests"
```

## Task 2: PHP Extension Metadata And Overlay Helpers

**Files:**
- Create: `crates/resources/src/php_extensions.rs`
- Modify: `crates/resources/src/lib.rs`
- Modify: `crates/resources/src/manifest.rs`
- Modify: `crates/resources/src/runtime.rs`
- Test: `crates/resources/tests/php_extensions.rs`
- Test: `crates/resources/tests/manifest_foundation.rs`

**Interfaces:**
- Consumes: `PvPaths`, installed artifact root paths, and requested extension names.
- Produces: `PhpExtensionModule`, `PhpExtensionLoadKind`, `PhpExtensionResolution`, `resolve_php_extension_request`, `ensure_php_runtime_overlay`, `php_runtime_environment`.

- [ ] **Step 1: Write failing PHP extension metadata tests**

Create `crates/resources/tests/php_extensions.rs`:

```rust
use anyhow::Result;
use camino_tempfile::tempdir;
use resources::{
    PhpExtensionLoadKind, ensure_php_runtime_overlay, php_runtime_environment,
    resolve_php_extension_request,
};
use state::{PvPaths, fs};

#[test]
fn resolves_available_and_ignored_php_extensions_from_artifact_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact = tempdir.path().join("php");
    fs::write_sensitive_file(
        &artifact.join("share/pv/php-extensions.json"),
        r#"
[
  {"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"},
  {"name":"xdebug","load_kind":"zend_extension","path":"lib/php/extensions/xdebug.so"}
]
"#,
    )?;

    let resolution =
        resolve_php_extension_request(&artifact, &["xdebug".into(), "missing".into(), "redis".into()])?;

    assert_eq!(resolution.requested, ["xdebug", "missing", "redis"]);
    assert_eq!(
        resolution
            .loaded
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>(),
        ["redis", "xdebug"]
    );
    assert_eq!(resolution.loaded[1].load_kind, PhpExtensionLoadKind::ZendExtension);
    assert_eq!(resolution.ignored, ["missing"]);

    Ok(())
}

#[test]
fn writes_runtime_overlay_for_loaded_php_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let artifact = tempdir.path().join("php");
    fs::write_sensitive_file(
        &artifact.join("share/pv/php-extensions.json"),
        r#"
[
  {"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"},
  {"name":"xdebug","load_kind":"zend_extension","path":"lib/php/extensions/xdebug.so"}
]
"#,
    )?;
    let resolution = resolve_php_extension_request(&artifact, &["redis".into(), "xdebug".into()])?;

    let overlay = ensure_php_runtime_overlay(&paths, "8.4+redis+xdebug", &artifact, &resolution.loaded)?;
    let redis_ini = fs::read_to_string(&overlay.join("10-redis.ini"))?;
    let xdebug_ini = fs::read_to_string(&overlay.join("20-xdebug.ini"))?;
    let env = php_runtime_environment(&paths, "8.4", "8.4+redis+xdebug", &artifact, &resolution.loaded)?;

    assert!(redis_ini.contains("extension="));
    assert!(redis_ini.contains("redis.so"));
    assert!(xdebug_ini.contains("zend_extension="));
    assert!(xdebug_ini.contains("xdebug.so"));
    assert!(env["PHP_INI_SCAN_DIR"].contains("conf.d"));
    assert!(env["PHP_INI_SCAN_DIR"].contains("php-runtimes/8.4+redis+xdebug/conf.d"));

    Ok(())
}
```

- [ ] **Step 2: Run metadata tests to verify failure**

Run:

```bash
cargo nextest run -p resources -E 'test(resolves_available_and_ignored_php_extensions_from_artifact_metadata) or test(writes_runtime_overlay_for_loaded_php_extensions)'
```

Expected: FAIL to compile because the exported helper types and functions do not exist.

- [ ] **Step 3: Implement `php_extensions.rs`**

Create `crates/resources/src/php_extensions.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

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
    if !fs::path_entry_exists(&path)? {
        return Ok(Vec::new());
    }

    let source = fs::read_to_string(&path)?;
    let raw = serde_json::from_str::<Vec<RawPhpExtensionModule>>(&source)
        .map_err(|error| ResourcesError::InvalidArtifactLayout {
            resource: "php".to_string(),
            reason: format!("invalid PHP extension metadata: {error}"),
        })?;

    raw.into_iter().map(PhpExtensionModule::from_raw).collect()
}

pub fn resolve_php_extension_request(
    artifact_root: &Utf8Path,
    requested: &[String],
) -> Result<PhpExtensionResolution> {
    let mut catalog = BTreeMap::new();
    for module in read_php_extension_metadata(artifact_root)? {
        catalog.insert(module.name.clone(), module);
    }

    let requested = requested.to_vec();
    let mut requested_unique = BTreeSet::new();
    let mut loaded = BTreeSet::new();
    let mut ignored = Vec::new();

    for name in &requested {
        if !requested_unique.insert(name.clone()) {
            continue;
        }
        if let Some(module) = catalog.get(name) {
            loaded.insert(module.clone());
        } else {
            ignored.push(name.clone());
        }
    }

    Ok(PhpExtensionResolution {
        requested,
        loaded: loaded.into_iter().collect(),
        ignored,
    })
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
    fs::ensure_user_dir(&conf_dir)?;

    for (index, module) in modules.iter().enumerate() {
        let prefix = 10 + (index * 10);
        let directive = match module.load_kind {
            PhpExtensionLoadKind::Extension => "extension",
            PhpExtensionLoadKind::ZendExtension => "zend_extension",
        };
        let module_path = artifact_root.join(&module.relative_path);
        let ini = format!("{directive}={module_path}\n");
        fs::write_sensitive_file(&conf_dir.join(format!("{prefix}-{}.ini", module.name)), &ini)?;
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
    let mut environment = php_track_environment(paths, track)?;
    if !modules.is_empty() {
        let overlay = ensure_php_runtime_overlay(paths, runtime_key, artifact_root, modules)?;
        if let Some(scan_dir) = environment.get_mut("PHP_INI_SCAN_DIR") {
            scan_dir.push(':');
            scan_dir.push_str(overlay.as_str());
        }
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
    Ok(php_runtime_environment(paths, track, runtime_key, artifact_root, modules)?
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect())
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
    if path.is_absolute() || path.components().any(|component| component.as_str() == "..") {
        return Err(ResourcesError::InvalidArtifactLayout {
            resource: "php".to_string(),
            reason: format!("invalid PHP extension path `{path}`"),
        });
    }

    Ok(path)
}
```

- [ ] **Step 4: Export helpers and parse manifest metadata**

In `crates/resources/src/lib.rs`:

```rust
pub mod php_extensions;
pub use php_extensions::{
    PHP_EXTENSION_METADATA_PATH, PhpExtensionLoadKind, PhpExtensionModule, PhpExtensionResolution,
    ensure_php_runtime_overlay, php_runtime_environment, php_runtime_exec_environment,
    read_php_extension_metadata, resolve_php_extension_request,
};
```

In `crates/resources/src/manifest.rs`, add a defaulted raw field and public accessor:

```rust
#[derive(Debug, Deserialize)]
struct RawArtifact {
    artifact_version: String,
    upstream_version: String,
    pv_build_revision: String,
    platform: String,
    url: String,
    sha256: String,
    size: u64,
    published_at: String,
    #[serde(default)]
    php_extensions: Vec<RawManifestPhpExtension>,
    #[serde(default)]
    revoked: bool,
    #[serde(default)]
    revocation_reason: Option<String>,
}
```

Add manifest module structs mirroring `PhpExtensionModule`, then store `php_extensions: Vec<PhpExtensionModule>` on `ManifestArtifact` and expose:

```rust
pub fn php_extensions(&self) -> &[PhpExtensionModule] {
    &self.php_extensions
}
```

- [ ] **Step 5: Run resources tests**

Run:

```bash
cargo nextest run -p resources -E 'test(resolves_available_and_ignored_php_extensions_from_artifact_metadata) or test(writes_runtime_overlay_for_loaded_php_extensions) or test(manifest_parses_registry_backed_resources_tracks_and_artifacts)'
```

Expected: PASS.

- [ ] **Step 6: Commit Task 2**

```bash
git add crates/resources/src/php_extensions.rs crates/resources/src/lib.rs crates/resources/src/manifest.rs crates/resources/src/runtime.rs crates/resources/tests/php_extensions.rs crates/resources/tests
git commit -m "feat(resources): add PHP extension metadata helpers"
```

## Task 3: Release Metadata And PHP Artifact Recipe Split

**Files:**
- Modify: `release/artifacts/recipes/php/tracks.toml`
- Modify: `release/artifacts/recipes/php/build.sh`
- Modify: `release/artifacts/recipes/php/smoke.sh`
- Modify: `crates/pv-release/src/recipe.rs`
- Modify: `crates/pv-release/src/record.rs`
- Modify: `crates/pv-release/src/record_writer.rs`
- Modify: `crates/pv-release/src/manifest.rs`
- Test: `crates/pv-release/tests/recipe_metadata.rs`
- Test: `crates/pv-release/tests/release_records.rs`
- Test: `crates/pv-release/tests/manifest_generation.rs`
- Test: `crates/pv-release/tests/smoke.rs`

**Interfaces:**
- Consumes: `PhpExtensionModule` JSON schema from Task 2.
- Produces: `php.default_extensions`, `php.optional_extensions`, `PV_DEFAULT_EXTENSIONS`, `PV_OPTIONAL_EXTENSIONS`, and manifest `php_extensions`.

- [ ] **Step 1: Write failing recipe metadata test**

In `crates/pv-release/tests/recipe_metadata.rs`, add:

```rust
#[test]
fn php_recipe_splits_default_and_optional_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let php = write_php_recipe(&tempdir)?;
    let env = php_recipe_env(&php, "php", "8.4", "darwin-arm64")?;

    assert!(env.contains("PV_DEFAULT_EXTENSIONS='bcmath,curl,intl,mbstring,openssl,pcntl,pdo_mysql,pdo_pgsql,pdo_sqlite,sodium,zip'"));
    assert!(env.contains("PV_OPTIONAL_EXTENSIONS='redis,sqlsrv,pdo_sqlsrv,xdebug,apcu,pcov,imagick,mongodb,yaml'"));
    assert!(env.contains("PV_EXPECTED_EXTENSIONS='bcmath,ctype,curl"));
    assert!(!env.contains("PV_BUILD_EXTENSIONS='"));

    Ok(())
}
```

- [ ] **Step 2: Run recipe metadata test to verify failure**

Run:

```bash
cargo nextest run -p pv-release -E 'test(php_recipe_splits_default_and_optional_extensions)'
```

Expected: FAIL because the recipe still has `build_extensions` only.

- [ ] **Step 3: Update PHP recipe model**

In `crates/pv-release/src/recipe.rs`, replace `build_extensions` in `RawPhpSettings` and `PhpSettings`:

```rust
#[derive(Clone, Debug)]
pub struct PhpSettings {
    deployment_target: String,
    default_extensions: Vec<String>,
    optional_extensions: Vec<String>,
    expected_extensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhpSettings {
    deployment_target: String,
    default_extensions: Vec<String>,
    optional_extensions: Vec<String>,
    expected_extensions: Vec<String>,
}
```

Update `PhpSettings::from_raw` to validate both lists:

```rust
validate_expected_extensions(path, &raw.expected_extensions)?;
validate_build_extensions(path, &raw.default_extensions, &raw.expected_extensions)?;
validate_extension_list(path, "php.optional_extensions", &raw.optional_extensions)?;
```

Change `php_recipe_env` assignments:

```rust
let default_extensions = recipe.default_extensions().join(",");
let optional_extensions = recipe.optional_extensions().join(",");
let build_extensions = if optional_extensions.is_empty() {
    default_extensions.clone()
} else {
    format!("{default_extensions},{optional_extensions}")
};
```

Emit:

```rust
("PV_DEFAULT_EXTENSIONS", "default_extensions", default_extensions.as_str()),
("PV_OPTIONAL_EXTENSIONS", "optional_extensions", optional_extensions.as_str()),
("PV_BUILD_EXTENSIONS", "build_extensions", build_extensions.as_str()),
("PV_EXPECTED_EXTENSIONS", "expected_extensions", expected_extensions.as_str()),
```

- [ ] **Step 4: Update `tracks.toml` extension split**

In `release/artifacts/recipes/php/tracks.toml`, replace `[php].build_extensions` with:

```toml
default_extensions = [
  "bcmath",
  "ctype",
  "curl",
  "dom",
  "fileinfo",
  "filter",
  "iconv",
  "intl",
  "libxml",
  "mbstring",
  "openssl",
  "pcntl",
  "pdo",
  "pdo_mysql",
  "pdo_pgsql",
  "pdo_sqlite",
  "phar",
  "posix",
  "session",
  "simplexml",
  "sodium",
  "sqlite3",
  "tokenizer",
  "xml",
  "xmlreader",
  "xmlwriter",
  "zip",
  "zlib",
]
optional_extensions = [
  "redis",
  "sqlsrv",
  "pdo_sqlsrv",
  "xdebug",
  "apcu",
  "pcov",
  "imagick",
  "mongodb",
  "yaml",
]
```

Remove `redis`, `sqlsrv`, and `pdo_sqlsrv` from `expected_extensions`; keep implicit `hash`, `json`, and `pcre`.

- [ ] **Step 5: Carry extension metadata through records and manifests**

In `crates/pv-release/src/record.rs`, add defaulted fields:

```rust
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PhpExtensionRecord {
    name: String,
    load_kind: String,
    path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReleaseRecord {
    ...
    #[serde(default)]
    php_extensions: Vec<PhpExtensionRecord>,
    provenance: Provenance,
}
```

Expose:

```rust
pub fn php_extensions(&self) -> &[PhpExtensionRecord] {
    &self.php_extensions
}
```

In `crates/pv-release/src/record_writer.rs`, add `php_extensions` to the JSON request and generated record. In `crates/pv-release/src/manifest.rs`, serialize `php_extensions` on each `ManifestArtifactJson` with `#[serde(skip_serializing_if = "Vec::is_empty")]`.

- [ ] **Step 6: Update build script staging**

In `release/artifacts/recipes/php/build.sh`, build shared optional modules by changing the StaticPHP command:

```sh
optional_shared_args=
if [ -n "$PHP_OPTIONAL_EXTENSIONS" ]; then
  optional_shared_args="--build-shared=$PHP_OPTIONAL_EXTENSIONS"
fi

spc build:php "$PHP_BUILD_EXTENSIONS" \
  $optional_shared_args \
  --build-cli \
  --build-frankenphp \
  --enable-zts \
  --with-config-file-path=/var/empty/com.prvious.pv/php \
  --with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d \
  --dl-with-php="$PHP_PHP_VERSION" \
  --dl-retry=3 \
  --dl-custom-local "php-src:$php_source_dir" \
  --dl-custom-local "frankenphp:$frankenphp_source_dir"
```

After copying `bin/php` or `bin/frankenphp`, stage optional modules and metadata:

```sh
stage_optional_php_extensions() {
  root_dir=$1
  mkdir -p "$root_dir/lib/php/extensions" "$root_dir/share/pv"
  metadata="$root_dir/share/pv/php-extensions.json"
  printf '[' >"$metadata"
  first=1
  old_ifs=$IFS
  IFS=,
  for extension in $PHP_OPTIONAL_EXTENSIONS; do
    [ -n "$extension" ] || continue
    module=$(find "$spc_work_dir/buildroot" -type f -name "$extension.so" | head -n 1)
    [ -n "$module" ] || die "optional PHP extension $extension did not produce a shared module"
    cp "$module" "$root_dir/lib/php/extensions/$extension.so"
    load_kind=extension
    [ "$extension" = "xdebug" ] && load_kind=zend_extension
    [ "$first" -eq 1 ] || printf ',' >>"$metadata"
    first=0
    printf '{"name":"%s","load_kind":"%s","path":"lib/php/extensions/%s.so"}' "$extension" "$load_kind" "$extension" >>"$metadata"
  done
  IFS=$old_ifs
  printf ']\n' >>"$metadata"
}
```

Call it from `stage_artifact` after copying the binary:

```sh
stage_optional_php_extensions "$root_dir"
```

- [ ] **Step 7: Update PHP smoke hook**

In `release/artifacts/recipes/php/smoke.sh`, require default extensions only for the normal run. Add optional-module smoke:

```sh
check_optional_extensions() {
  metadata="$artifact_root/share/pv/php-extensions.json"
  [ -f "$metadata" ] || return 0
  need python3
  scan_dir=$(mktemp -d)
  python3 - "$metadata" "$artifact_root" "$scan_dir" <<'PY'
import json
import pathlib
import sys

metadata = pathlib.Path(sys.argv[1])
artifact_root = pathlib.Path(sys.argv[2])
scan_dir = pathlib.Path(sys.argv[3])
for index, module in enumerate(json.loads(metadata.read_text())):
    directive = module["load_kind"]
    path = artifact_root / module["path"]
    prefix = 10 + index * 10
    (scan_dir / f"{prefix}-{module['name']}.ini").write_text(f"{directive}={path}\n")
PY
  PHP_INI_SCAN_DIR="$scan_dir" check_extensions "$php_binary" -m
  rm -rf "$scan_dir"
}
```

Call `check_optional_extensions` after the normal PHP CLI extension check when `bin/php` is present.

- [ ] **Step 8: Run release tests**

Run:

```bash
cargo nextest run -p pv-release -E 'test(php_recipe_splits_default_and_optional_extensions) or test(release_record) or test(manifest_generator)'
```

Run shell syntax:

```bash
shellcheck release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh
```

Expected: PASS.

- [ ] **Step 9: Commit Task 3**

```bash
git add release/artifacts/recipes/php/tracks.toml release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh crates/pv-release/src crates/pv-release/tests
git commit -m "feat(release): bundle optional PHP extension metadata"
```

## Task 4: Persist PHP Runtime Extension State

**Files:**
- Create: `crates/state/src/sql/008_project_php_runtime_extensions.sql`
- Modify: `crates/state/src/migrations.rs`
- Modify: `crates/state/src/database.rs`
- Modify: `crates/state/src/lib.rs`
- Modify: `crates/state/src/paths.rs`
- Test: `crates/state/tests/state_foundation.rs`

**Interfaces:**
- Consumes: loaded extension names from runtime resolution.
- Produces: `ProjectPhpRuntimeInput`, `ProjectPhpRuntimeRecord`, `php_runtime_key`, runtime-key-safe worker subjects and ports.

- [ ] **Step 1: Write failing state tests**

Add tests in `crates/state/tests/state_foundation.rs`:

```rust
#[test]
fn project_php_runtime_extensions_round_trip_through_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("acme"),
        original_path: tempdir.path().join("acme"),
        primary_hostname: "acme.test".to_string(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: Some("8.4".to_string()),
        additional_hostnames: Vec::new(),
    })?;

    database.replace_project_php_runtime(
        &project.project.id,
        Some(&state::ProjectPhpRuntimeInput {
            track: "8.4".to_string(),
            requested_extensions: vec!["xdebug".to_string(), "redis".to_string()],
            loaded_extensions: vec!["redis".to_string(), "xdebug".to_string()],
            ignored_extensions: vec!["missing".to_string()],
        }),
    )?;

    let project = database
        .project_by_id(&project.project.id)?
        .ok_or_else(|| anyhow!("missing project"))?;

    assert_eq!(project.php_runtime.track.as_deref(), Some("8.4"));
    assert_eq!(project.php_runtime.loaded_extensions, ["redis", "xdebug"]);
    assert_eq!(project.php_runtime.ignored_extensions, ["missing"]);
    assert_eq!(
        state::php_runtime_key("8.4", &project.php_runtime.loaded_extensions)?,
        "8.4+redis+xdebug"
    );

    Ok(())
}

#[test]
fn php_worker_port_allocator_uses_runtime_identity() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let plain = database.assign_port(
        PortRequest::php_worker("8.4", 45000, 45000, 45009),
        |_port| true,
    )?;
    let redis = database.assign_port(
        PortRequest::php_worker("8.4+redis", 45000, 45000, 45009),
        |_port| true,
    )?;

    assert_ne!(plain.port, redis.port);
    assert_eq!(
        redis.owner,
        PortOwner::PhpWorker {
            php_track: "8.4+redis".to_string()
        }
    );

    Ok(())
}
```

- [ ] **Step 2: Run state tests to verify failure**

Run:

```bash
cargo nextest run -p state -E 'test(project_php_runtime_extensions_round_trip_through_state) or test(php_worker_port_allocator_uses_runtime_identity)'
```

Expected: FAIL to compile because runtime extension state APIs do not exist.

- [ ] **Step 3: Add migration 8**

Create `crates/state/src/sql/008_project_php_runtime_extensions.sql`:

```sql
ALTER TABLE projects ADD COLUMN desired_php_requested_extensions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE projects ADD COLUMN desired_php_loaded_extensions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE projects ADD COLUMN desired_php_ignored_extensions_json TEXT NOT NULL DEFAULT '[]';
```

Register in `crates/state/src/migrations.rs`:

```rust
const PROJECT_PHP_RUNTIME_EXTENSIONS_SQL: &str =
    include_str!("sql/008_project_php_runtime_extensions.sql");

Migration::new(
    8,
    "project_php_runtime_extensions",
    PROJECT_PHP_RUNTIME_EXTENSIONS_SQL,
),
```

- [ ] **Step 4: Add runtime state structs and JSON helpers**

In `crates/state/src/database.rs`, add:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectPhpRuntimeRecord {
    pub track: Option<String>,
    pub requested_extensions: Vec<String>,
    pub loaded_extensions: Vec<String>,
    pub ignored_extensions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPhpRuntimeInput {
    pub track: String,
    pub requested_extensions: Vec<String>,
    pub loaded_extensions: Vec<String>,
    pub ignored_extensions: Vec<String>,
}
```

Add `pub php_runtime: ProjectPhpRuntimeRecord` to `ProjectRecord`. Keep `desired_php_track` for compatibility and existing callers.

Add:

```rust
pub fn php_runtime_key(track: &str, loaded_extensions: &[String]) -> Result<String, StateError> {
    validate_project_php_track(track)?;
    for extension in loaded_extensions {
        validate_php_extension_identity(extension)?;
    }
    if loaded_extensions.is_empty() {
        return Ok(track.to_string());
    }

    Ok(format!("{track}+{}", loaded_extensions.join("+")))
}
```

Add validators for extension identities using ASCII alphanumeric plus `_`.

- [ ] **Step 5: Add `replace_project_php_runtime`**

In `impl Database`, add:

```rust
pub fn replace_project_php_runtime(
    &mut self,
    project_id: &str,
    runtime: Option<&ProjectPhpRuntimeInput>,
) -> Result<ProjectRecord, StateError> {
    let (track, requested, loaded, ignored) = match runtime {
        Some(runtime) => {
            validate_project_php_track(&runtime.track)?;
            validate_php_extension_list(&runtime.requested_extensions)?;
            validate_php_extension_list(&runtime.loaded_extensions)?;
            validate_php_extension_list(&runtime.ignored_extensions)?;
            (
                Some(runtime.track.as_str()),
                extension_json(&runtime.requested_extensions)?,
                extension_json(&runtime.loaded_extensions)?,
                extension_json(&runtime.ignored_extensions)?,
            )
        }
        None => (None, "[]".to_string(), "[]".to_string(), "[]".to_string()),
    };

    self.connection.execute(
        "UPDATE projects
         SET desired_php_track = ?2,
             desired_php_requested_extensions_json = ?3,
             desired_php_loaded_extensions_json = ?4,
             desired_php_ignored_extensions_json = ?5,
             updated_at = datetime('now')
         WHERE id = ?1",
        rusqlite::params![project_id, track, requested, loaded, ignored],
    )?;

    self.project_by_id(project_id)?.ok_or_else(|| StateError::ProjectNotFound {
        target: project_id.to_string(),
    })
}
```

Update `replace_project_desired_php_track` to call `replace_project_php_runtime` with empty extension lists.

- [ ] **Step 6: Allow PHP worker ports and subjects to use runtime keys**

Replace `validate_runtime_php_track` internals with runtime-key validation. `RuntimeSubject::PhpWorker { php_track }` can keep the existing field name for a smaller diff, but its value is now a runtime key.

Update error kind strings from `"php_track"` to `"php_runtime"` in new tests, then update snapshots.

- [ ] **Step 7: Run state tests**

Run:

```bash
cargo nextest run -p state -E 'test(project_php_runtime_extensions_round_trip_through_state) or test(php_worker_port_allocator_uses_runtime_identity) or test(runtime_observed_state_round_trips_through_observed_states) or test(database_runs_migrations_and_exposes_core_schema)'
```

Expected: PASS after accepting intentional snapshots:

```bash
cargo insta test --accept --test-runner nextest -p state -- state_foundation
```

- [ ] **Step 8: Commit Task 4**

```bash
git add crates/state/src/sql/008_project_php_runtime_extensions.sql crates/state/src/migrations.rs crates/state/src/database.rs crates/state/src/lib.rs crates/state/src/paths.rs crates/state/tests/state_foundation.rs crates/state/tests/snapshots
git commit -m "feat(state): persist PHP runtime extension identity"
```

## Task 5: Daemon Runtime Resolution And Worker Grouping

**Files:**
- Modify: `crates/daemon/src/project_env.rs`
- Modify: `crates/daemon/src/gateway.rs`
- Test: `crates/daemon/tests/project_env_reconciliation.rs`
- Test: `crates/daemon/tests/gateway_reconciliation.rs`

**Interfaces:**
- Consumes: `PhpConfig`, `ProjectPhpRuntimeInput`, `php_runtime_key`, `resolve_php_extension_request`, `php_runtime_environment`.
- Produces: Project reconciliation persists requested/loaded/ignored extensions; gateway groups by runtime key.

- [ ] **Step 1: Write failing project env reconciliation test**

In `crates/daemon/tests/project_env_reconciliation.rs`, add:

```rust
#[tokio::test]
async fn project_env_reconciliation_persists_php_extension_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        "php:\n  version: \"8.4\"\n  extensions: [redis, missing]\n",
    )?;
    let release = tempdir.path().join("php-release");
    state::fs::write_sensitive_file(&release.join("bin/php"), "#!/bin/sh\n")?;
    state::fs::write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    {
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_installed("php", "8.4", "8.4.8-pv1", &release)?;
    }

    run_project_reconciliation(&paths, &project).await?;

    let database = Database::open(&paths)?;
    let project = database
        .project_by_id(&project.id)?
        .ok_or_else(|| anyhow!("expected linked project"))?;
    let observed = database
        .project_env_observed_state(&project.id)?
        .ok_or_else(|| anyhow!("expected observed project env state"))?;

    assert_eq!(project.php_runtime.track.as_deref(), Some("8.4"));
    assert_eq!(project.php_runtime.requested_extensions, ["redis", "missing"]);
    assert_eq!(project.php_runtime.loaded_extensions, ["redis"]);
    assert_eq!(project.php_runtime.ignored_extensions, ["missing"]);
    assert_eq!(observed.status, ProjectEnvObservedStatus::Warning);
    assert_eq!(observed.warnings[0].kind, "ignored_php_extension");

    Ok(())
}
```

- [ ] **Step 2: Write failing gateway grouping test**

In `crates/daemon/tests/gateway_reconciliation.rs`, add a plan-level test near existing `build_runtime_plan` coverage:

```rust
#[test]
fn gateway_runtime_plan_groups_projects_by_php_track_and_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let acme = create_project_with_config(
        tempdir.path(),
        "acme",
        "php:\n  version: 8.4\n  extensions: [redis]\n",
    )?;
    let api = create_project_with_config(
        tempdir.path(),
        "api",
        "php:\n  version: 8.4\n  extensions: [xdebug, redis]\n",
    )?;
    let release = seed_installed_php_with_extensions(&paths, "8.4", &["redis", "xdebug"])?;
    seed_installed_frankenphp_with_extensions(&paths, "8.4", &release, &["redis", "xdebug"])?;
    link_project_record(&paths, &acme, "acme.test", Some("8.4"))?;
    link_project_record(&paths, &api, "api.test", Some("8.4"))?;

    let plan = daemon::gateway::build_runtime_plan(&paths)?;
    let runtime_keys = plan
        .workers
        .iter()
        .map(|worker| worker.runtime_key.as_str())
        .collect::<Vec<_>>();

    assert_eq!(runtime_keys, ["8.4+redis", "8.4+redis+xdebug"]);

    Ok(())
}
```

If the helper names in this snippet do not exist in the file, implement them locally next to the existing test helpers using the same fixture style already used in `gateway_reconciliation.rs`.

- [ ] **Step 3: Run daemon tests to verify failure**

Run:

```bash
cargo nextest run -p daemon -E 'test(project_env_reconciliation_persists_php_extension_runtime) or test(gateway_runtime_plan_groups_projects_by_php_track_and_extensions)'
```

Expected: FAIL to compile because daemon runtime resolution still reads only `config.php.as_deref()` and `PhpWorkerRuntimePlan` has no `runtime_key`.

- [ ] **Step 4: Add runtime resolver in `project_env.rs`**

Add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedPhpRuntime {
    pub(crate) track: String,
    pub(crate) runtime_key: String,
    pub(crate) requested_extensions: Vec<String>,
    pub(crate) loaded_extensions: Vec<String>,
    pub(crate) ignored_extensions: Vec<String>,
    pub(crate) loaded_modules: Vec<resources::PhpExtensionModule>,
}
```

Implement:

```rust
pub(crate) fn resolve_project_php_runtime(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    php: Option<&config::PhpConfig>,
) -> Result<ResolvedPhpRuntime, DaemonError> {
    let selector = php.and_then(config::PhpConfig::version_selector);
    let track = resolve_project_php_track(paths, selector, project.desired_php_track.as_deref())?;
    let requested_extensions = php
        .map(|php| php.requested_extensions().to_vec())
        .unwrap_or_default();
    let release = installed_php_release(database, &track);
    let resolution = match release {
        Some(release) => resources::resolve_php_extension_request(&release, &requested_extensions)?,
        None => resources::PhpExtensionResolution {
            requested: requested_extensions.clone(),
            loaded: Vec::new(),
            ignored: requested_extensions.clone(),
        },
    };
    let loaded_extensions = resolution
        .loaded
        .iter()
        .map(|module| module.name.clone())
        .collect::<Vec<_>>();
    let runtime_key = state::php_runtime_key(&track, &loaded_extensions)?;

    Ok(ResolvedPhpRuntime {
        track,
        runtime_key,
        requested_extensions: resolution.requested,
        loaded_extensions,
        ignored_extensions: resolution.ignored,
        loaded_modules: resolution.loaded,
    })
}
```

Add `installed_php_release` by scanning `database.managed_resource_tracks()?` for resource `php`, matching track, desired installed state, installed version, and current artifact path.

- [ ] **Step 5: Persist runtime and ignored-extension warnings**

In `reconcile_loaded_project`, replace `resolved_project_php_track_for_state` usage with `resolve_project_php_runtime` and write:

```rust
database.replace_project_php_runtime(
    &project.id,
    Some(&ProjectPhpRuntimeInput {
        track: runtime.track.clone(),
        requested_extensions: runtime.requested_extensions.clone(),
        loaded_extensions: runtime.loaded_extensions.clone(),
        ignored_extensions: runtime.ignored_extensions.clone(),
    }),
)?;
```

Build warning inputs:

```rust
fn ignored_php_extension_warnings(runtime: &ResolvedPhpRuntime) -> Vec<ProjectEnvObservedWarningInput> {
    runtime
        .ignored_extensions
        .iter()
        .map(|extension| ProjectEnvObservedWarningInput {
            kind: "ignored_php_extension".to_string(),
            message: format!("ignored unsupported PHP extension `{extension}`"),
        })
        .collect()
}
```

Merge these warnings with existing `.env` warnings. When there are no env mappings but extension warnings exist, record `ProjectEnvObservedStatus::Warning` with message `"Project runtime has warnings"`.

- [ ] **Step 6: Group gateway workers by runtime key**

In `crates/daemon/src/gateway.rs`, change `PhpWorkerRuntimePlan`:

```rust
pub struct PhpWorkerRuntimePlan {
    pub php_track: String,
    pub runtime_key: String,
    pub loaded_modules: Vec<resources::PhpExtensionModule>,
    pub port: u16,
    pub projects: Vec<RuntimeProject>,
}
```

Replace `projects_by_php_track` with `projects_by_runtime_key`. In valid config flow, call `resolve_project_php_runtime`; in invalid config fallback, use `project.php_runtime` fields and `state::php_runtime_key`.

Update `append_runtime_project` to assign ports with:

```rust
PortRequest::php_worker(
    &runtime.runtime_key,
    RUNTIME_PORT_FALLBACK_START,
    RUNTIME_PORT_FALLBACK_START,
    RUNTIME_PORT_FALLBACK_END,
)
```

- [ ] **Step 7: Use runtime overlays for workers**

Change worker path calls to use `worker.runtime_key`. Change worker environment:

```rust
fn frankenphp_worker_environment(
    paths: &PvPaths,
    worker: &PhpWorkerRuntimePlan,
    artifact_root: &Utf8Path,
) -> Result<BTreeMap<String, String>, StateError> {
    let mut environment = frankenphp_xdg_environment(paths);
    environment.extend(resources::php_runtime_environment(
        paths,
        &worker.php_track,
        &worker.runtime_key,
        artifact_root,
        &worker.loaded_modules,
    )?);

    Ok(environment)
}
```

Use the installed FrankenPHP release path as `artifact_root` when preparing the worker process spec.

- [ ] **Step 8: Run daemon tests**

Run:

```bash
cargo nextest run -p daemon -E 'test(project_env_reconciliation_persists_php_extension_runtime) or test(gateway_runtime_plan_groups_projects_by_php_track_and_extensions) or test(gateway_reconciliation_preserves_last_valid_runtime_when_project_config_breaks)'
```

Expected: PASS after updating snapshots:

```bash
cargo insta test --accept --test-runner nextest -p daemon -- gateway_reconciliation
```

- [ ] **Step 9: Commit Task 5**

```bash
git add crates/daemon/src/project_env.rs crates/daemon/src/gateway.rs crates/daemon/tests/project_env_reconciliation.rs crates/daemon/tests/gateway_reconciliation.rs crates/daemon/tests/snapshots
git commit -m "feat(daemon): group PHP workers by extension runtime"
```

## Task 6: PHP And Composer Shim Runtime Overlays

**Files:**
- Modify: `crates/cli/src/commands/php.rs`
- Modify: `crates/cli/src/commands/composer.rs`
- Test: `crates/cli/tests/php.rs`
- Test: `crates/cli/tests/composer.rs`

**Interfaces:**
- Consumes: `ProjectRecord.php_runtime`, `state::php_runtime_key`, `resources::php_runtime_exec_environment`.
- Produces: PHP and Composer shims using the Project runtime overlay inside linked Projects.

- [ ] **Step 1: Write failing PHP shim test**

In `crates/cli/tests/php.rs`, add:

```rust
#[test]
fn php_shim_uses_project_extension_runtime_overlay() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php:\n  version: 8.4\n  extensions: [redis]\n")?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    fs::write_sensitive_file(
        &release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.4".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: vec!["redis".to_string()],
                ignored_extensions: Vec::new(),
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:php", "-m"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(exec_calls[0]
        .env
        .iter()
        .any(|(key, value)| key == "PHP_INI_SCAN_DIR" && value.contains("php-runtimes/8.4+redis/conf.d")));

    Ok(())
}
```

- [ ] **Step 2: Run shim test to verify failure**

Run:

```bash
cargo nextest run -p cli -E 'test(php_shim_uses_project_extension_runtime_overlay)'
```

Expected: FAIL because the PHP shim only uses `php_track_exec_environment`.

- [ ] **Step 3: Resolve shim runtime from Project state**

In `crates/cli/src/commands/php.rs`, replace `resolve_php_track_for_shim` with `resolve_php_runtime_for_shim` returning:

```rust
struct PhpShimRuntime {
    track: String,
    runtime_key: String,
    loaded_extensions: Vec<String>,
}
```

Inside linked Projects:

```rust
if let Some(project) = database.nearest_project_for_path(&current_dir)?
    && let Some(track) = project.php_runtime.track.clone()
{
    let runtime_key = state::php_runtime_key(&track, &project.php_runtime.loaded_extensions)?;
    return Ok(PhpShimRuntime {
        track,
        runtime_key,
        loaded_extensions: project.php_runtime.loaded_extensions,
    });
}
```

Outside Projects, return the global/default track with `runtime_key = track` and an empty extension list.

- [ ] **Step 4: Build shim env from installed artifact metadata**

In `shim_with_args_and_env`, after finding the installed PHP record:

```rust
let requested = runtime.loaded_extensions.clone();
let resolution = resources::resolve_php_extension_request(installed.release_path(), &requested)?;
env.extend(resources::php_runtime_exec_environment(
    &paths,
    &runtime.track,
    &runtime.runtime_key,
    installed.release_path(),
    &resolution.loaded,
)?);
```

Keep `resources::ensure_php_track_defaults(&paths, &runtime.track)?` before generating env.

- [ ] **Step 5: Add Composer shim test**

In `crates/cli/tests/composer.rs`, add a test mirroring the PHP shim test:

```rust
#[test]
fn composer_shim_inherits_project_php_extension_runtime_overlay() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let project_record = register_project(&home, &project, "acme.test")?;
    let php_release = record_installed_php(&home, "8.4", "8.4.8-pv1")?;
    let composer_phar = record_installed_composer(&home)?;
    fs::write_sensitive_file(
        &php_release.join("share/pv/php-extensions.json"),
        r#"[{"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}]"#,
    )?;
    {
        let mut database = Database::open(&pv_paths(&home))?;
        database.replace_project_php_runtime(
            &project_record.id,
            Some(&state::ProjectPhpRuntimeInput {
                track: "8.4".to_string(),
                requested_extensions: vec!["redis".to_string()],
                loaded_extensions: vec!["redis".to_string()],
                ignored_extensions: Vec::new(),
            }),
        )?;
    }
    let environment = TestEnvironment::new(&home, &project_record.path, ScriptedClient::new());

    let output = run_pv(&["shim:composer", "about"], &environment)?;
    let exec_calls = environment.exec_calls();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(exec_calls[0].args[0], composer_phar.to_string());
    assert!(exec_calls[0]
        .env
        .iter()
        .any(|(key, value)| key == "PHP_INI_SCAN_DIR" && value.contains("php-runtimes/8.4+redis/conf.d")));

    Ok(())
}
```

- [ ] **Step 6: Run CLI shim tests**

Run:

```bash
cargo nextest run -p cli -E 'test(php_shim_uses_project_extension_runtime_overlay) or test(composer_shim_inherits_project_php_extension_runtime_overlay) or test(php_shim_execs_resolved_project_track) or test(composer_shim_execs_installed_composer_through_php)'
```

Expected: PASS after updating snapshots:

```bash
cargo insta test --accept --test-runner nextest -p cli -- php_shim composer_shim
```

- [ ] **Step 7: Commit Task 6**

```bash
git add crates/cli/src/commands/php.rs crates/cli/src/commands/composer.rs crates/cli/tests/php.rs crates/cli/tests/composer.rs crates/cli/tests/snapshots
git commit -m "feat(cli): load PHP extension overlays in shims"
```

## Task 7: CLI Diagnostics For Ignored Extensions

**Files:**
- Modify: `crates/cli/src/commands/project.rs`
- Modify: `crates/cli/src/commands/status.rs`
- Test: `it/cli.rs`
- Test: `crates/cli/tests/status.rs`

**Interfaces:**
- Consumes: `ProjectEnvObservedWarningRecord` with kind `ignored_php_extension`.
- Produces: user-visible ignored-extension warnings in Project list/status output.

- [ ] **Step 1: Write failing integration list test**

In `it/cli.rs`, add:

```rust
#[test]
fn project_list_reports_ignored_php_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php:\n  extensions: [redis, missing]\n")?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let paths = PvPaths::for_home(home.clone());
    let mut database = Database::open(&paths)?;
    let linked_project = database
        .projects()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Warning,
        Some("Project runtime has warnings"),
        &[ProjectEnvObservedWarningInput {
            kind: "ignored_php_extension".to_string(),
            message: "ignored unsupported PHP extension `missing`".to_string(),
        }],
    )?;

    let list = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list));
    });

    Ok(())
}
```

- [ ] **Step 2: Run list test to verify failure or snapshot drift**

Run:

```bash
cargo nextest run --test cli -E 'test(project_list_reports_ignored_php_extensions)'
```

Expected: FAIL with a new snapshot or missing warning detail.

- [ ] **Step 3: Add compact warning detail**

In `crates/cli/src/commands/project.rs`, extend `project_env_observed_warning_summary`:

```rust
fn project_env_observed_warning_summary(observed: &ProjectEnvObservedStateRecord) -> String {
    let ignored = observed
        .warnings
        .iter()
        .filter(|warning| warning.kind == "ignored_php_extension")
        .map(|warning| warning.message.as_str())
        .collect::<Vec<_>>();
    if ignored.len() == 1 {
        return format!("warning: {}", ignored[0]);
    }
    if ignored.len() > 1 {
        return format!("warning: {} ignored PHP extensions", ignored.len());
    }

    match observed.warnings.as_slice() {
        [warning] => format!("warning: {}", warning.message),
        [] => observed
            .message
            .as_ref()
            .map(|message| format!("warning: {message}"))
            .unwrap_or_else(|| "warning".to_string()),
        warnings => format!("warning: {} warnings", warnings.len()),
    }
}
```

- [ ] **Step 4: Add status JSON/plain visibility**

In `crates/cli/src/commands/status.rs`, include warning text in `ProjectStatus.message` when status is warning:

```rust
let message = if observed.status == ProjectEnvObservedStatus::Warning {
    observed
        .warnings
        .first()
        .map(|warning| warning.message.clone())
        .or(observed.message)
} else {
    observed.message
};
```

- [ ] **Step 5: Run diagnostics tests**

Run:

```bash
cargo insta test --accept --test-runner nextest --test cli -- project_list_reports_ignored_php_extensions
cargo nextest run -p cli -E 'test(status_reports_warning_project_env_as_success)'
```

Expected: PASS.

- [ ] **Step 6: Commit Task 7**

```bash
git add crates/cli/src/commands/project.rs crates/cli/src/commands/status.rs it/cli.rs it/snapshots crates/cli/tests/status.rs crates/cli/tests/snapshots
git commit -m "feat(cli): report ignored PHP extensions"
```

## Task 8: Final Verification And Documentation Sweep

**Files:**
- Modify if needed: `docs/user/README.md`
- Modify if needed: `DESIGN.md`
- Modify if needed: `release/artifacts/README.md`

**Interfaces:**
- Consumes: all completed implementation tasks.
- Produces: verified feature branch and user-facing docs updates.

- [ ] **Step 1: Update user docs with config examples**

Add this section to `docs/user/README.md` near Project config documentation:

```markdown
### PHP Extensions

The `php` key may be a scalar version or an object:

```yaml
php:
  version: 8.4
  extensions:
    - redis
    - xdebug
```

If `version` is omitted, PV uses the configured default PHP track:

```yaml
php:
  extensions:
    - xdebug
```

PV loads bundled optional extensions that are available in the installed PHP artifact. Unknown extension names are ignored and reported as warnings.
```

- [ ] **Step 2: Run focused test suite**

Run:

```bash
cargo nextest run -p config -p resources -p state -p daemon -p cli -E 'test(php) or test(extension) or test(gateway_runtime_plan) or test(project_env_reconciliation)'
```

Expected: PASS.

- [ ] **Step 3: Run release tooling tests**

Run:

```bash
cargo nextest run -p pv-release -E 'test(php_recipe) or test(release_record) or test(manifest_generator) or test(php_smoke)'
```

Expected: PASS.

- [ ] **Step 4: Run formatting**

Run:

```bash
cargo fmt --all
```

Expected: no output and exit 0.

- [ ] **Step 5: Run clippy**

Run:

```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Run shellcheck**

Run:

```bash
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh
```

Expected: PASS.

- [ ] **Step 7: Commit final docs and cleanup**

```bash
git add docs/user/README.md DESIGN.md release/artifacts/README.md
git commit -m "docs: document PHP extension opt-ins"
```

If no docs changed in this task, skip the commit and record that all required docs were already current.

- [ ] **Step 8: Inspect final branch state**

Run:

```bash
git status --short
git log --oneline --max-count=8
```

Expected: working tree clean, with one commit per completed task.

## Plan Self-Review

- Spec coverage: covered config shape, curated default/optional split, artifact metadata, no arbitrary `.so`, runtime grouping, CLI/Composer parity, unsupported-name warnings, state fallback, release recipe smoke tests, and docs.
- Placeholder scan: clean for blocked-work markers and unresolved placeholders.
- Type consistency: `PhpConfig`, `PhpExtensionModule`, `PhpExtensionResolution`, `ProjectPhpRuntimeInput`, and `php_runtime_key` are introduced before dependent tasks consume them.
- Scope check: artifact packaging and app runtime are both required for the feature to work end-to-end; they are split into independently testable tasks inside one implementation plan.
