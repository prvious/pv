# PHP Track Defaults Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Seed PV-owned PHP defaults per track and run CLI PHP, Composer, and FrankenPHP workers with the same track-level `PHPRC` and `PHP_INI_SCAN_DIR`.

**Architecture:** Add one shared `resources::php_defaults` component that owns PHP track default paths, seeding, validation, and environment overlays. Resource install/update paths seed defaults before recording a PHP/FrankenPHP pair as installed; CLI shims and daemon worker specs consume the shared environment helpers. Artifact recipes move compiled fallback ini paths away from `/usr/local/etc/php`, while normal runtime uses explicit process environment.

**Tech Stack:** Rust 2024, `resources`, `cli`, `daemon`, `state::fs`, `pv-release`, POSIX shell, `cargo nextest`, `insta`, `shellcheck`.

## Global Constraints

- Read `CONTRIBUTING.md` for tool guidance before executing tasks.
- Consult `DESIGN.md` before product or implementation decisions; ask if a decision is not covered.
- Prefer integration tests under `it/...` or crate integration tests over narrow unit tests.
- Prefer `insta` snapshots following nearby tests over substring assertions.
- Avoid `panic!`, `unreachable!`, `.unwrap()`, unsafe code, and clippy rule ignores.
- Prefer `if let` and let chains over nested fallible branching.
- Prefer top-level imports over local imports or fully qualified names.
- Do not update all dependencies in `Cargo.lock`; use `cargo update --precise` for lockfile changes.
- PV v1 is macOS-only and targets macOS 13 and newer.
- PHP track defaults live under `~/.pv/resources/php/<track>/etc`.
- Supported PHP tracks for this default profile are `8.3`, `8.4`, and `8.5`.
- PV must not render the seeded defaults into Caddyfile `php_ini` directives.
- PV must not pass PHP ini discovery paths through Caddyfile `env` directives.
- Normal PV execution must use process-level `PHPRC` and `PHP_INI_SCAN_DIR`.

---

## File Structure

- Create `crates/resources/src/php_defaults.rs`: shared PHP track default paths, seeding, validation, and env overlay helpers.
- Create `crates/resources/src/php-defaults.ini`: tracked stripped default profile generated from the approved root `php.ini` sample.
- Modify `crates/resources/src/lib.rs`: export PHP default helpers.
- Modify `crates/resources/src/command.rs`: seed PHP defaults when PHP/FrankenPHP pair installs or updates are recorded.
- Create `crates/resources/tests/php_defaults.rs`: focused integration tests for the shared defaults component.
- Modify `crates/resources/tests/managed_resource_commands.rs`: install/update behavior tests for default seeding and preservation.
- Modify `crates/cli/src/commands/php.rs`: use track-level defaults instead of artifact release `etc`.
- Modify `crates/cli/tests/php.rs`: update shim env expectations and assert shim seeding.
- Modify `crates/cli/tests/composer.rs`: update Composer-through-PHP env expectations.
- Modify `crates/daemon/src/gateway.rs`: use track-level worker env for validation and process specs, while leaving Gateway env PHP-neutral.
- Modify `crates/daemon/tests/gateway_reconciliation.rs`: update worker process spec and validation env coverage.
- Modify `crates/daemon/tests/gateway_config.rs` snapshots only if the worker root Caddyfile snapshot changes; it should not gain `php_ini`.
- Modify `release/artifacts/recipes/php/build.sh`: pass safe compiled fallback ini paths to StaticPHP.
- Modify `release/artifacts/recipes/php/smoke.sh`: check real PHP/FrankenPHP artifacts do not report `/usr/local/etc/php`.
- Modify `crates/pv-release/tests/smoke.rs`: assert StaticPHP receives safe config-file path flags.
- Modify `DESIGN.md`: document PHP track defaults.
- Modify `docs/superpowers/plans/2026-06-21-php-track-defaults.md`: check off steps as tasks are completed.

---

### Task 1: Shared PHP Track Defaults Component

**Files:**

- Create: `crates/resources/src/php-defaults.ini`
- Create: `crates/resources/src/php_defaults.rs`
- Create: `crates/resources/tests/php_defaults.rs`
- Modify: `crates/resources/src/lib.rs`

**Interfaces:**

- Produces: `PHP_TRACK_DEFAULT_INI: &str`
- Produces: `PhpTrackDefaults { etc_dir, php_ini, conf_dir }`
- Produces: `php_track_defaults(paths: &PvPaths, track: &str) -> PhpTrackDefaults`
- Produces: `ensure_php_track_defaults(paths: &PvPaths, track: &str) -> Result<PhpTrackDefaults, StateError>`
- Produces: `php_track_environment(paths: &PvPaths, track: &str) -> BTreeMap<String, String>`
- Produces: `php_track_exec_environment(paths: &PvPaths, track: &str) -> Vec<(OsString, OsString)>`
- Consumes: `state::PvPaths`, `state::fs`

- [x] **Step 1: Write the failing integration tests**

Create `crates/resources/tests/php_defaults.rs`:

