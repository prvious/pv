# PR 15 PHP Shim And Composer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Project-aware PHP shims, `pv php:*` commands, and Composer track `2` commands from the approved PR 15 design.

**Architecture:** Keep user-facing behavior in `cli`, persisted global PHP default in `state`, structured Project config mutation in `config`, and artifact lifecycle logic in `resources`. Explicit PV management commands install missing artifacts; direct `php` and `composer` shim execution fails clearly when required artifacts are missing.

**Tech Stack:** Rust 2024, Clap, rusqlite, yaml_serde, insta snapshots, cargo nextest, existing PV `state`, `config`, `resources`, `daemon`, and `cli` crates.

---

## Reference Documents

- Spec: `docs/superpowers/specs/2026-06-07-pr-15-php-shim-composer-design.md`
- Roadmap row: `IMPLEMENTATION.md` PR 15
- Current command contract to update: `DESIGN.md` PHP and Composer sections
- Contributor rules: `CONTRIBUTING.md`

## File Structure

- Modify `crates/state/src/sql/006_global_php_default.sql`: new migration storing the global PHP default track.
- Modify `crates/state/src/migrations.rs`: register migration 6.
- Modify `crates/state/src/database.rs`: add `global_php_default_track()` and `set_global_php_default_track()`.
- Modify `crates/state/src/error.rs`: reuse `ReservedConcreteTrack` for `latest`; no new error is required.
- Modify `crates/state/tests/state_foundation.rs`: add state persistence and validation tests.
- Create `crates/config/src/writer.rs`: update or create `php:` while preserving known semantic fields.
- Modify `crates/config/src/lib.rs`: export the writer helper.
- Modify `crates/config/tests/project_config.rs`: add Project config mutation tests.
- Modify `crates/resources/src/runtime.rs`: add `composer_adapter()` and stable executable/PHAR path helpers.
- Modify `crates/resources/src/command.rs`: add PHP pair and Composer command helpers around existing `ManagedResourceCommands`.
- Modify `crates/resources/src/lib.rs`: export new adapters/helpers.
- Modify `crates/resources/tests/managed_resource_commands.rs`: add paired PHP/FrankenPHP and Composer command coverage.
- Modify `crates/resources/tests/runtime_adapters.rs`: add Composer artifact layout tests.
- Modify `crates/cli/src/args.rs`: add `php:use`, `php:update`, `php:uninstall`, `php:list`, `composer:*`, and hidden shim entrypoints.
- Modify `crates/cli/src/commands/mod.rs`: route new commands.
- Replace `crates/cli/src/commands/php.rs`: implement PHP management commands and shim behavior.
- Create `crates/cli/src/commands/composer.rs`: implement Composer commands and shim behavior.
- Modify `crates/cli/src/error.rs`: add clear missing-install and command-layer errors.
- Modify `crates/cli/src/environment.rs`: add process exec and argument/env hooks for shim tests.
- Modify `it/cli.rs` and snapshots: add binary-level command/help coverage.
- Modify `DESIGN.md`: replace `php:default` with `php:use --global`, add Project-level `php:use`, and keep command tables consistent.

---

### Task 1: Persist Global PHP Default In `pv.db`

**Files:**
- Create: `crates/state/src/sql/006_global_php_default.sql`
- Modify: `crates/state/src/migrations.rs`
- Modify: `crates/state/src/database.rs`
- Test: `crates/state/tests/state_foundation.rs`

- [ ] **Step 1: Write failing state tests**

Add these tests near `resource_state_apis_reject_latest_tracks` in `crates/state/tests/state_foundation.rs`:

```rust
#[test]
fn global_php_default_track_round_trips() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    assert_eq!(database.global_php_default_track()?, None);

    database.set_global_php_default_track("8.4")?;
    assert_eq!(database.global_php_default_track()?, Some("8.4".to_string()));

    database.set_global_php_default_track("8.3")?;
    assert_eq!(database.global_php_default_track()?, Some("8.3".to_string()));

    Ok(())
}

#[test]
fn global_php_default_rejects_latest_and_invalid_tracks() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    assert!(matches!(
        database.set_global_php_default_track("latest"),
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    assert!(matches!(
        database.set_global_php_default_track(""),
        Err(StateError::InvalidProjectTrack { track }) if track.is_empty()
    ));

    Ok(())
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```shell
cargo nextest run -p state -E 'test(global_php_default)' --locked
```

Expected: compile failure because `Database::global_php_default_track` and `Database::set_global_php_default_track` do not exist.

- [ ] **Step 3: Add migration SQL**

Create `crates/state/src/sql/006_global_php_default.sql`:

```sql
CREATE TABLE global_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

Register it in `crates/state/src/migrations.rs`:

```rust
const GLOBAL_PHP_DEFAULT_SQL: &str = include_str!("sql/006_global_php_default.sql");
```

Add this migration to `DEFAULT_MIGRATIONS` after version 5:

```rust
Migration::new(6, "global_php_default", GLOBAL_PHP_DEFAULT_SQL),
```

- [ ] **Step 4: Implement database methods**

Add this constant near existing database constants in `crates/state/src/database.rs`:

```rust
const GLOBAL_PHP_DEFAULT_KEY: &str = "php.default_track";
```

Add these methods to `impl Database` near the Project PHP track methods:

```rust
pub fn global_php_default_track(&self) -> Result<Option<String>, StateError> {
    self.connection
        .query_row(
            "SELECT value FROM global_settings WHERE key = ?1",
            params![GLOBAL_PHP_DEFAULT_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(StateError::from)
}

pub fn set_global_php_default_track(&mut self, track: &str) -> Result<String, StateError> {
    validate_project_php_track(track)?;
    validate_concrete_track(track)?;

    let updated_at = timestamp()?;
    self.connection.execute(
        "INSERT INTO global_settings (key, value, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at",
        params![GLOBAL_PHP_DEFAULT_KEY, track, updated_at],
    )?;

    Ok(track.to_string())
}
```

- [ ] **Step 5: Run state tests**

Run:

```shell
cargo nextest run -p state -E 'test(global_php_default)' --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```shell
git add crates/state/src/sql/006_global_php_default.sql crates/state/src/migrations.rs crates/state/src/database.rs crates/state/tests/state_foundation.rs
git commit -m "feat(state): persist global PHP default track"
```

---

### Task 2: Add Project Config `php:` Mutation Helper

**Files:**
- Create: `crates/config/src/writer.rs`
- Modify: `crates/config/src/lib.rs`
- Test: `crates/config/tests/project_config.rs`

- [ ] **Step 1: Write failing config writer tests**

Add these tests to `crates/config/tests/project_config.rs`:

```rust
#[test]
fn project_config_writer_updates_existing_preferred_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("project");
    create_dir(&project)?;
    let config_path = project.join("pv.yml");
    write_file(
        &config_path,
        "hostnames:\n  - api.acme.test\nphp: 8.3\nenv:\n  APP_URL: \"${project_url}\"\n",
    )?;

    let written = config::write_project_php_track(&project, "8.4")?;
    let content = read_file(&config_path)?;
    let parsed = ProjectConfigFile::read_from_root(&project)?;

    assert_eq!(written.path, config_path);
    assert_eq!(parsed.config.php.as_deref(), Some("8.4"));
    assert!(content.contains("hostnames:"));
    assert!(content.contains("env:"));

    Ok(())
}

#[test]
fn project_config_writer_updates_existing_alternate_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("project");
    create_dir(&project)?;
    let config_path = project.join("pv.yaml");
    write_file(&config_path, "document_root: public\n")?;

    let written = config::write_project_php_track(&project, "8.4")?;
    let parsed = ProjectConfigFile::read_from_root(&project)?;

    assert_eq!(written.path, config_path);
    assert_eq!(parsed.config.php.as_deref(), Some("8.4"));
    assert_eq!(parsed.config.document_root.as_deref(), Some(camino::Utf8Path::new("public")));

    Ok(())
}

#[test]
fn project_config_writer_creates_preferred_file_when_missing() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("project");
    create_dir(&project)?;

    let written = config::write_project_php_track(&project, "8.4")?;
    let parsed = ProjectConfigFile::read_from_root(&project)?;

    assert_eq!(written.path, project.join("pv.yml"));
    assert!(written.created);
    assert_eq!(parsed.config.php.as_deref(), Some("8.4"));

    Ok(())
}

#[test]
fn project_config_writer_reuses_existing_conflict_error() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("project");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.3\n")?;
    write_file(&project.join("pv.yaml"), "php: 8.4\n")?;

    let result = config::write_project_php_track(&project, "8.4");

    assert!(matches!(result, Err(ConfigError::ConfigFileConflict { .. })));

    Ok(())
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```shell
cargo nextest run -p config -E 'test(project_config_writer)' --locked
```

Expected: compile failure because `write_project_php_track` and its return type do not exist.

- [ ] **Step 3: Add writer module**

Create `crates/config/src/writer.rs`:

```rust
use camino::{Utf8Path, Utf8PathBuf};
use resources::TrackSelector;
use yaml_serde::{Mapping, Value};

use crate::filesystem::{read_to_string, write_string_atomically_with_mode};
use crate::{ConfigError, ProjectConfigFile};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigWrite {
    pub path: Utf8PathBuf,
    pub created: bool,
}

pub fn write_project_php_track(
    project_root: &Utf8Path,
    track: &str,
) -> Result<ProjectConfigWrite, ConfigError> {
    TrackSelector::parse(track.to_string()).map_err(|source| ConfigError::InvalidPhpTrack {
        track: track.to_string(),
        reason: source.to_string(),
    })?;

    let config_file = ProjectConfigFile::read_from_root(project_root)?;
    let mut value = if config_file.exists {
        yaml_serde::from_str::<Value>(&read_to_string(&config_file.path)?)
            .map_err(|source| ConfigError::Parse { source })?
    } else {
        Value::Mapping(Mapping::new())
    };
    value
        .apply_merge()
        .map_err(|source| ConfigError::Parse { source })?;

    let mapping = match value {
        Value::Null => Mapping::new(),
        Value::Mapping(mapping) => mapping,
        value => {
            return Err(ConfigError::RootMustBeMapping {
                found: value_type(&value),
            });
        }
    };
    let mut mapping = mapping;
    mapping.insert(Value::String("php".to_string()), Value::String(track.to_string()));

    let content = format_project_config(Value::Mapping(mapping))?;
    write_string_atomically_with_mode(&config_file.path, &content, 0o644)?;

    Ok(ProjectConfigWrite {
        path: config_file.path,
        created: !config_file.exists,
    })
}

fn format_project_config(value: Value) -> Result<String, ConfigError> {
    let mut content = yaml_serde::to_string(&value).map_err(|source| ConfigError::Parse {
        source,
    })?;
    if !content.ends_with('\n') {
        content.push('\n');
    }

    Ok(content)
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "sequence",
        Value::Mapping(_) => "mapping",
    }
}
```

- [ ] **Step 4: Export writer**