```rust
use std::collections::BTreeMap;

use anyhow::Result;
use camino_tempfile::tempdir;
use resources::{
    PHP_TRACK_DEFAULT_INI, ensure_php_track_defaults, php_track_defaults,
    php_track_environment, php_track_exec_environment,
};
use state::{PvPaths, fs};

#[test]
fn php_track_defaults_seed_stripped_sample_once() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let defaults = ensure_php_track_defaults(&paths, "8.4")?;
    let first_content = fs::read_to_string(defaults.php_ini())?;

    assert_eq!(defaults.etc_dir(), paths.resources().join("php/8.4/etc"));
    assert_eq!(defaults.conf_dir(), paths.resources().join("php/8.4/etc/conf.d"));
    assert_eq!(first_content, PHP_TRACK_DEFAULT_INI);
    assert!(first_content.starts_with("[PHP]\nengine = On\n"));
    assert!(first_content.contains("\n[Date]\n"));
    assert!(first_content.contains("\nunserialize_callback_func =\n"));
    assert!(!first_content.contains("; About php.ini"));

    fs::write_sensitive_file(defaults.php_ini(), "memory_limit = 768M\n")?;
    let seeded_again = ensure_php_track_defaults(&paths, "8.4")?;

    assert_eq!(seeded_again, defaults);
    assert_eq!(fs::read_to_string(defaults.php_ini())?, "memory_limit = 768M\n");

    Ok(())
}

#[test]
fn php_track_defaults_reject_blocking_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let defaults = php_track_defaults(&paths, "8.5");
    fs::ensure_user_dir(defaults.etc_dir())?;
    fs::write_sensitive_file(defaults.conf_dir(), "not a directory\n")?;

    let error = match ensure_php_track_defaults(&paths, "8.5") {
        Ok(_) => anyhow::bail!("expected blocking conf.d path to fail"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("PHP track defaults conf.d path is not a directory")
    );

    Ok(())
}

#[test]
fn php_track_defaults_env_helpers_point_at_track_etc() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    assert_eq!(
        php_track_environment(&paths, "8.3"),
        BTreeMap::from([
            (
                "PHPRC".to_owned(),
                paths.resources().join("php/8.3/etc").to_string(),
            ),
            (
                "PHP_INI_SCAN_DIR".to_owned(),
                paths.resources().join("php/8.3/etc/conf.d").to_string(),
            ),
        ])
    );

    let exec_env = php_track_exec_environment(&paths, "8.3")
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        exec_env,
        vec![
            (
                "PHPRC".to_owned(),
                paths.resources().join("php/8.3/etc").to_string(),
            ),
            (
                "PHP_INI_SCAN_DIR".to_owned(),
                paths.resources().join("php/8.3/etc/conf.d").to_string(),
            ),
        ]
    );

    Ok(())
}
```

- [x] **Step 2: Run the tests to verify they fail**

Run:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

Expected: FAIL because `resources::PHP_TRACK_DEFAULT_INI`, `ensure_php_track_defaults`, `php_track_defaults`, `php_track_environment`, and `php_track_exec_environment` do not exist.

- [x] **Step 3: Add the stripped PHP defaults asset**

Create `crates/resources/src/php-defaults.ini` with exactly:

```ini
[PHP]
engine = On
short_open_tag = Off
precision = 14
output_buffering = 4096
zlib.output_compression = Off
implicit_flush = Off
unserialize_callback_func =
serialize_precision = -1
disable_functions =
zend.enable_gc = On
zend.exception_ignore_args = Off
zend.exception_string_param_max_len = 15
expose_php = On
max_execution_time = 30
max_input_time = 60
memory_limit = 1024M
max_memory_limit = -1
error_reporting = E_ALL
display_errors = On
display_startup_errors = On
log_errors = On
ignore_repeated_errors = Off
ignore_repeated_source = Off
variables_order = "GPCS"
request_order = "GP"
auto_globals_jit = On
post_max_size = 128M
auto_prepend_file =
auto_append_file =
default_mimetype = "text/html"
default_charset = "UTF-8"
doc_root =
user_dir =
enable_dl = Off
file_uploads = On
upload_max_filesize = 128M
max_file_uploads = 20
allow_url_fopen = On
allow_url_include = Off
default_socket_timeout = 60
[CLI Server]
cli_server.color = On
[Date]
[filter]
[iconv]
[intl]
[sqlite3]
[Pcre]
[Pdo]
[Pdo_mysql]
pdo_mysql.default_socket=
[Phar]
[mail function]
SMTP = localhost
smtp_port = 25
mail.add_x_header = Off
mail.mixed_lf_and_crlf = Off
mail.cr_lf_mode = crlf
[ODBC]
odbc.allow_persistent = On
odbc.check_persistent = On
odbc.max_persistent = -1
odbc.max_links = -1
odbc.defaultlrl = 4096
odbc.defaultbinmode = 1
[MySQLi]
mysqli.max_persistent = -1
mysqli.allow_persistent = On
mysqli.max_links = -1
mysqli.default_port = 3306
mysqli.default_socket =
mysqli.default_host =
mysqli.default_user =
mysqli.default_pw =
[mysqlnd]
mysqlnd.collect_statistics = On
mysqlnd.collect_memory_statistics = On
[PostgreSQL]
pgsql.allow_persistent = On
pgsql.auto_reset_persistent = Off
pgsql.max_persistent = -1
pgsql.max_links = -1
pgsql.ignore_notice = 0
pgsql.log_notice = 0
[bcmath]
bcmath.scale = 0
[browscap]
[Session]
session.save_handler = files
session.use_strict_mode = 0
session.use_cookies = 1
session.use_only_cookies = 1
session.name = PHPSESSID
session.auto_start = 0
session.cookie_lifetime = 0
session.cookie_path = /
session.cookie_domain =
session.cookie_httponly =
session.cookie_samesite =
session.serialize_handler = php
session.gc_probability = 1
session.gc_divisor = 1000
session.gc_maxlifetime = 1440
session.referer_check =
session.cache_limiter = nocache
session.cache_expire = 180
session.use_trans_sid = 0
session.trans_sid_tags = "a=href,area=href,frame=src,form="
[Assertion]
zend.assertions = 1
[COM]
[mbstring]
[gd]
[exif]
[Tidy]
tidy.clean_output = Off
[soap]
soap.wsdl_cache_enabled=1
soap.wsdl_cache_dir="/tmp"
soap.wsdl_cache_ttl=86400
soap.wsdl_cache_limit = 5
[sysvshm]
[ldap]
ldap.max_links = -1
[dba]
[opcache]
[curl]
[openssl]
[ffi]
```

- [x] **Step 4: Implement the defaults module**

Create `crates/resources/src/php_defaults.rs`:

```rust
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use state::{PvPaths, StateError, fs};

pub const PHP_TRACK_DEFAULT_INI: &str = include_str!("php-defaults.ini");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpTrackDefaults {
    etc_dir: Utf8PathBuf,
    php_ini: Utf8PathBuf,
    conf_dir: Utf8PathBuf,
}

impl PhpTrackDefaults {
    pub fn etc_dir(&self) -> &Utf8Path {
        &self.etc_dir
    }

    pub fn php_ini(&self) -> &Utf8Path {
        &self.php_ini
    }

    pub fn conf_dir(&self) -> &Utf8Path {
        &self.conf_dir
    }
}

pub fn php_track_defaults(paths: &PvPaths, track: &str) -> PhpTrackDefaults {
    let etc_dir = paths.resources().join("php").join(track).join("etc");
    let php_ini = etc_dir.join("php.ini");
    let conf_dir = etc_dir.join("conf.d");

    PhpTrackDefaults {
        etc_dir,
        php_ini,
        conf_dir,
    }
}

pub fn ensure_php_track_defaults(
    paths: &PvPaths,
    track: &str,
) -> Result<PhpTrackDefaults, StateError> {
    let defaults = php_track_defaults(paths, track);

    fs::ensure_user_dir(defaults.etc_dir())?;
    validate_existing_php_ini(&defaults)?;
    validate_existing_conf_dir(&defaults)?;

    if !fs::path_entry_exists(defaults.conf_dir())? {
        fs::ensure_user_dir(defaults.conf_dir())?;
    }
    if !fs::path_entry_exists(defaults.php_ini())? {
        fs::write_sensitive_file(defaults.php_ini(), PHP_TRACK_DEFAULT_INI)?;
    }

    Ok(defaults)
}

pub fn php_track_environment(paths: &PvPaths, track: &str) -> BTreeMap<String, String> {
    let defaults = php_track_defaults(paths, track);

    BTreeMap::from([
        ("PHPRC".to_owned(), defaults.etc_dir().to_string()),
        (
            "PHP_INI_SCAN_DIR".to_owned(),
            defaults.conf_dir().to_string(),
        ),
    ])
}

pub fn php_track_exec_environment(paths: &PvPaths, track: &str) -> Vec<(OsString, OsString)> {
    let defaults = php_track_defaults(paths, track);

    vec![
        (
            OsString::from("PHPRC"),
            defaults.etc_dir().as_std_path().as_os_str().to_os_string(),
        ),
        (
            OsString::from("PHP_INI_SCAN_DIR"),
            defaults.conf_dir().as_std_path().as_os_str().to_os_string(),
        ),
    ]
}

fn validate_existing_php_ini(defaults: &PhpTrackDefaults) -> Result<(), StateError> {
    if !fs::path_entry_exists(defaults.php_ini())? {
        return Ok(());
    }
    if !fs::path_is_file(defaults.php_ini())? {
        return invalid_path(
            defaults.php_ini(),
            "PHP track defaults php.ini path is not a regular file",
        );
    }

    fs::read_to_string(defaults.php_ini())?;

    Ok(())
}

fn validate_existing_conf_dir(defaults: &PhpTrackDefaults) -> Result<(), StateError> {
    if !fs::path_entry_exists(defaults.conf_dir())? {
        return Ok(());
    }
    if !fs::path_is_directory(defaults.conf_dir())? {
        return invalid_path(
            defaults.conf_dir(),
            "PHP track defaults conf.d path is not a directory",
        );
    }

    Ok(())
}

fn invalid_path<T>(path: &Utf8Path, reason: &'static str) -> Result<T, StateError> {
    Err(StateError::Filesystem {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, reason),
    })
}
```

- [x] **Step 5: Export the module**

Modify `crates/resources/src/lib.rs`:

```rust
pub mod php_defaults;
```

Add exports near the other `pub use` blocks:

```rust
pub use php_defaults::{
    PHP_TRACK_DEFAULT_INI, PhpTrackDefaults, ensure_php_track_defaults,
    php_track_defaults, php_track_environment, php_track_exec_environment,
};
```

- [x] **Step 6: Run the component tests**

Run:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

Expected: PASS.

- [x] **Step 7: Commit**

```shell
git add crates/resources/src/lib.rs crates/resources/src/php_defaults.rs crates/resources/src/php-defaults.ini crates/resources/tests/php_defaults.rs
git commit -m "feat(resources): add PHP track defaults"
```

---

### Task 2: Seed Defaults During PHP Pair Install And Update

**Files:**

- Modify: `crates/resources/src/command.rs`
- Modify: `crates/resources/tests/managed_resource_commands.rs`
- Test snapshots: `crates/resources/tests/snapshots/managed_resource_commands__*.snap`

**Interfaces:**