Modify `crates/config/src/lib.rs`:

```rust
mod writer;
```

Add the export:

```rust
pub use writer::{ProjectConfigWrite, write_project_php_track};
```

- [ ] **Step 5: Run config tests**

Run:

```shell
cargo nextest run -p config -E 'test(project_config_writer)' --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```shell
git add crates/config/src/writer.rs crates/config/src/lib.rs crates/config/tests/project_config.rs
git commit -m "feat(config): update project PHP track"
```

---

### Task 3: Add Runtime Adapters And Paired Resource Commands

**Files:**
- Modify: `crates/resources/src/runtime.rs`
- Modify: `crates/resources/src/command.rs`
- Modify: `crates/resources/src/lib.rs`
- Test: `crates/resources/tests/runtime_adapters.rs`
- Test: `crates/resources/tests/managed_resource_commands.rs`

- [ ] **Step 1: Write failing adapter test**

Add to `crates/resources/tests/runtime_adapters.rs`:

```rust
#[test]
fn composer_adapter_validates_expected_phar_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = composer_adapter()?;
    let phar_path = release.join("composer.phar");
    write_sensitive_file(&phar_path, "composer phar")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), phar_path);

    Ok(())
}
```

Update the import:

```rust
use resources::{ResourceAdapter, ResourcesError, composer_adapter, frankenphp_adapter, php_adapter};
```

- [ ] **Step 2: Run adapter test to verify failure**

Run:

```shell
cargo nextest run -p resources -E 'test(composer_adapter)' --locked
```

Expected: compile failure because `composer_adapter` is not exported.

- [ ] **Step 3: Implement Composer adapter**

Add to `crates/resources/src/runtime.rs`:

```rust
pub fn composer_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("composer")?,
        "composer.phar",
    ))
}
```

Export it in `crates/resources/src/lib.rs`:

```rust
pub use runtime::{RuntimeArtifactAdapter, composer_adapter, frankenphp_adapter, php_adapter};
```

- [ ] **Step 4: Write failing paired command tests**

Add a focused test to `crates/resources/tests/managed_resource_commands.rs`:

```rust
#[test]
fn managed_resource_commands_install_php_pair() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let php_artifact = fixture_resource_artifact("php", "8.4.1-pv1", "bin/php", "php")?;
    let frankenphp_artifact =
        fixture_resource_artifact("frankenphp", "8.4.1-pv1", "bin/frankenphp", "frankenphp")?;
    let manifest = manifest_with_resources(&[
        ("php", "8.4", &[&php_artifact]),
        ("frankenphp", "8.4", &[&frankenphp_artifact]),
    ]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(php_artifact.bytes())
        .with_text(&manifest)
        .with_bytes(frankenphp_artifact.bytes());

    let installed = commands.install_php_pair(TrackSelector::Latest, &client)?;
    let listed = commands.list(None)?;

    assert_debug_snapshot!((
        install_summary(installed.php(), tempdir.path())?,
        install_summary(installed.frankenphp(), tempdir.path())?,
        track_records_summary(&listed, tempdir.path())?,
    ));

    Ok(())
}
```

Add these additional tests in the same file:

```rust
#[test]
fn managed_resource_commands_update_php_pairs() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let first_php = fixture_resource_artifact("php", "8.4.1-pv1", "bin/php", "php first")?;
    let second_php = fixture_resource_artifact("php", "8.4.2-pv1", "bin/php", "php second")?;
    let first_frankenphp =
        fixture_resource_artifact("frankenphp", "8.4.1-pv1", "bin/frankenphp", "frankenphp first")?;
    let second_frankenphp =
        fixture_resource_artifact("frankenphp", "8.4.2-pv1", "bin/frankenphp", "frankenphp second")?;
    let initial_manifest = manifest_with_resources(&[
        ("php", "8.4", &[&first_php]),
        ("frankenphp", "8.4", &[&first_frankenphp]),
    ]);
    let updated_manifest = manifest_with_resources(&[
        ("php", "8.4", &[&first_php, &second_php]),
        ("frankenphp", "8.4", &[&first_frankenphp, &second_frankenphp]),
    ]);
    let client = ScriptedClient::new()
        .with_text(&initial_manifest)
        .with_bytes(first_php.bytes())
        .with_text(&initial_manifest)
        .with_bytes(first_frankenphp.bytes())
        .with_text(&updated_manifest)
        .with_bytes(second_php.bytes())
        .with_text(&updated_manifest)
        .with_bytes(second_frankenphp.bytes());

    commands.install_php_pair(TrackSelector::Latest, &client)?;
    let updated = commands.update_php_pairs(&client)?;
    let listed = commands.list(None)?;

    assert_debug_snapshot!((
        update_summary(updated.php(), tempdir.path())?,
        update_summary(updated.frankenphp(), tempdir.path())?,
        track_records_summary(&listed, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_uninstall_php_pair() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let php_artifact = fixture_resource_artifact("php", "8.4.1-pv1", "bin/php", "php")?;
    let frankenphp_artifact =
        fixture_resource_artifact("frankenphp", "8.4.1-pv1", "bin/frankenphp", "frankenphp")?;
    let manifest = manifest_with_resources(&[
        ("php", "8.4", &[&php_artifact]),
        ("frankenphp", "8.4", &[&frankenphp_artifact]),
    ]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(php_artifact.bytes())
        .with_text(&manifest)
        .with_bytes(frankenphp_artifact.bytes());
    commands.install_php_pair(TrackSelector::Latest, &client)?;

    let removal = commands.uninstall_php_pair(
        &TrackName::new("8.4")?,
        ManagedResourceUninstallOptions::default(),
    )?;
    let listed = commands.list(None)?;

    assert_debug_snapshot!((
        removal_intent_summary(removal.php()),
        removal_intent_summary(removal.frankenphp()),
        track_records_summary(&listed, tempdir.path())?,
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_install_composer_track_two() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let composer_artifact =
        fixture_resource_artifact("composer", "2.8.0-pv1", "composer.phar", "composer")?;
    let manifest = manifest_with_resources(&[("composer", "2", &[&composer_artifact])]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(composer_artifact.bytes());

    let installed = commands.install_composer(&client)?;

    assert_debug_snapshot!(install_summary(&installed, tempdir.path())?);

    Ok(())
}
```

Create `manifest_with_resources` and `fixture_resource_artifact` by generalizing the existing single-resource fixture helpers in `crates/resources/tests/managed_resource_commands.rs`. The helper signatures must be:

```rust
fn fixture_resource_artifact(
    resource: &str,
    version: &str,
    required_path: &str,
    content: &str,
) -> Result<FixtureArtifact>

fn manifest_with_resources(resources: &[(&str, &str, &[&FixtureArtifact])]) -> String
```

- [ ] **Step 5: Run command tests to verify failure**

Run:

```shell
cargo nextest run -p resources -E 'test(managed_resource_commands_install_php_pair) or test(composer_adapter)' --locked
```

Expected: compile failure because the pair helper methods do not exist.

- [ ] **Step 6: Add pair result types**

Add to `crates/resources/src/command.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairInstall {
    php: ManagedResourceInstall,
    frankenphp: ManagedResourceInstall,
}

impl PhpPairInstall {
    pub fn php(&self) -> &ManagedResourceInstall {
        &self.php
    }

    pub fn frankenphp(&self) -> &ManagedResourceInstall {
        &self.frankenphp
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairRemovalIntent {
    php: ManagedResourceRemovalIntent,
    frankenphp: ManagedResourceRemovalIntent,
}

impl PhpPairRemovalIntent {
    pub fn php(&self) -> &ManagedResourceRemovalIntent {
        &self.php
    }

    pub fn frankenphp(&self) -> &ManagedResourceRemovalIntent {
        &self.frankenphp
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpPairUpdate {
    php: ManagedResourceUpdate,
    frankenphp: ManagedResourceUpdate,
}

impl PhpPairUpdate {
    pub fn php(&self) -> &ManagedResourceUpdate {
        &self.php
    }

    pub fn frankenphp(&self) -> &ManagedResourceUpdate {
        &self.frankenphp
    }
}
```

- [ ] **Step 7: Implement pair helpers**

Add methods to `impl ManagedResourceCommands` in `crates/resources/src/command.rs`:

```rust
pub fn install_php_pair(
    &self,
    selector: TrackSelector,
    client: &impl ResourceHttpClient,
) -> ManagedResourceCommandResult<PhpPairInstall> {
    let php = self.install(&crate::php_adapter()?, selector.clone(), client)?;
    let frankenphp = self.install(
        &crate::frankenphp_adapter()?,
        TrackSelector::Track(php.track().clone()),
        client,
    )?;

    Ok(PhpPairInstall { php, frankenphp })
}

pub fn uninstall_php_pair(
    &self,
    track: &TrackName,
    options: ManagedResourceUninstallOptions,
) -> ManagedResourceCommandResult<PhpPairRemovalIntent> {
    let php = self.uninstall(&ResourceName::new("php")?, track, options)?;
    let frankenphp = self.uninstall(&ResourceName::new("frankenphp")?, track, options)?;

    Ok(PhpPairRemovalIntent { php, frankenphp })
}

pub fn update_php_pairs(
    &self,
    client: &impl ResourceHttpClient,
) -> ManagedResourceCommandResult<PhpPairUpdate> {
    let php = self.update(&crate::php_adapter()?, client)?;
    let frankenphp = self.update(&crate::frankenphp_adapter()?, client)?;

    Ok(PhpPairUpdate { php, frankenphp })
}

pub fn install_composer(
    &self,
    client: &impl ResourceHttpClient,
) -> ManagedResourceCommandResult<ManagedResourceInstall> {
    self.install(
        &crate::composer_adapter()?,
        TrackSelector::Track(TrackName::new("2")?),
        client,
    )
}

pub fn update_composer(
    &self,
    client: &impl ResourceHttpClient,
) -> ManagedResourceCommandResult<ManagedResourceUpdate> {
    self.update(&crate::composer_adapter()?, client)
}
```

- [ ] **Step 8: Export result types**

Modify `crates/resources/src/lib.rs` command exports:

```rust
ManagedResourceTrack, ManagedResourceUninstallOptions, ManagedResourceUpdate, PhpPairInstall,
PhpPairRemovalIntent, PhpPairUpdate,
```

- [ ] **Step 9: Run resource tests**

Run:

```shell
cargo nextest run -p resources -E 'test(managed_resource_commands_install_php_pair) or test(composer_adapter)' --locked
```

Expected: PASS.

- [ ] **Step 10: Commit**

```shell
git add crates/resources/src/runtime.rs crates/resources/src/command.rs crates/resources/src/lib.rs crates/resources/tests/runtime_adapters.rs crates/resources/tests/managed_resource_commands.rs
git commit -m "feat(resources): manage PHP runtime pairs"
```

---

### Task 4: Implement PHP CLI Management Commands

**Files:**
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/commands/php.rs`
- Modify: `crates/cli/src/error.rs`
- Test: `it/cli.rs`
- Test: `crates/cli/tests/php.rs` if a crate-level harness is added for injected environment tests.

- [ ] **Step 1: Write failing CLI route/help tests**

Add to `it/cli.rs`:

```rust
#[test]
fn php_management_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["php:use", "--help"])?,
        run_pv(&["php:install", "--help"])?,
        run_pv(&["php:update", "--help"])?,
        run_pv(&["php:uninstall", "--help"])?,
        run_pv(&["php:list", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}
```

- [ ] **Step 2: Run test to verify failure**

Run:

```shell
cargo nextest run -p pv -E 'test(php_management_commands_are_documented)' --locked
```

Expected: FAIL because most commands are unknown.

- [ ] **Step 3: Add Clap args**

Modify `crates/cli/src/args.rs`:

```rust
    #[command(name = "php:use", about = "Use a PHP track for a Project or globally")]
    PhpUse(PhpUseArgs),

    #[command(name = "php:update", about = "Update installed PHP tracks")]
    PhpUpdate,

    #[command(name = "php:uninstall", about = "Uninstall a PHP track")]
    PhpUninstall(PhpUninstallArgs),

    #[command(name = "php:list", about = "List installed PHP tracks")]
    PhpList,
```

Add argument structs:

```rust
#[derive(Debug, clap::Args)]
pub(crate) struct PhpUseArgs {
    #[arg(value_name = "version", help = "PHP track to use")]
    pub(crate) track: String,

    #[arg(short = 'g', long, help = "Set the global default PHP track")]
    pub(crate) global: bool,
}

#[derive(Debug, clap::Args)]
pub(crate) struct PhpUninstallArgs {
    #[arg(value_name = "version", help = "PHP track to uninstall")]
    pub(crate) track: String,

    #[arg(long, help = "Remove PV-owned runtime data for the track")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove the track even if Projects or defaults use it")]
    pub(crate) force: bool,
}
```

- [ ] **Step 4: Route commands**

Modify `crates/cli/src/commands/mod.rs`:

```rust
        Command::PhpUse(args) => php::use_track(args, environment, stdout),
        Command::PhpInstall(args) => php::install(args, environment, stdout),
        Command::PhpUpdate => php::update(environment, stdout),
        Command::PhpUninstall(args) => php::uninstall(args, environment, stdout),
        Command::PhpList => php::list(environment, stdout),
```

- [ ] **Step 5: Update `php::install` signature**

Replace the deferred stub in `crates/cli/src/commands/php.rs` with a real module skeleton:

```rust
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ManagedResourceCommands, ManagedResourceUninstallOptions, TargetPlatform, TrackName,
    TrackSelector, UreqResourceHttpClient,
};
use state::{Database, PvPaths, StateError};

use crate::args::{PhpInstallArgs, PhpUninstallArgs, PhpUseArgs};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

const DEFAULT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";

pub(crate) fn use_track(
    args: PhpUseArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = TrackSelector::parse(args.track)?;
    let commands = resource_commands(&paths)?;
    let client = UreqResourceHttpClient::default();
    let installed = commands.install_php_pair(selector, &client)?;
    let track = installed.php().track().as_str().to_string();
    let mut output = Output::new(stdout, OutputMode::plain());

    if args.global {
        let mut database = Database::open(&paths)?;
        database.set_global_php_default_track(&track)?;
        output.line(&format!("Set global PHP track to {track}"))?;
        request_system_reconciliation(&paths, &mut output)?;
    } else {
        let mut database = Database::open(&paths)?;
        let project = resolve_current_project(&database, environment)?;
        let write = config::write_project_php_track(&project.path, &track)?;
        database.replace_project_desired_php_track(&project.id, Some(&track))?;
        output.line(&format!("Set {} PHP track to {track}", project.primary_hostname))?;
        output.line(&format!("Updated Project config: {}", write.path))?;
        request_project_reconciliation(&paths, &project, &mut output)?;
    }

    output.line(&format!("Installed PHP track {track}"))?;

    Ok(ExitCode::SUCCESS)
}
```

This skeleton will not compile until helper functions below are added.

- [ ] **Step 6: Add helper functions**

Add these helpers to `crates/cli/src/commands/php.rs`:

```rust
pub(crate) fn install(
    args: PhpInstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = match args.track {
        Some(track) => TrackSelector::parse(track)?,
        None => TrackSelector::Latest,
    };
    let commands = resource_commands(&paths)?;
    let client = UreqResourceHttpClient::default();
    let installed = commands.install_php_pair(selector, &client)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!(
        "Installed PHP track {}",
        installed.php().track().as_str()
    ))?;
    output.line(&format!(
        "Installed FrankenPHP track {}",
        installed.frankenphp().track().as_str()
    ))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(environment: &impl Environment, stdout: &mut impl Write) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths)?;
    let client = UreqResourceHttpClient::default();
    let php = commands.update(&resources::php_adapter()?, &client)?;
    let frankenphp = commands.update(&resources::frankenphp_adapter()?, &client)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Updated {} PHP track(s)", php.installs().len()))?;
    output.line(&format!("Updated {} FrankenPHP track(s)", frankenphp.installs().len()))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    args: PhpUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let track = TrackName::new(args.track)?;
    let options = ManagedResourceUninstallOptions::new()
        .prune(args.prune)
        .force(args.force);
    let commands = resource_commands(&paths)?;
    let removal = commands.uninstall_php_pair(&track, options)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Queued removal for PHP track {}", removal.php().track()))?;
    output.line(&format!(
        "Queued removal for FrankenPHP track {}",
        removal.frankenphp().track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(environment: &impl Environment, stdout: &mut impl Write) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let default_track = database.global_php_default_track()?;
    let php = resources::ResourceName::new("php")?;
    let commands = resource_commands(&paths)?;
    let tracks = commands.list(Some(&php))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    if tracks.is_empty() {
        output.line("No PHP tracks installed")?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Track  Default  Projects  Version  Path")?;
    for track in tracks {
        let marker = if default_track.as_deref() == Some(track.track().as_str()) {
            "yes"
        } else {
            "no"
        };
        output.line(&format!(
            "{}  {}  {}  {}  {}",
            track.track(),
            marker,
            track.usage_count(),
            track.installed_version(),
            track.current_artifact_path()
        ))?;
    }

    Ok(ExitCode::SUCCESS)
}

fn resource_commands(paths: &PvPaths) -> Result<ManagedResourceCommands, ExecuteError> {
    Ok(ManagedResourceCommands::new(
        paths.clone(),
        DEFAULT_MANIFEST_URL,
        TargetPlatform::DarwinArm64,
    ))
}

fn resolve_current_project(
    database: &Database,
    environment: &impl Environment,
) -> Result<state::ProjectRecord, ExecuteError> {
    let current_dir = Utf8PathBuf::from_path_buf(environment.current_dir()?)
        .map_err(|path| crate::CliError::NonUtf8Path { path })?;
    database
        .nearest_project_for_path(&current_dir)?
        .ok_or_else(|| crate::CliError::ProjectNotResolved.into())
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
```

- [ ] **Step 7: Add reconciliation helpers**

Add to `crates/cli/src/commands/php.rs`:

```rust
fn request_project_reconciliation(
    paths: &PvPaths,
    project: &state::ProjectRecord,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let scope = format!("project:{}", project.id);
    match daemon::submit_job_blocking(paths.clone(), "reconcile", &scope) {
        Ok(job) => output.line(&format!(
            "Queued reconciliation {} for {}",
            job.id, project.primary_hostname
        ))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => output.line(
            "warning: PV daemon is not running; reconciliation will run after `pv setup` starts it",
        )?,
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn request_system_reconciliation(
    paths: &PvPaths,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    match daemon::submit_job_blocking(paths.clone(), "reconcile", "system") {
        Ok(job) => output.line(&format!("System reconciliation requested: {}", job.id))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => output.line(
            "warning: PV daemon is not running; reconciliation will run after `pv setup` starts it",
        )?,
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn daemon_is_unavailable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}
```

- [ ] **Step 8: Run help route tests**

Run:

```shell
cargo insta test --accept --test-runner nextest -- php_management_commands_are_documented
```

Expected: PASS and accepted snapshots document the new command surface.

- [ ] **Step 9: Commit**

```shell
git add crates/cli/src/args.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/php.rs crates/cli/src/error.rs it/cli.rs it/snapshots
git commit -m "feat(cli): add PHP management commands"
```

---

### Task 5: Implement PHP Shim Execution

**Files:**
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/commands/php.rs`
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/cli/src/error.rs`
- Test: `crates/cli/tests/php.rs`

- [ ] **Step 1: Write failing injected-environment shim tests**

Create `crates/cli/tests/php.rs` with a test environment matching style from `crates/cli/tests/project_open.rs`. Add this test:

```rust
#[test]
fn php_shim_fails_clearly_when_resolved_project_track_is_missing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("project");
    state::fs::write_sensitive_file(&project.join("pv.yml"), "php: 8.4\n")?;
    let environment = TestEnvironment::new(&home, &project);
    let mut database = Database::open(&PvPaths::for_home(&home))?;
    database.link_project(state::LinkProjectInput {
        path: project.clone(),
        original_path: project.clone(),
        primary_hostname: "project.test".to_string(),
        config_path: project.join("pv.yml"),
        desired_php_track: Some("8.4".to_string()),
        additional_hostnames: Vec::new(),
    })?;

    let output = run_pv(&["shim:php", "-v"], &environment)?;

    assert_debug_snapshot!(output);
    assert!(environment.exec_calls().is_empty());

    Ok(())
}
```

Also add a success test that seeds `managed_resource_tracks` with `php:8.4` and a fake current artifact path containing `bin/php`, then asserts the environment recorded an exec call to that path with `-v`.

- [ ] **Step 2: Run tests to verify failure**

Run:

```shell
cargo nextest run -p cli -E 'test(php_shim)' --locked
```

Expected: compile failure because `shim:php` and exec hooks do not exist.

- [ ] **Step 3: Add hidden shim args**

Modify `crates/cli/src/args.rs`:

```rust
    #[command(name = "shim:php", about = "Run the internal PV PHP shim", hide = true, trailing_var_arg = true)]
    ShimPhp(ShimArgs),
```

Add:

```rust
#[derive(Debug, clap::Args)]
pub(crate) struct ShimArgs {
    #[arg(value_name = "args", allow_hyphen_values = true, trailing_var_arg = true)]
    pub(crate) args: Vec<String>,
}
```

Route it in `crates/cli/src/commands/mod.rs`:

```rust
        Command::ShimPhp(args) => php::shim(args, environment),
```

- [ ] **Step 4: Add exec hook to environment**

Modify `crates/cli/src/environment.rs`:

```rust
fn exec(&self, program: &std::path::Path, args: &[String]) -> io::Result<ExitCode>;
```

For `ProcessEnvironment`, implement this using platform-specific process replacement on Unix. If direct `exec` triggers clippy disallowed process-spawn lints, route through a narrow `platform` helper instead of calling raw process APIs from `cli`.

For tests, `TestEnvironment::exec` should record `(program, args)` and return `ExitCode::SUCCESS`.

- [ ] **Step 5: Implement shim resolution**

Add to `crates/cli/src/commands/php.rs`:

```rust
pub(crate) fn shim(
    args: crate::args::ShimArgs,
    environment: &impl Environment,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let track = resolve_php_track_for_shim(&paths, &database, environment)?;
    let executable = installed_php_executable(&database, &track)?;

    environment.exec(executable.as_std_path(), &args.args).map_err(ExecuteError::from)
}

fn resolve_php_track_for_shim(
    paths: &PvPaths,
    database: &Database,
    environment: &impl Environment,
) -> Result<String, ExecuteError> {
    let current_dir = Utf8PathBuf::from_path_buf(environment.current_dir()?)
        .map_err(|path| crate::CliError::NonUtf8Path { path })?;
    if let Some(project) = database.nearest_project_for_path(&current_dir)? {
        if let Some(track) = project.desired_php_track {
            return Ok(track);
        }
    }

    if let Some(track) = database.global_php_default_track()? {
        return Ok(track);
    }

    let manifest = resources::ArtifactManifestCache::new(paths.downloads()).load_cached()?;
    let php = resources::ResourceName::new("php")?;
    Ok(manifest
        .resolve_track(&php, TrackSelector::Latest)?
        .as_str()
        .to_string())
}

fn installed_php_executable(
    database: &Database,
    track: &str,
) -> Result<camino::Utf8PathBuf, ExecuteError> {
    let record = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| {
            record.resource_name == "php"
                && record.track == track
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
        .ok_or_else(|| crate::CliError::MissingPhpTrack {
            track: track.to_string(),
        })?;
    let release = record.current_artifact_path.ok_or_else(|| crate::CliError::MissingPhpTrack {
        track: track.to_string(),
    })?;

    Ok(resources::php_adapter()?.executable_path(&release))
}
```

Add `CliError::MissingPhpTrack`:

```rust
#[error("PHP track {track} is not installed.\nRun `pv php:install {track}` to install it.")]
MissingPhpTrack { track: String },
```

- [ ] **Step 6: Run shim tests**

Run:

```shell
cargo insta test --accept --test-runner nextest -- php_shim
```

Expected: PASS and snapshots show clear missing-track errors.

- [ ] **Step 7: Commit**

```shell
git add crates/cli/src/args.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/php.rs crates/cli/src/environment.rs crates/cli/src/error.rs crates/cli/tests/php.rs crates/cli/tests/snapshots
git commit -m "feat(cli): add PHP shim"
```

---

### Task 6: Implement Composer Commands And Shim

**Files:**
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Create: `crates/cli/src/commands/composer.rs`
- Modify: `crates/cli/src/error.rs`
- Test: `it/cli.rs`
- Test: `crates/cli/tests/composer.rs`

- [ ] **Step 1: Write failing Composer command help test**

Add to `it/cli.rs`:

```rust
#[test]
fn composer_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["composer:install", "--help"])?,
        run_pv(&["composer:update", "--help"])?,
        run_pv(&["composer:uninstall", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}
```

- [ ] **Step 2: Run test to verify failure**

Run:

```shell
cargo nextest run -p pv -E 'test(composer_commands_are_documented)' --locked
```

Expected: FAIL because Composer commands are unknown.

- [ ] **Step 3: Add args and routing**

Modify `crates/cli/src/args.rs`:

```rust
    #[command(name = "composer:install", about = "Install Composer track 2")]
    ComposerInstall,

    #[command(name = "composer:update", about = "Update Composer track 2")]
    ComposerUpdate,

    #[command(name = "composer:uninstall", about = "Uninstall Composer")]
    ComposerUninstall(ComposerUninstallArgs),

    #[command(name = "shim:composer", about = "Run the internal PV Composer shim", hide = true, trailing_var_arg = true)]
    ShimComposer(ShimArgs),
```

Add:

```rust
#[derive(Debug, clap::Args)]
pub(crate) struct ComposerUninstallArgs {
    #[arg(long, help = "Remove PV-owned Composer home/cache")]
    pub(crate) prune: bool,

    #[arg(long, help = "Remove Composer even if in use")]
    pub(crate) force: bool,
}
```

Modify `crates/cli/src/commands/mod.rs`:

```rust
mod composer;
```

Add match arms:

```rust
        Command::ComposerInstall => composer::install(environment, stdout),
        Command::ComposerUpdate => composer::update(environment, stdout),
        Command::ComposerUninstall(args) => composer::uninstall(args, environment, stdout),
        Command::ShimComposer(args) => composer::shim(args, environment),
```

- [ ] **Step 4: Implement Composer commands**

Create `crates/cli/src/commands/composer.rs`:

```rust
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ManagedResourceCommands, ManagedResourceUninstallOptions, ResourceName, TargetPlatform,
    TrackName, TrackSelector, UreqResourceHttpClient,
};
use state::{Database, PvPaths, StateError};

use crate::args::{ComposerUninstallArgs, ShimArgs};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

const DEFAULT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";

pub(crate) fn install(environment: &impl Environment, stdout: &mut impl Write) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths);
    let client = UreqResourceHttpClient::default();
    let database = Database::open(&paths)?;
    let default_php_track = resolved_global_php_track(&paths, &database)?;
    let composer = commands.install_composer(&client)?;
    commands.install_php_pair(TrackSelector::Track(TrackName::new(default_php_track)?), &client)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Installed Composer track {}", composer.track()))?;

    Ok(ExitCode::SUCCESS)
}

fn resolved_global_php_track(paths: &PvPaths, database: &Database) -> Result<String, ExecuteError> {
    if let Some(track) = database.global_php_default_track()? {
        return Ok(track);
    }

    let manifest = resources::ArtifactManifestCache::new(paths.downloads()).load_cached()?;
    let php = ResourceName::new("php")?;
    Ok(manifest
        .resolve_track(&php, TrackSelector::Latest)?
        .as_str()
        .to_string())
}

pub(crate) fn update(environment: &impl Environment, stdout: &mut impl Write) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths);
    let client = UreqResourceHttpClient::default();
    let updated = commands.update_composer(&client)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Updated {} Composer track(s)", updated.installs().len()))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    args: ComposerUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let commands = resource_commands(&paths);
    let composer = ResourceName::new("composer")?;
    let track = TrackName::new("2")?;
    let options = ManagedResourceUninstallOptions::new()
        .prune(args.prune)
        .force(args.force);
    let removal = commands.uninstall(&composer, &track, options)?;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Queued removal for Composer track {}", removal.track()))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn shim(args: ShimArgs, environment: &impl Environment) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let phar = installed_composer_phar(&database)?;
    let mut shim_args = Vec::with_capacity(args.args.len() + 1);
    shim_args.push(phar.to_string());
    shim_args.extend(args.args);

    super::php::shim_with_args(shim_args, environment)
}

fn installed_composer_phar(database: &Database) -> Result<camino::Utf8PathBuf, ExecuteError> {
    let record = database
        .managed_resource_tracks()?
        .into_iter()
        .find(|record| {
            record.resource_name == "composer"
                && record.track == "2"
                && record.installed_version.is_some()
                && record.current_artifact_path.is_some()
        })
        .ok_or(crate::CliError::MissingComposer)?;
    let release = record
        .current_artifact_path
        .ok_or(crate::CliError::MissingComposer)?;

    Ok(resources::composer_adapter()?.executable_path(&release))
}

fn resource_commands(paths: &PvPaths) -> ManagedResourceCommands {
    ManagedResourceCommands::new(paths.clone(), DEFAULT_MANIFEST_URL, TargetPlatform::DarwinArm64)
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
```

- [ ] **Step 5: Add Composer error**

Modify `crates/cli/src/error.rs`:

```rust
#[error("Composer track 2 is not installed.\nRun `pv composer:install` to install it.")]
MissingComposer,
```

- [ ] **Step 6: Add shared PHP shim entry helper**

Expose a crate-private helper in `crates/cli/src/commands/php.rs`:

```rust
pub(crate) fn shim_with_args(
    args: Vec<String>,
    environment: &impl Environment,
) -> Result<ExitCode, ExecuteError> {
    run_php_shim(args, environment)
}
```

Make the existing `shim` call `run_php_shim(args.args, environment)`.

- [ ] **Step 7: Run Composer tests**

Run:

```shell
cargo insta test --accept --test-runner nextest -- composer_commands_are_documented
cargo nextest run -p cli -E 'test(composer)' --locked
```

Expected: PASS.

- [ ] **Step 8: Commit**

```shell
git add crates/cli/src/args.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/composer.rs crates/cli/src/commands/php.rs crates/cli/src/error.rs crates/cli/tests/composer.rs crates/cli/tests/snapshots it/cli.rs it/snapshots
git commit -m "feat(cli): add Composer commands and shim"
```

---

### Task 7: Update `DESIGN.md` And Command Snapshots

**Files:**
- Modify: `DESIGN.md`
- Modify: `it/cli.rs`
- Modify: `it/snapshots/*`

- [ ] **Step 1: Write documentation diff**

Edit `DESIGN.md`:

- Replace `pv php:default <track>` prose with `pv php:use <track> --global`.
- Add Project-level `pv php:use <track>` prose.
- Keep `php` shim missing-track behavior explicit: shims do not auto-download.
- Keep Composer track `2` prose aligned with `composer:install`, `composer:update`, and `composer:uninstall [--prune] [--force]`.
- Update the command table to include `pv php:use <version> [--global]`.

- [ ] **Step 2: Verify no stale command text remains**

Run:

```shell
rg -n "php:default|default <version>" DESIGN.md docs/superpowers/specs/2026-06-07-pr-15-php-shim-composer-design.md
```

Expected: only the PR 15 spec references `php:default` as a replaced command.

- [ ] **Step 3: Refresh command snapshots**

Run:

```shell
cargo insta test --accept --test-runner nextest -- php_management_commands_are_documented composer_commands_are_documented
```

Expected: PASS.

- [ ] **Step 4: Commit**

```shell
git add DESIGN.md it/cli.rs it/snapshots
git commit -m "docs: update PHP command contract"
```

---

### Task 8: Final Verification

**Files:**
- No new files unless tests reveal a focused fix.

- [ ] **Step 1: Run formatting**

Run:

```shell
cargo fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 2: Run focused package tests**

Run:

```shell
cargo nextest run -p pv -p cli -p resources -p state -p config -p daemon --locked
```

Expected: PASS.

- [ ] **Step 3: Run clippy**

Run:

```shell
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Run diff hygiene**

Run:

```shell
git diff --check
```

Expected: no output.

- [ ] **Step 5: Inspect commit stack**

Run:

```shell
git log --oneline origin/main..HEAD
```

Expected: commits are small, conventional, and ordered by state/config/resources/cli/docs.