- Consumes: `ensure_php_track_defaults(paths: &PvPaths, track: &str) -> Result<PhpTrackDefaults, StateError>` from Task 1.
- Produces: PHP pair install/update records only after defaults are usable.

- [x] **Step 1: Write failing install/update tests**

Add these tests near existing PHP pair command tests in `crates/resources/tests/managed_resource_commands.rs`:

```rust
#[test]
fn managed_resource_commands_install_php_pair_seeds_track_defaults() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let php_artifact = runtime_fixture_artifact("php", "8.4.8-pv1", "bin/php", "php 8.4")?;
    let frankenphp_artifact = runtime_fixture_artifact(
        "frankenphp",
        "8.4.8-pv1",
        "bin/frankenphp",
        "frankenphp 8.4",
    )?;
    let manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![manifest_track("8.4", vec![&php_artifact])],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![manifest_track("8.4", vec![&frankenphp_artifact])],
        ),
    ]);
    let client = ScriptedClient::new()
        .with_text(&manifest)
        .with_bytes(php_artifact.bytes())
        .with_bytes(frankenphp_artifact.bytes());

    let installed = commands.install_php_pair(TrackSelector::Latest, &client)?;
    let defaults = resources::php_track_defaults(&paths, installed.php().track().as_str());

    assert_eq!(
        state::fs::read_to_string(defaults.php_ini())?,
        resources::PHP_TRACK_DEFAULT_INI
    );
    assert!(state::fs::path_is_directory(defaults.conf_dir())?);
    assert_debug_snapshot!((
        install_summary(installed.php(), tempdir.path())?,
        install_summary(installed.frankenphp(), tempdir.path())?,
        defaults.php_ini().strip_prefix(tempdir.path())?.to_string(),
        defaults.conf_dir().strip_prefix(tempdir.path())?.to_string(),
    ));

    Ok(())
}

#[test]
fn managed_resource_commands_update_php_pairs_preserves_existing_php_ini() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands =
        ManagedResourceCommands::new(paths.clone(), MANIFEST_URL, TargetPlatform::DarwinArm64);
    let old_php = runtime_fixture_artifact("php", "8.4.7-pv1", "bin/php", "old php")?;
    let old_frankenphp =
        runtime_fixture_artifact("frankenphp", "8.4.7-pv1", "bin/frankenphp", "old fpm")?;
    let new_php = runtime_fixture_artifact("php", "8.4.8-pv1", "bin/php", "new php")?;
    let new_frankenphp =
        runtime_fixture_artifact("frankenphp", "8.4.8-pv1", "bin/frankenphp", "new fpm")?;
    let initial_manifest = manifest_with_resources(&[
        manifest_resource("php", "8.4", vec![manifest_track("8.4", vec![&old_php])]),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![manifest_track("8.4", vec![&old_frankenphp])],
        ),
    ]);
    let updated_manifest = manifest_with_resources(&[
        manifest_resource(
            "php",
            "8.4",
            vec![manifest_track("8.4", vec![&old_php, &new_php])],
        ),
        manifest_resource(
            "frankenphp",
            "8.4",
            vec![manifest_track("8.4", vec![&old_frankenphp, &new_frankenphp])],
        ),
    ]);
    let client = ScriptedClient::new()
        .with_text(&initial_manifest)
        .with_bytes(old_php.bytes())
        .with_bytes(old_frankenphp.bytes())
        .with_text(&updated_manifest)
        .with_bytes(new_php.bytes())
        .with_bytes(new_frankenphp.bytes());

    commands.install_php_pair(TrackSelector::Latest, &client)?;
    let defaults = resources::php_track_defaults(&paths, "8.4");
    state::fs::write_sensitive_file(defaults.php_ini(), "memory_limit = 768M\n")?;

    let updated = commands.update_php_pairs(&client)?;

    assert_eq!(
        state::fs::read_to_string(defaults.php_ini())?,
        "memory_limit = 768M\n"
    );
    assert_debug_snapshot!(update_summary(&updated, tempdir.path())?);

    Ok(())
}
```

- [x] **Step 2: Run the tests to verify they fail**

Run:

```shell
cargo nextest run -p resources -E 'test(managed_resource_commands_install_php_pair_seeds_track_defaults) | test(managed_resource_commands_update_php_pairs_preserves_existing_php_ini)'
```

Expected: FAIL because `install_php_pair` and `update_php_pairs` prepare artifacts and state but do not seed `resources/php/<track>/etc`.

- [x] **Step 3: Seed defaults before recording PHP pair installs**

Modify `crates/resources/src/command.rs`.

Add this helper inside `impl ManagedResourceCommands`:

```rust
    fn ensure_php_pair_defaults(
        &self,
        install: &PhpPairInstall,
    ) -> ManagedResourceCommandResult<()> {
        crate::php_defaults::ensure_php_track_defaults(&self.paths, install.php.track.as_str())?;

        Ok(())
    }
```

Update `record_php_pair_install`:

```rust
    fn record_php_pair_install(
        &self,
        install: &PhpPairInstall,
    ) -> ManagedResourceCommandResult<()> {
        self.ensure_php_pair_defaults(install)?;

        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_tracks_desired_and_installed(&[
            ManagedResourceTrackInstallInput {
                resource_name: install.php.resource_name.as_str(),
                track: install.php.track.as_str(),
                installed_version: install.php.artifact_version.as_str(),
                current_artifact_path: &install.php.current_artifact_path,
            },
            ManagedResourceTrackInstallInput {
                resource_name: install.frankenphp.resource_name.as_str(),
                track: install.frankenphp.track.as_str(),
                installed_version: install.frankenphp.artifact_version.as_str(),
                current_artifact_path: &install.frankenphp.current_artifact_path,
            },
        ])?;

        Ok(())
    }
```

Update `record_composer_with_php_pair_install` so Composer installs also seed PHP defaults before state is recorded:

```rust
    fn record_composer_with_php_pair_install(
        &self,
        php_pair: &PhpPairInstall,
        composer: &ManagedResourceInstall,
    ) -> ManagedResourceCommandResult<()> {
        self.ensure_php_pair_defaults(php_pair)?;

        let mut database = Database::open(&self.paths)?;
        database.record_managed_resource_tracks_desired_and_installed(&[
            ManagedResourceTrackInstallInput {
                resource_name: php_pair.php.resource_name.as_str(),
                track: php_pair.php.track.as_str(),
                installed_version: php_pair.php.artifact_version.as_str(),
                current_artifact_path: &php_pair.php.current_artifact_path,
            },
            ManagedResourceTrackInstallInput {
                resource_name: php_pair.frankenphp.resource_name.as_str(),
                track: php_pair.frankenphp.track.as_str(),
                installed_version: php_pair.frankenphp.artifact_version.as_str(),
                current_artifact_path: &php_pair.frankenphp.current_artifact_path,
            },
            ManagedResourceTrackInstallInput {
                resource_name: composer.resource_name.as_str(),
                track: composer.track.as_str(),
                installed_version: composer.artifact_version.as_str(),
                current_artifact_path: &composer.current_artifact_path,
            },
        ])?;

        Ok(())
    }
```

- [x] **Step 4: Run and accept focused snapshots**

Run:

```shell
cargo insta test --accept --test-runner nextest -p resources -- managed_resource_commands_install_php_pair_seeds_track_defaults
cargo insta test --accept --test-runner nextest -p resources -- managed_resource_commands_update_php_pairs_preserves_existing_php_ini
```

Expected: PASS and snapshot files are created or updated under `crates/resources/tests/snapshots/`.

- [x] **Step 5: Commit**

```shell
git add crates/resources/src/command.rs crates/resources/tests/managed_resource_commands.rs crates/resources/tests/snapshots
git commit -m "feat(resources): seed PHP track defaults on install"
```

---

### Task 3: Point CLI PHP And Composer At Track Defaults

**Files:**

- Modify: `crates/cli/src/commands/php.rs`
- Modify: `crates/cli/tests/php.rs`
- Modify: `crates/cli/tests/composer.rs`
- Test snapshots: `crates/cli/tests/snapshots/*.snap` if affected

**Interfaces:**

- Consumes: `ensure_php_track_defaults` and `php_track_exec_environment` from Task 1.
- Produces: PHP shim and Composer shim env entries using `resources/php/<track>/etc`, not `resources/php/<track>/releases/<artifact-version>/etc`.

- [x] **Step 1: Write failing CLI PHP shim assertions**

In `crates/cli/tests/php.rs`, replace the helper:

```rust
fn php_exec_env(home: &Utf8Path, track: &str) -> Vec<(String, String)> {
    let defaults = resources::php_track_defaults(&pv_paths(home), track);

    vec![
        ("PHPRC".to_string(), defaults.etc_dir().to_string()),
        (
            "PHP_INI_SCAN_DIR".to_string(),
            defaults.conf_dir().to_string(),
        ),
    ]
}
```

Update the `php_shim_sets_only_php_ini_env_overlay` assertion:

```rust
assert_eq!(
    exec_calls,
    vec![ExecCall {
        program: release.join("bin/php").as_std_path().to_path_buf(),
        args: vec!["--ini".to_string()],
        env: php_exec_env(&home, "8.4"),
    }]
);
let defaults = resources::php_track_defaults(&pv_paths(&home), "8.4");
assert_eq!(
    state::fs::read_to_string(defaults.php_ini())?,
    resources::PHP_TRACK_DEFAULT_INI
);
```

Update every `php_exec_env(&release)` call in `crates/cli/tests/php.rs` to `php_exec_env(&home, "<track>")` with the concrete track from that test.

- [x] **Step 2: Write failing Composer shim assertions**

In `crates/cli/tests/composer.rs`, replace the helper:

```rust
fn composer_exec_env(home: &Utf8Path, php_track: &str) -> Vec<(String, String)> {
    let paths = pv_paths(home);
    let defaults = resources::php_track_defaults(&paths, php_track);

    vec![
        ("COMPOSER_HOME".to_string(), paths.composer().to_string()),
        (
            "COMPOSER_CACHE_DIR".to_string(),
            paths.composer().join("cache").to_string(),
        ),
        (
            "PATH".to_string(),
            format!("{}:{}", paths.bin(), paths.composer().join("vendor/bin")),
        ),
        ("PHPRC".to_string(), defaults.etc_dir().to_string()),
        (
            "PHP_INI_SCAN_DIR".to_string(),
            defaults.conf_dir().to_string(),
        ),
    ]
}
```

Update Composer expected calls:

```rust
env: composer_exec_env(&home, "8.4"),
```

For `composer_shim_sets_pv_owned_env_overlay`, keep the explicit expected `PATH`, but replace the last two entries:

```rust
let defaults = resources::php_track_defaults(&pv_paths(&home), "8.4");
("PHPRC".to_string(), defaults.etc_dir().to_string()),
(
    "PHP_INI_SCAN_DIR".to_string(),
    defaults.conf_dir().to_string(),
),
```

- [x] **Step 3: Run the shim tests to verify they fail**

Run:

```shell
cargo nextest run -p cli -E 'test(php_shim_sets_only_php_ini_env_overlay) | test(composer_shim_execs_installed_phar_through_php_shim) | test(composer_shim_sets_pv_owned_env_overlay)'
```

Expected: FAIL because `crates/cli/src/commands/php.rs` still builds `PHPRC` from the artifact release path.

- [x] **Step 4: Update PHP shim implementation**

Modify `crates/cli/src/commands/php.rs`.

In `shim_with_args_and_env`, replace:

```rust
    env.extend(php_env_overlay(&installed.release));
```

with:

```rust
    resources::ensure_php_track_defaults(&paths, &track)?;
    env.extend(resources::php_track_exec_environment(&paths, &track));
```

Delete `fn php_env_overlay(release: &Utf8Path) -> Vec<(OsString, OsString)>`.

Remove the now-unused `Utf8Path` import if it becomes unused:

```rust
use camino::Utf8PathBuf;
```

Keep `InstalledPhp.release` because the executable path still comes from the installed artifact path.

- [x] **Step 5: Run the focused CLI tests**

Run:

```shell
cargo nextest run -p cli -E 'test(php_shim_sets_only_php_ini_env_overlay) | test(composer_shim_execs_installed_phar_through_php_shim) | test(composer_shim_sets_pv_owned_env_overlay)'
```

Expected: PASS.

- [x] **Step 6: Accept CLI snapshots if the filtered paths changed**

Run:

```shell
cargo insta test --accept --test-runner nextest -p cli -- php_shim_sets_only_php_ini_env_overlay
cargo insta test --accept --test-runner nextest -p cli -- composer_shim_execs_installed_phar_through_php_shim
cargo insta test --accept --test-runner nextest -p cli -- composer_shim_sets_pv_owned_env_overlay
```

Expected: PASS. Snapshot diffs should show `resources/php/<track>/etc` instead of `resources/php/<track>/releases/<artifact-version>/etc`.

- [x] **Step 7: Commit**

```shell
git add crates/cli/src/commands/php.rs crates/cli/tests/php.rs crates/cli/tests/composer.rs crates/cli/tests/snapshots
git commit -m "fix(cli): use PHP track defaults in shims"
```

---

### Task 4: Use Track Defaults For FrankenPHP Worker Validation And Runtime

**Files:**

- Modify: `crates/daemon/src/gateway.rs`
- Modify: `crates/daemon/tests/gateway_reconciliation.rs`
- Modify: `crates/daemon/tests/gateway_config.rs` only if snapshots need explicit no-`php_ini` assertions
- Test snapshots: `crates/daemon/tests/snapshots/*.snap`

**Interfaces:**

- Consumes: `ensure_php_track_defaults` and `php_track_environment` from Task 1.
- Produces: `worker_process_spec` includes `PHPRC` and `PHP_INI_SCAN_DIR`; `gateway_process_spec` does not.
- Produces: worker config validation receives the same private env as worker process startup.

- [x] **Step 1: Write failing worker process spec assertions**

In `crates/daemon/tests/gateway_reconciliation.rs`, update `frankenphp_command_and_process_specs_are_stable`:

```rust
    assert_eq!(gateway.private_environment.get("PHPRC"), None);
    assert_eq!(gateway.private_environment.get("PHP_INI_SCAN_DIR"), None);
    assert_eq!(
        worker.private_environment.get("PHPRC").map(String::as_str),
        Some(paths.resources().join("php/8.4/etc").as_str())
    );
    assert_eq!(
        worker
            .private_environment
            .get("PHP_INI_SCAN_DIR")
            .map(String::as_str),
        Some(paths.resources().join("php/8.4/etc/conf.d").as_str())
    );
```

- [x] **Step 2: Extend validation env coverage**

In `frankenphp_config_validation_receives_xdg_environment`, add two observed files:

```rust
    let observed_phprc = tempdir.path().join("observed-phprc");
    let observed_scan_dir = tempdir.path().join("observed-scan-dir");
```

Extend the fake validator script:

```rust
printf '%s' "${PHPRC}" > {}
printf '%s' "${PHP_INI_SCAN_DIR}" > {}
```

Add both paths to the `format!` call using `shell_single_quoted`.

Extend `private_environment`:

```rust
        (
            "PHPRC".to_owned(),
            tempdir.path().join("php/etc").as_str().to_owned(),
        ),
        (
            "PHP_INI_SCAN_DIR".to_owned(),
            tempdir.path().join("php/etc/conf.d").as_str().to_owned(),
        ),
```

Assert both values after validation:

```rust
    assert_eq!(
        state::fs::read_to_string(&observed_phprc)?,
        tempdir.path().join("php/etc").to_string()
    );
    assert_eq!(
        state::fs::read_to_string(&observed_scan_dir)?,
        tempdir.path().join("php/etc/conf.d").to_string()
    );
```

- [x] **Step 3: Run the daemon tests to verify they fail**

Run:

```shell
cargo nextest run -p daemon -E 'test(frankenphp_command_and_process_specs_are_stable) | test(frankenphp_config_validation_receives_xdg_environment)'
```

Expected: `frankenphp_command_and_process_specs_are_stable` fails because worker specs only contain XDG env today. The direct validation test may still pass because it supplies the env manually.

- [x] **Step 4: Build worker and gateway private environment helpers**

Modify `crates/daemon/src/gateway.rs`.

Add:

```rust
fn frankenphp_worker_environment(paths: &PvPaths, php_track: &str) -> BTreeMap<String, String> {
    let mut environment = frankenphp_xdg_environment(paths);
    environment.extend(resources::php_track_environment(paths, php_track));
    environment
}
```

Update `worker_process_spec`:

```rust
        private_environment: frankenphp_worker_environment(paths, php_track),
```

Leave `gateway_process_spec` unchanged:

```rust
        private_environment: frankenphp_xdg_environment(paths),
```

- [x] **Step 5: Ensure defaults and validate worker configs with worker env**

In `reconcile_worker_config`, after `subject` is created, add:

```rust
    if let Err(error) = resources::ensure_php_track_defaults(paths, &worker.php_track) {
        let error = DaemonError::from(error);
        record_runtime_error(paths, subject.clone(), &error)?;

        return Err(error);
    }
```

Change `promote_runtime_config_tree` signature:

```rust
async fn promote_runtime_config_tree(
    paths: &PvPaths,
    subject: RuntimeSubject,
    config_path: Utf8PathBuf,
    candidate_content: &str,
    active_content: &str,
    private_environment: BTreeMap<String, String>,
    promote_fragments: impl FnOnce() -> Result<PromotedConfigDir, DaemonError>,
    command: &FrankenphpCommand,
) -> Result<PromotedConfigTree, DaemonError> {
```

In its validation closure, remove the local `frankenphp_xdg_environment(paths)` allocation:

```rust
        |candidate_path| {
            let private_environment = private_environment.clone();

            async move { validate_config(command, &candidate_path, &private_environment).await }
        },
```

Update the gateway caller:

```rust
                frankenphp_xdg_environment(paths),
                || promote_config_dir(&active_dir, &candidate_dir),
                command,
```

Update the worker caller:

```rust
                frankenphp_worker_environment(paths, &worker.php_track),
                || promote_config_dir(&active_dir, &candidate_dir),
                command,
```

- [x] **Step 6: Assert worker Caddyfile snapshots stay free of generated php_ini defaults**

In `crates/daemon/tests/gateway_config.rs`, extend `worker_config_renderer_outputs_track_caddyfile`:

```rust
    let rendered = render_php_worker_config(&input)?;

    assert!(!rendered.contains("php_ini"));
    assert_snapshot!(rendered);
```

- [x] **Step 7: Run and accept daemon snapshots**

Run:

```shell
cargo insta test --accept --test-runner nextest -p daemon -- frankenphp_command_and_process_specs_are_stable
cargo insta test --accept --test-runner nextest -p daemon -- worker_config_renderer_outputs_track_caddyfile
cargo nextest run -p daemon -E 'test(frankenphp_config_validation_receives_xdg_environment)'
```

Expected: PASS. The process spec snapshot should show redacted `PHPRC` and `PHP_INI_SCAN_DIR` keys for the PHP worker only.

- [x] **Step 8: Commit**

```shell
git add crates/daemon/src/gateway.rs crates/daemon/tests/gateway_reconciliation.rs crates/daemon/tests/gateway_config.rs crates/daemon/tests/snapshots
git commit -m "fix(daemon): pass PHP track defaults to workers"
```

---

### Task 5: Move PHP Artifact Fallback Ini Paths Away From /usr/local

**Files:**

- Modify: `release/artifacts/recipes/php/build.sh`
- Modify: `release/artifacts/recipes/php/smoke.sh`
- Modify: `crates/pv-release/tests/smoke.rs`
- Test snapshots or inline expected strings in `crates/pv-release/tests/smoke.rs`

**Interfaces:**

- Produces: StaticPHP build command includes `--with-config-file-path=/var/empty/com.prvious.pv/php`.
- Produces: StaticPHP build command includes `--with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d`.
- Produces: PHP smoke rejects `/usr/local/etc/php` in `php --ini` and FrankenPHP `phpinfo()`.

- [x] **Step 1: Write failing StaticPHP argv assertion**

In `crates/pv-release/tests/smoke.rs`, find the test that asserts `run.spc_log` for `php_build_recipe_smoke`. Add:

```rust
    assert!(
        run.spc_log
            .contains("[--with-config-file-path=/var/empty/com.prvious.pv/php]"),
        "PHP recipe should set safe compiled php.ini fallback path: {}",
        run.spc_log
    );
    assert!(
        run.spc_log
            .contains("[--with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d]"),
        "PHP recipe should set safe compiled php.ini scan fallback path: {}",
        run.spc_log
    );
    assert!(
        !run.spc_log.contains("/usr/local/etc/php"),
        "PHP recipe must not pass /usr/local/etc/php fallback paths: {}",
        run.spc_log
    );
```

If the test uses an exact `assert_debug_snapshot!` for `run.spc_log`, update the expected argv to include:

```text
[--with-config-file-path=/var/empty/com.prvious.pv/php][--with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d]
```

- [x] **Step 2: Run the failing recipe test**

Run:

```shell
cargo nextest run -p pv-release -E 'test(php_build_recipe_smoke)'
```

Expected: FAIL because `release/artifacts/recipes/php/build.sh` does not pass safe config-file path flags.

- [x] **Step 3: Add safe fallback flags to the build script**

Modify the `spc build:php` invocation in `release/artifacts/recipes/php/build.sh`:

```sh
  spc build:php "$PHP_BUILD_EXTENSIONS" \
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

- [x] **Step 4: Update real smoke checks**

Modify `release/artifacts/recipes/php/smoke.sh`.

For standalone PHP, after the extension check, add:

```sh
  if "$php_binary" --ini 2>&1 | grep -F '/usr/local/etc/php' >/dev/null; then
    printf '%s\n' "PHP artifact reports unsafe /usr/local/etc/php ini fallback" >&2
    exit 46
  fi
```

For FrankenPHP, after the extension check, create the smoke `index.php` with `phpinfo()` content that can expose loaded config paths:

```sh
  cat >"$site_dir/index.php" <<'PHP'
<?php
echo "pv-frankenphp-ok\n";
phpinfo(INFO_CONFIGURATION);
PHP
```

Then replace the success curl check with:

```sh
    response=$(curl --fail --silent "http://127.0.0.1:$port/" || true)
    if printf '%s' "$response" | grep -F pv-frankenphp-ok >/dev/null; then
      if printf '%s' "$response" | grep -F '/usr/local/etc/php' >/dev/null; then
        printf '%s\n' "FrankenPHP artifact reports unsafe /usr/local/etc/php ini fallback" >&2
        exit 46
      fi
      exit 0
    fi
```

- [x] **Step 5: Run focused recipe checks**

Run:

```shell
cargo nextest run -p pv-release -E 'test(php_build_recipe_smoke)'
sh -n release/artifacts/recipes/php/build.sh
sh -n release/artifacts/recipes/php/smoke.sh
```

Expected: PASS.

If `shellcheck` is installed, also run:

```shell
shellcheck release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh
```

Expected: PASS.

- [x] **Step 6: Commit**

```shell
git add release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh crates/pv-release/tests/smoke.rs
git commit -m "fix(release): use safe PHP ini fallback paths"
```

---

### Task 6: Documentation And End-To-End Verification

**Files:**

- Modify: `DESIGN.md`
- Modify: `docs/2026-06-20-php-track-defaults-design.md` only if implementation discovers a correction
- Modify: `docs/superpowers/plans/2026-06-21-php-track-defaults.md` checkboxes as tasks are completed

**Interfaces:**

- Consumes: behavior implemented in Tasks 1-5.
- Produces: project design docs aligned with implemented behavior.

- [x] **Step 1: Update DESIGN.md**

Add a paragraph after the existing multi-version PHP ini statements around `DESIGN.md`'s Multi-version PHP section:

```markdown
For each installed PHP track, PV seeds track-level PHP defaults under `~/.pv/resources/php/<track>/etc/php.ini` and `~/.pv/resources/php/<track>/etc/conf.d/`. The defaults are mutable track data, not artifact release payload data, so artifact updates and old-release pruning do not remove user edits. PV runs standalone PHP, Composer-through-PHP, and Project-serving FrankenPHP workers with process-level `PHPRC` and `PHP_INI_SCAN_DIR` pointing at the track defaults. PV does not pass these ini discovery paths through Caddyfile `env` and does not expand the default profile into Caddyfile `php_ini` directives.
```

- [x] **Step 2: Run focused test suites**

Run:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_) | test(managed_resource_commands_install_php_pair_seeds_track_defaults) | test(managed_resource_commands_update_php_pairs_preserves_existing_php_ini)'
cargo nextest run -p cli -E 'test(php_shim_sets_only_php_ini_env_overlay) | test(composer_shim_execs_installed_phar_through_php_shim) | test(composer_shim_sets_pv_owned_env_overlay)'
cargo nextest run -p daemon -E 'test(frankenphp_command_and_process_specs_are_stable) | test(frankenphp_config_validation_receives_xdg_environment) | test(worker_config_renderer_outputs_track_caddyfile)'
cargo nextest run -p pv-release -E 'test(php_build_recipe_smoke)'
```

Expected: PASS.

- [x] **Step 3: Run formatting and diff checks**

Run:

```shell
cargo fmt --all -- --check
git diff --check
```

Expected: PASS.

- [x] **Step 4: Run clippy if local prerequisites are available**

Run:

```shell
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Expected: PASS. If `shellcheck` or `cargo-shear` prerequisites are missing in the local environment, record that in the final handoff instead of treating it as a code failure.

- [x] **Step 5: Inspect final diff for scope**

Run:

```shell
git status --short
git diff --stat
git diff -- docs/2026-06-20-php-track-defaults-design.md DESIGN.md
```

Expected: only PHP defaults, artifact fallback, test, snapshot, and documentation changes are present. The root `php.ini` sample may remain untracked if it was not intentionally added.

- [x] **Step 6: Commit**

```shell
git add DESIGN.md docs/superpowers/plans/2026-06-21-php-track-defaults.md
git commit -m "docs: document PHP track defaults"
```

---

## Final Verification Before Handoff

- [x] Run the focused commands from Task 6 Step 2.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `git diff --check`.
- [x] Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` when local prerequisites are installed.
- [x] Confirm snapshots were accepted intentionally.
- [x] Confirm no code path uses `/usr/local/etc/php` except historical docs/spec discussion or tests proving it is absent.
- [x] Confirm `php.ini` remains seed-only and existing files are preserved.
- [x] Confirm Gateway specs do not receive `PHPRC` or `PHP_INI_SCAN_DIR`.
- [x] Confirm worker validation and worker runtime specs use identical PHP track env values.

## Self-Review Notes

- Spec coverage: Tasks 1-4 cover track defaults, seed-only behavior, CLI/Composer/worker env, and no Caddyfile `php_ini`; Task 5 covers artifact fallback and smoke checks; Task 6 covers `DESIGN.md`.
- Scope: single implementation plan is appropriate because all tasks are coupled through one PHP defaults behavior and are independently testable.
- Ambiguity resolved: Caddyfile `env` is not used for `PHPRC` or `PHP_INI_SCAN_DIR`; process environment is the only runtime path.
