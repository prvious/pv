# PR 24 PHP, FrankenPHP, and Composer Artifact Recipes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add independently mergeable, data-driven artifact recipes for PHP/FrankenPHP and Composer, with cheap local validation and manual native macOS artifact builds in CI.

**Architecture:** Extend `pv-release` just enough to understand recipe metadata, explicit manifest default tracks, fixture archive generation, and shell-friendly recipe environment output. Keep real PHP/FrankenPHP builds in shell recipes that run only in manual native macOS CI. Do not change the public artifact manifest schema or depend on PR 15 PHP shim behavior.

**Tech Stack:** Rust 2024, `pv-release`, `resources::ArtifactManifest`, `toml`, `serde`, `insta`, POSIX shell, `shellcheck`, GitHub Actions macOS runners, `actions/upload-artifact@v7`, static-php-cli, FrankenPHP.

---

## File Structure

- Modify `Cargo.toml`: add workspace `toml` dependency.
- Modify `crates/pv-release/Cargo.toml`: use workspace `toml`.
- Modify `crates/pv-release/src/lib.rs`: export new release-tool modules.
- Modify `crates/pv-release/src/error.rs`: add typed recipe/default-track metadata errors.
- Modify `crates/pv-release/src/cli.rs`: add default-track and recipe subcommands.
- Modify `crates/pv-release/src/manifest.rs`: accept explicit default-track metadata for multi-track resources.
- Create `crates/pv-release/src/defaults.rs`: parse default-track metadata.
- Create `crates/pv-release/src/recipe.rs`: parse PHP/Composer recipe metadata and produce release-record inputs.
- Create `crates/pv-release/src/fixture.rs`: create tiny archives and records for cheap local validation.
- Create `crates/pv-release/tests/manifest_defaults.rs`: default-track manifest generation coverage.
- Create `crates/pv-release/tests/recipe_metadata.rs`: PHP/Composer TOML validation coverage.
- Create `crates/pv-release/tests/recipe_fixtures.rs`: fixture archive, release record, and generated manifest coverage.
- Create `release/artifacts/default-tracks.toml`: explicit public manifest defaults.
- Create `release/artifacts/recipes/common.sh`: shared shell helpers.
- Create `release/artifacts/recipes/php/tracks.toml`: PHP/FrankenPHP track matrix.
- Create `release/artifacts/recipes/php/build.sh`: CI-only PHP/FrankenPHP build recipe.
- Create `release/artifacts/recipes/php/smoke.sh`: CI-only PHP/FrankenPHP smoke checks.
- Create `release/artifacts/recipes/composer/composer.toml`: Composer track metadata.
- Create `release/artifacts/recipes/composer/build.sh`: Composer PHAR packaging recipe.
- Create `release/artifacts/recipes/composer/smoke.sh`: Composer PHAR smoke checks.
- Create `.github/workflows/artifact-recipes.yml`: manual native macOS build workflow.
- Modify `.github/workflows/ci.yml`: add cheap shellcheck and release recipe validation.
- Modify `release/artifacts/README.md`: document local and manual CI commands.

## Task 1: Add Explicit Manifest Default Tracks

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/pv-release/Cargo.toml`
- Modify: `crates/pv-release/src/error.rs`
- Modify: `crates/pv-release/src/lib.rs`
- Modify: `crates/pv-release/src/manifest.rs`
- Modify: `crates/pv-release/src/cli.rs`
- Create: `crates/pv-release/src/defaults.rs`
- Create: `crates/pv-release/tests/manifest_defaults.rs`

- [ ] **Step 1: Write the failing manifest-default test**

Create `crates/pv-release/tests/manifest_defaults.rs` with:

```rust
use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use pv_release::manifest::generate_manifest_file_with_defaults;
use resources::ArtifactManifest;
use serde_json::Value;

#[test]
fn manifest_generator_uses_default_track_metadata_for_multi_track_resources() -> Result<()> {
    let tempdir = tempdir()?;
    let records_dir = tempdir.path().join("records");
    let revocations_dir = tempdir.path().join("revocations");
    let defaults = tempdir.path().join("default-tracks.toml");
    let output = tempdir.path().join("manifests/manifest.json");

    create_dir_all(&records_dir)?;
    create_dir_all(&revocations_dir)?;
    write_file(
        &records_dir.join("php-8.3.31-pv1-darwin-arm64.json"),
        PHP_8_3_RECORD,
    )?;
    write_file(
        &records_dir.join("php-8.4.20-pv1-darwin-arm64.json"),
        PHP_8_4_RECORD,
    )?;
    write_file(
        &defaults,
        r#"
[[resource]]
name = "php"
default_track = "8.4"
"#,
    )?;

    generate_manifest_file_with_defaults(
        &records_dir,
        &revocations_dir,
        Some(&defaults),
        &output,
        "https://artifacts.example.test",
    )?;
    let manifest_json = read_file(&output)?;
    ArtifactManifest::parse(&manifest_json)?;
    let manifest: Value = serde_json::from_str(&manifest_json)?;
    let default_track = manifest["resources"]
        .as_array()
        .and_then(|resources| {
            resources
                .iter()
                .find(|resource| resource["name"].as_str() == Some("php"))
        })
        .and_then(|resource| resource["default_track"].as_str())
        .unwrap_or("");
    assert_eq!(default_track, "8.4");
    assert_snapshot!(manifest_json);

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create local metadata fixtures"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local metadata fixtures"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated local manifests"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

const PHP_8_3_RECORD: &str = r#"{
  "resource": "php",
  "track": "8.3",
  "upstream_version": "8.3.31",
  "pv_build_revision": "pv1",
  "artifact_version": "8.3.31-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/php/8.3/8.3.31-pv1/darwin-arm64/php-8.3.31-pv1-darwin-arm64.tar.gz",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "size": 42,
  "published_at": "2026-06-07T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://www.php.net/distributions/php-8.3.31.tar.gz",
    "source_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "recipe": "release/artifacts/recipes/php/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;

const PHP_8_4_RECORD: &str = r#"{
  "resource": "php",
  "track": "8.4",
  "upstream_version": "8.4.20",
  "pv_build_revision": "pv1",
  "artifact_version": "8.4.20-pv1",
  "platform": "darwin-arm64",
  "object_key": "resources/php/8.4/8.4.20-pv1/darwin-arm64/php-8.4.20-pv1-darwin-arm64.tar.gz",
  "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "size": 42,
  "published_at": "2026-06-07T12:00:00Z",
  "minimum_pv_version": "0.1.0",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "https://www.php.net/distributions/php-8.4.20.tar.gz",
    "source_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "recipe": "release/artifacts/recipes/php/build.sh",
    "pv_commit": "0123456789abcdef0123456789abcdef01234567",
    "build_run_id": "local-test"
  }
}"#;
```

- [ ] **Step 2: Run the failing test**

Run: `cargo nextest run -p pv-release -E 'test(manifest_generator_uses_default_track_metadata_for_multi_track_resources)'`

Expected: FAIL because `pv_release::manifest::generate_manifest_file_with_defaults` does not exist.

- [ ] **Step 3: Add the `toml` dependency precisely**

Modify root `Cargo.toml`:

```toml
[workspace.dependencies]
toml = "1.1.2+spec-1.1.0"
```

Modify `crates/pv-release/Cargo.toml`:

```toml
[dependencies]
toml = { workspace = true }
```

Run: `cargo update -p toml --precise 1.1.2+spec-1.1.0`

Expected: `Cargo.lock` changes only for `toml` and its transitive dependencies.

- [ ] **Step 4: Add the default-track parser**

Create `crates/pv-release/src/defaults.rs`:

```rust
use camino::{Utf8Path, Utf8PathBuf};
use resources::{ResourceName, TrackName};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct ManifestDefaults {
    path: Option<Utf8PathBuf>,
    default_tracks: BTreeMap<String, TrackName>,
}

#[derive(Debug, Deserialize)]
struct RawDefaults {
    #[serde(default)]
    resource: Vec<RawResourceDefault>,
}

#[derive(Debug, Deserialize)]
struct RawResourceDefault {
    name: String,
    default_track: String,
}

impl ManifestDefaults {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_toml(path: &Utf8Path, content: &str) -> crate::Result<Self> {
        let raw: RawDefaults = toml::from_str(content).map_err(|error| invalid(path, error))?;
        let mut default_tracks = BTreeMap::new();

        for resource in raw.resource {
            let name = ResourceName::new(resource.name)
                .map_err(|error| invalid(path, format!("invalid resource name: {error}")))?;
            let track = TrackName::new(resource.default_track)
                .map_err(|error| invalid(path, format!("invalid default track: {error}")))?;
            if default_tracks
                .insert(name.as_str().to_string(), track)
                .is_some()
            {
                return Err(invalid(
                    path,
                    format!("duplicate default track for resource `{name}`"),
                ));
            }
        }

        Ok(Self {
            path: Some(path.to_path_buf()),
            default_tracks,
        })
    }

    pub fn load(path: &Utf8Path) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, &content)
    }

    pub fn default_track_for(&self, resource: &str) -> Option<&TrackName> {
        self.default_tracks.get(resource)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads repository-local default-track metadata"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    })
}

fn invalid(path: &Utf8Path, reason: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::InvalidDefaultTracks {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}
```

- [ ] **Step 5: Export the module and error type**

Modify `crates/pv-release/src/lib.rs`:

```rust
pub mod defaults;
```

Add to `ReleaseError` in `crates/pv-release/src/error.rs`:

```rust
#[error("invalid manifest default tracks `{path}`: {reason}")]
InvalidDefaultTracks { path: String, reason: String },
```

- [ ] **Step 6: Thread defaults through manifest generation**

In `crates/pv-release/src/manifest.rs`, add `use crate::defaults::ManifestDefaults;`.

Add:

```rust
pub fn generate_manifest_file_with_defaults(
    records: &Utf8Path,
    revocations: &Utf8Path,
    defaults: Option<&Utf8Path>,
    output: &Utf8Path,
    base_url: &str,
) -> crate::Result<()> {
    let defaults = match defaults {
        Some(path) => ManifestDefaults::load(path)?,
        None => ManifestDefaults::empty(),
    };
    let records = load_release_records(records)?;
    let revocations = load_revocation_records(revocations)?;
    let manifest = generate_manifest_json_with_defaults(&records, &revocations, &defaults, base_url)?;

    if let Some(parent) = output.parent() {
        create_dir_all(parent)?;
    }
    write(output, &manifest)
}

pub fn generate_manifest_json_with_defaults(
    records: &[ReleaseRecord],
    revocations: &[RevocationRecord],
    defaults: &ManifestDefaults,
    base_url: &str,
) -> crate::Result<String> {
    generate_manifest_json_inner(records, revocations, defaults, base_url)
}
```

Refactor the existing `generate_manifest_json` body into `generate_manifest_json_inner`. Keep `generate_manifest_json` calling the inner helper with `ManifestDefaults::empty()`.

Change `ManifestResourceJson::from_tracks` to accept defaults:

```rust
fn from_tracks(
    name: String,
    tracks: BTreeMap<String, Vec<ManifestArtifactJson>>,
    defaults: &ManifestDefaults,
) -> crate::Result<Self> {
    let default_track = match defaults.default_track_for(&name) {
        Some(track) => {
            if !tracks.contains_key(track.as_str()) {
                return Err(crate::ReleaseError::GeneratedManifestInvalid {
                    reason: format!(
                        "resource `{name}` default_track `{track}` does not match any generated track"
                    ),
                });
            }
            track.as_str().to_string()
        }
        None if tracks.len() == 1 => tracks
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!("resource `{name}` has no tracks"),
            })?,
        None => {
            let track_names = tracks.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(crate::ReleaseError::GeneratedManifestInvalid {
                reason: format!(
                    "resource `{name}` has multiple tracks (`{track_names}`) but no explicit default_track metadata"
                ),
            });
        }
    };

    let tracks = tracks
        .into_iter()
        .map(|(name, artifacts)| ManifestTrackJson { name, artifacts })
        .collect::<Vec<_>>();

    Ok(Self {
        name,
        default_track,
        tracks,
    })
}
```

- [ ] **Step 7: Add CLI support for defaults**

Modify `Command::GenerateManifest` in `crates/pv-release/src/cli.rs`:

```rust
GenerateManifest {
    #[arg(long)]
    records: Utf8PathBuf,
    #[arg(long)]
    revocations: Utf8PathBuf,
    #[arg(long)]
    defaults: Option<Utf8PathBuf>,
    #[arg(long)]
    output: Utf8PathBuf,
    #[arg(long)]
    base_url: String,
},
```

Call:

```rust
crate::manifest::generate_manifest_file_with_defaults(
    &records,
    &revocations,
    defaults.as_deref(),
    &output,
    &base_url,
)
```

Add a CLI parse test that includes `--defaults release/artifacts/default-tracks.toml`.

- [ ] **Step 8: Run the manifest-default tests**

Run: `cargo nextest run -p pv-release -E 'test(manifest_generator_uses_default_track_metadata_for_multi_track_resources)'`

Expected: PASS.

- [ ] **Step 9: Commit Task 1**

```bash
git add Cargo.toml Cargo.lock crates/pv-release/Cargo.toml crates/pv-release/src crates/pv-release/tests/manifest_defaults.rs crates/pv-release/tests/snapshots
git commit -m "feat(release): support manifest default track metadata"
```

## Task 2: Add Recipe Metadata Parsing

**Files:**
- Modify: `crates/pv-release/src/error.rs`
- Modify: `crates/pv-release/src/lib.rs`
- Create: `crates/pv-release/src/recipe.rs`
- Create: `crates/pv-release/tests/recipe_metadata.rs`

- [ ] **Step 1: Write failing recipe metadata tests**

Create `crates/pv-release/tests/recipe_metadata.rs` with tests for valid PHP metadata, valid Composer metadata, duplicate tracks, missing expected extension, and invalid checksum.

Use this test skeleton:

```rust
use anyhow::Result;
use camino::Utf8Path;
use insta::assert_debug_snapshot;
use pv_release::recipe::{ComposerRecipe, PhpRecipe};

#[test]
fn recipe_metadata_parses_php_tracks_and_composer() -> Result<()> {
    let php = PhpRecipe::from_toml(Utf8Path::new("tracks.toml"), VALID_PHP_TOML)?;
    let composer = ComposerRecipe::from_toml(Utf8Path::new("composer.toml"), VALID_COMPOSER_TOML)?;

    assert_debug_snapshot!((
        php_summary(&php),
        composer.track().as_str(),
        composer.upstream_version(),
        composer.platform().as_str(),
    ));
    Ok(())
}

#[test]
fn recipe_metadata_rejects_invalid_shapes() -> Result<()> {
    let duplicate_track = VALID_PHP_TOML.replace(
        "name = \"8.3\"",
        "name = \"8.4\"",
    );
    let missing_extension = VALID_PHP_TOML.replace(
        "\"pdo_mysql\",",
        "",
    );
    let bad_checksum = VALID_COMPOSER_TOML.replace(
        "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06",
        "bad",
    );

    assert_debug_snapshot!((
        PhpRecipe::from_toml(Utf8Path::new("duplicate.toml"), &duplicate_track),
        PhpRecipe::from_toml(Utf8Path::new("missing-extension.toml"), &missing_extension),
        ComposerRecipe::from_toml(Utf8Path::new("bad-composer.toml"), &bad_checksum),
    ));
    Ok(())
}

fn php_summary(recipe: &PhpRecipe) -> Vec<(String, String, String)> {
    recipe
        .tracks()
        .iter()
        .map(|track| {
            (
                track.name().as_str().to_string(),
                track.php_version().to_string(),
                track.php_source_url().to_string(),
            )
        })
        .collect()
}

const VALID_PHP_TOML: &str = r#"
[php]
deployment_target = "13.0"
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
default_track = "8.4"
frankenphp_version = "1.12.3"
frankenphp_source_url = "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz"
frankenphp_source_sha256 = "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363"
build_extensions = ["bcmath", "curl", "intl", "mbstring", "openssl", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "redis", "zip"]
expected_extensions = ["bcmath", "ctype", "curl", "dom", "fileinfo", "filter", "hash", "iconv", "intl", "json", "libxml", "mbstring", "openssl", "pcntl", "pcre", "pdo", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "phar", "posix", "redis", "session", "simplexml", "sodium", "sqlite3", "tokenizer", "xml", "xmlreader", "xmlwriter", "zip", "zlib"]

[[tracks]]
name = "8.3"
php_version = "8.3.31"
php_source_url = "https://www.php.net/distributions/php-8.3.31.tar.gz"
php_source_sha256 = "4e7baaf0a690e954a20e7ced3dd633ce8cb8094e2b6b612a55e703ecbbdcbf4f"

[[tracks]]
name = "8.4"
php_version = "8.4.20"
php_source_url = "https://www.php.net/distributions/php-8.4.20.tar.gz"
php_source_sha256 = "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
"#;

const VALID_COMPOSER_TOML: &str = r#"
[composer]
track = "2"
upstream_version = "2.10.1"
pv_build_revision = "pv1"
platform = "any"
minimum_pv_version = "0.1.0"
source_url = "https://getcomposer.org/download/2.10.1/composer.phar"
source_sha256 = "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]
"#;
```

- [ ] **Step 2: Run the failing recipe metadata tests**

Run: `cargo nextest run -p pv-release -E 'test(recipe_metadata)'`

Expected: FAIL because `pv_release::recipe` does not exist.

- [ ] **Step 3: Add `ReleaseError::InvalidRecipeMetadata`**

Add to `crates/pv-release/src/error.rs`:

```rust
#[error("invalid recipe metadata `{path}`: {reason}")]
InvalidRecipeMetadata { path: String, reason: String },
```

- [ ] **Step 4: Implement `crates/pv-release/src/recipe.rs`**

Create types:

```rust
use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ArtifactPlatform, PvVersion, ResourceName, Sha256Digest, TrackName,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use url::Url;

#[derive(Clone, Debug)]
pub struct PhpRecipe {
    path: Utf8PathBuf,
    deployment_target: String,
    minimum_pv_version: PvVersion,
    pv_build_revision: String,
    default_track: TrackName,
    frankenphp_version: String,
    frankenphp_source_url: String,
    frankenphp_source_sha256: Sha256Digest,
    build_extensions: Vec<String>,
    expected_extensions: Vec<String>,
    tracks: Vec<PhpTrack>,
}

#[derive(Clone, Debug)]
pub struct PhpTrack {
    name: TrackName,
    php_version: String,
    php_source_url: String,
    php_source_sha256: Sha256Digest,
}

#[derive(Clone, Debug)]
pub struct ComposerRecipe {
    path: Utf8PathBuf,
    track: TrackName,
    upstream_version: String,
    pv_build_revision: String,
    platform: ArtifactPlatform,
    minimum_pv_version: PvVersion,
    source_url: String,
    source_sha256: Sha256Digest,
    license_files: Vec<String>,
    notice_files: Vec<String>,
}
```

Deserialize raw TOML into private `RawPhpRecipe`, `RawPhpSettings`, `RawPhpTrack`, `RawComposerRecipe`, and `RawComposerSettings` structs. In `from_toml`, validate:

- URLs are HTTPS and have a host.
- SHA-256 values use `Sha256Digest`.
- track names use `TrackName`.
- PHP track name matches the major/minor prefix of `php_version`.
- `default_track` exists in `tracks`.
- `expected_extensions` contains every extension listed in `DESIGN.md`.
- `build_extensions` is not empty and all values also appear in `expected_extensions`.
- Composer platform is `any`.
- Composer track is `2`.

Add getters used by the tests:

```rust
impl PhpRecipe {
    pub fn from_toml(path: &Utf8Path, content: &str) -> crate::Result<Self> {
        let raw: RawPhpRecipe = toml::from_str(content).map_err(|error| invalid(path, error))?;
        Self::from_raw(path, raw)
    }

    pub fn load(path: &Utf8Path) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, &content)
    }

    pub fn tracks(&self) -> &[PhpTrack] {
        &self.tracks
    }

    pub fn default_track(&self) -> &TrackName {
        &self.default_track
    }
}

impl PhpTrack {
    pub fn name(&self) -> &TrackName {
        &self.name
    }

    pub fn php_version(&self) -> &str {
        &self.php_version
    }

    pub fn php_source_url(&self) -> &str {
        &self.php_source_url
    }
}

impl ComposerRecipe {
    pub fn from_toml(path: &Utf8Path, content: &str) -> crate::Result<Self> {
        let raw: RawComposerRecipe =
            toml::from_str(content).map_err(|error| invalid(path, error))?;
        Self::from_raw(path, raw)
    }

    pub fn load(path: &Utf8Path) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, &content)
    }

    pub fn track(&self) -> &TrackName {
        &self.track
    }

    pub fn upstream_version(&self) -> &str {
        &self.upstream_version
    }

    pub fn platform(&self) -> ArtifactPlatform {
        self.platform
    }
}
```

- [ ] **Step 5: Export the recipe module**

Modify `crates/pv-release/src/lib.rs`:

```rust
pub mod recipe;
```

- [ ] **Step 6: Run metadata tests**

Run: `cargo nextest run -p pv-release -E 'test(recipe_metadata)'`

Expected: PASS. If snapshots are created, inspect and accept only the intended recipe metadata snapshots:

```bash
cargo insta accept --manifest-path crates/pv-release/Cargo.toml
```

- [ ] **Step 7: Commit Task 2**

```bash
git add crates/pv-release/src crates/pv-release/tests/recipe_metadata.rs crates/pv-release/tests/snapshots
git commit -m "feat(release): parse artifact recipe metadata"
```

## Task 3: Add Committed Recipe Metadata

**Files:**
- Create: `release/artifacts/default-tracks.toml`
- Create: `release/artifacts/recipes/php/tracks.toml`
- Create: `release/artifacts/recipes/composer/composer.toml`
- Modify: `release/artifacts/README.md`
- Modify: `crates/pv-release/tests/recipe_metadata.rs`

- [ ] **Step 1: Add committed default tracks**

Create `release/artifacts/default-tracks.toml`:

```toml
[[resource]]
name = "php"
default_track = "8.4"

[[resource]]
name = "frankenphp"
default_track = "8.4"

[[resource]]
name = "composer"
default_track = "2"
```

- [ ] **Step 2: Add PHP/FrankenPHP track matrix**

Create `release/artifacts/recipes/php/tracks.toml`:

```toml
[php]
deployment_target = "13.0"
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
default_track = "8.4"
frankenphp_version = "1.12.3"
frankenphp_source_url = "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz"
frankenphp_source_sha256 = "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363"
build_extensions = [
  "bcmath",
  "curl",
  "intl",
  "mbstring",
  "openssl",
  "pcntl",
  "pdo_mysql",
  "pdo_pgsql",
  "pdo_sqlite",
  "redis",
  "sodium",
  "zip",
]
expected_extensions = [
  "bcmath",
  "ctype",
  "curl",
  "dom",
  "fileinfo",
  "filter",
  "hash",
  "iconv",
  "intl",
  "json",
  "libxml",
  "mbstring",
  "openssl",
  "pcntl",
  "pcre",
  "pdo",
  "pdo_mysql",
  "pdo_pgsql",
  "pdo_sqlite",
  "phar",
  "posix",
  "redis",
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

[[tracks]]
name = "8.2"
php_version = "8.2.31"
php_source_url = "https://www.php.net/distributions/php-8.2.31.tar.gz"
php_source_sha256 = "083c2f61cc5f527eb293c4c468a91af46a9678785957e023b2796a9db290d870"

[[tracks]]
name = "8.3"
php_version = "8.3.31"
php_source_url = "https://www.php.net/distributions/php-8.3.31.tar.gz"
php_source_sha256 = "4e7baaf0a690e954a20e7ced3dd633ce8cb8094e2b6b612a55e703ecbbdcbf4f"

[[tracks]]
name = "8.4"
php_version = "8.4.20"
php_source_url = "https://www.php.net/distributions/php-8.4.20.tar.gz"
php_source_sha256 = "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
```

- [ ] **Step 3: Add Composer metadata**

Create `release/artifacts/recipes/composer/composer.toml`:

```toml
[composer]
track = "2"
upstream_version = "2.10.1"
pv_build_revision = "pv1"
platform = "any"
minimum_pv_version = "0.1.0"
source_url = "https://getcomposer.org/download/2.10.1/composer.phar"
source_sha256 = "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]
```

- [ ] **Step 4: Add committed metadata coverage**

Extend `crates/pv-release/tests/recipe_metadata.rs`:

```rust
#[test]
fn committed_recipe_metadata_parses() -> Result<()> {
    let php = PhpRecipe::load(Utf8Path::new(
        "release/artifacts/recipes/php/tracks.toml",
    ))?;
    let composer = ComposerRecipe::load(Utf8Path::new(
        "release/artifacts/recipes/composer/composer.toml",
    ))?;

    assert_eq!(php.default_track().as_str(), "8.4");
    assert_eq!(php.tracks().len(), 3);
    assert_eq!(composer.track().as_str(), "2");
    assert_eq!(composer.platform().as_str(), "any");

    Ok(())
}
```

- [ ] **Step 5: Document the metadata files**

Append to `release/artifacts/README.md`:

```markdown
## Recipes

`recipes/php/tracks.toml` is the data source for PHP and FrankenPHP artifact builds. It pins PHP tracks, source URLs, checksums, the expected extension set, the macOS deployment target, and the FrankenPHP source version used by the recipe.

`recipes/composer/composer.toml` is the data source for Composer track `2`. Composer is packaged as a `platform: "any"` artifact.

`default-tracks.toml` gives the manifest generator explicit default tracks for resources with more than one generated track.
```

- [ ] **Step 6: Run committed metadata tests**

Run: `cargo nextest run -p pv-release -E 'test(committed_recipe_metadata_parses)'`

Expected: PASS.

- [ ] **Step 7: Commit Task 3**

```bash
git add release/artifacts crates/pv-release/tests/recipe_metadata.rs
git commit -m "feat(release): add PHP and Composer recipe metadata"
```

## Task 4: Add Cheap Fixture Generation

**Files:**
- Modify: `crates/pv-release/src/cli.rs`
- Modify: `crates/pv-release/src/lib.rs`
- Modify: `crates/pv-release/src/recipe.rs`
- Create: `crates/pv-release/src/fixture.rs`
- Create: `crates/pv-release/tests/recipe_fixtures.rs`

- [ ] **Step 1: Write failing fixture generation test**

Create `crates/pv-release/tests/recipe_fixtures.rs`:

```rust
use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use pv_release::fixture::generate_recipe_fixtures;
use pv_release::manifest::generate_manifest_file_with_defaults;
use resources::ArtifactManifest;

#[test]
fn recipe_fixture_generation_validates_archives_records_and_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let archives = tempdir.path().join("archives");
    let records = tempdir.path().join("records");
    let manifest = tempdir.path().join("manifest.json");

    generate_recipe_fixtures(
        Utf8Path::new("release/artifacts/recipes/php/tracks.toml"),
        Utf8Path::new("release/artifacts/recipes/composer/composer.toml"),
        &archives,
        &records,
        "0123456789abcdef0123456789abcdef01234567",
        "local-test",
    )?;
    generate_manifest_file_with_defaults(
        &records,
        Utf8Path::new("release/artifacts/revocations"),
        Some(Utf8Path::new("release/artifacts/default-tracks.toml")),
        &manifest,
        "https://artifacts.example.test",
    )?;
    let manifest_json = read_file(&manifest)?;
    ArtifactManifest::parse(&manifest_json)?;

    assert_snapshot!(manifest_json);
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated local manifests"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
```

- [ ] **Step 2: Run the failing fixture test**

Run: `cargo nextest run -p pv-release -E 'test(recipe_fixture_generation_validates_archives_records_and_manifest)'`

Expected: FAIL because `pv_release::fixture` does not exist.

- [ ] **Step 3: Implement fixture archive and record generation**

Create `crates/pv-release/src/fixture.rs` that:

- loads PHP and Composer metadata
- creates one tiny archive per PHP track for resource `php`
- creates one tiny archive per PHP track for resource `frankenphp`
- creates one tiny archive for resource `composer`
- writes matching JSON release records
- validates each archive with `validate_archive_for_record_file`

Use archive roots:

```text
php-8.4.20-pv1/
frankenphp-8.4.20-frankenphp1.12.3-pv1/
composer-2.10.1-pv1/
```

Use fixture file layouts:

```text
php-*/LICENSE
php-*/NOTICE
php-*/bin/php

frankenphp-*/LICENSE
frankenphp-*/NOTICE
frankenphp-*/bin/frankenphp

composer-*/LICENSE
composer-*/NOTICE
composer-*/composer.phar
```

For `frankenphp`, generate `upstream_version` as `"{php_version}-frankenphp{frankenphp_version}"`, for example `8.4.20-frankenphp1.12.3`. This keeps the manifest schema unchanged while preserving the PHP patch and FrankenPHP source version in the artifact version.

- [ ] **Step 4: Add CLI command for cheap local validation**

Add to `crates/pv-release/src/cli.rs`:

```rust
GenerateRecipeFixtures {
    #[arg(long)]
    php: Utf8PathBuf,
    #[arg(long)]
    composer: Utf8PathBuf,
    #[arg(long)]
    archives: Utf8PathBuf,
    #[arg(long)]
    records: Utf8PathBuf,
    #[arg(long)]
    pv_commit: String,
    #[arg(long)]
    build_run_id: String,
},
```

Dispatch to:

```rust
crate::fixture::generate_recipe_fixtures(
    &php,
    &composer,
    &archives,
    &records,
    &pv_commit,
    &build_run_id,
)
```

- [ ] **Step 5: Export fixture module**

Modify `crates/pv-release/src/lib.rs`:

```rust
pub mod fixture;
```

- [ ] **Step 6: Run fixture generation test**

Run: `cargo nextest run -p pv-release -E 'test(recipe_fixture_generation_validates_archives_records_and_manifest)'`

Expected: PASS. Accept the intended manifest snapshot:

```bash
cargo insta accept --manifest-path crates/pv-release/Cargo.toml
```

- [ ] **Step 7: Commit Task 4**

```bash
git add crates/pv-release/src crates/pv-release/tests/recipe_fixtures.rs crates/pv-release/tests/snapshots
git commit -m "feat(release): generate recipe validation fixtures"
```

## Task 5: Add Shell Recipe Helpers and Composer Recipe

**Files:**
- Create: `release/artifacts/recipes/common.sh`
- Create: `release/artifacts/recipes/composer/build.sh`
- Create: `release/artifacts/recipes/composer/smoke.sh`

- [ ] **Step 1: Create common shell helpers**

Create `release/artifacts/recipes/common.sh`:

```sh
#!/bin/sh
set -eu

die() {
  printf '%s\n' "error: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

file_size() {
  stat -f '%z' "$1"
}

require_sha256() {
  file=$1
  expected=$2
  actual=$(sha256_file "$file")
  [ "$actual" = "$expected" ] || die "$file checksum mismatch: expected $expected, got $actual"
}

write_record() {
  record_path=$1
  resource=$2
  track=$3
  upstream_version=$4
  pv_build_revision=$5
  platform=$6
  object_key=$7
  archive=$8
  source_url=$9
  source_sha256=${10}
  recipe=${11}
  pv_commit=${12}
  build_run_id=${13}
  minimum_pv_version=${14}

  artifact_version="${upstream_version}-${pv_build_revision}"
  sha256=$(sha256_file "$archive")
  size=$(file_size "$archive")
  published_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  mkdir -p "$(dirname "$record_path")"
  cat >"$record_path" <<JSON
{
  "resource": "$resource",
  "track": "$track",
  "upstream_version": "$upstream_version",
  "pv_build_revision": "$pv_build_revision",
  "artifact_version": "$artifact_version",
  "platform": "$platform",
  "object_key": "$object_key",
  "sha256": "$sha256",
  "size": $size,
  "published_at": "$published_at",
  "minimum_pv_version": "$minimum_pv_version",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "$source_url",
    "source_sha256": "$source_sha256",
    "recipe": "$recipe",
    "pv_commit": "$pv_commit",
    "build_run_id": "$build_run_id"
  }
}
JSON
}
```

- [ ] **Step 2: Create Composer build script**

Create executable `release/artifacts/recipes/composer/build.sh`:

```sh
#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)
. "$ROOT/release/artifacts/recipes/common.sh"

TRACK=2
VERSION=2.10.1
PV_BUILD_REVISION=pv1
PLATFORM=any
SOURCE_URL=https://getcomposer.org/download/2.10.1/composer.phar
SOURCE_SHA256=345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06
MINIMUM_PV_VERSION=0.1.0

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
PV_COMMIT=${PV_COMMIT:-$(git -C "$ROOT" rev-parse HEAD)}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-composer}

need curl
need shasum
need tar

artifact_version="${VERSION}-${PV_BUILD_REVISION}"
work_dir="$OUT_DIR/work/composer-$artifact_version"
root_dir="$work_dir/composer-$artifact_version"
archive="$OUT_DIR/composer-$artifact_version.tar.gz"
record="$RECORD_DIR/composer/$TRACK/$artifact_version/$PLATFORM/composer-$artifact_version-$PLATFORM.json"
object_key="resources/composer/$TRACK/$artifact_version/$PLATFORM/composer-$artifact_version-$PLATFORM.tar.gz"

rm -rf "$work_dir"
mkdir -p "$root_dir"
curl -L --fail --show-error --silent "$SOURCE_URL" -o "$root_dir/composer.phar"
require_sha256 "$root_dir/composer.phar" "$SOURCE_SHA256"
cat >"$root_dir/LICENSE" <<'TEXT'
Composer PHAR license metadata is provided by the upstream Composer project.
TEXT
cat >"$root_dir/NOTICE" <<'TEXT'
Packaged by PV for Composer track 2.
TEXT
mkdir -p "$OUT_DIR"
tar -czf "$archive" -C "$work_dir" "composer-$artifact_version"

write_record "$record" composer "$TRACK" "$VERSION" "$PV_BUILD_REVISION" "$PLATFORM" "$object_key" "$archive" "$SOURCE_URL" "$SOURCE_SHA256" release/artifacts/recipes/composer/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$MINIMUM_PV_VERSION"

cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$ROOT/release/artifacts/recipes/composer/smoke.sh"
printf '%s\n' "$archive"
```

- [ ] **Step 3: Create Composer smoke script**

Create executable `release/artifacts/recipes/composer/smoke.sh`:

```sh
#!/bin/sh
set -eu

artifact_root=$1
php_binary=${PV_COMPOSER_SMOKE_PHP:-}
[ -n "$php_binary" ] || {
  printf '%s\n' "composer smoke skipped: PV_COMPOSER_SMOKE_PHP is not set" >&2
  exit 0
}

[ -f "$artifact_root/composer.phar" ] || {
  printf '%s\n' "missing composer.phar in $artifact_root" >&2
  exit 42
}

"$php_binary" "$artifact_root/composer.phar" --version >/tmp/pv-composer-smoke.txt
grep 'Composer version 2.10.1' /tmp/pv-composer-smoke.txt >/dev/null
```

- [ ] **Step 4: Mark scripts executable and run shellcheck**

Run:

```bash
chmod +x release/artifacts/recipes/common.sh release/artifacts/recipes/composer/build.sh release/artifacts/recipes/composer/smoke.sh
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/composer/build.sh release/artifacts/recipes/composer/smoke.sh
```

Expected: shellcheck exits 0.

- [ ] **Step 5: Run Composer package smoke locally**

Run:

```bash
PV_ARTIFACT_OUT_DIR=/tmp/pv-composer-artifacts \
PV_ARTIFACT_RECORD_DIR=/tmp/pv-composer-records \
release/artifacts/recipes/composer/build.sh
```

Expected: prints `/tmp/pv-composer-artifacts/composer-2.10.1-pv1.tar.gz` and `pv-release validate-archive` exits 0.

- [ ] **Step 6: Commit Task 5**

```bash
git add release/artifacts/recipes
git commit -m "feat(release): add Composer artifact recipe"
```

## Task 6: Add PHP and FrankenPHP CI-Only Recipes

**Files:**
- Modify: `crates/pv-release/src/cli.rs`
- Modify: `crates/pv-release/src/recipe.rs`
- Create: `release/artifacts/recipes/php/build.sh`
- Create: `release/artifacts/recipes/php/smoke.sh`

- [ ] **Step 1: Add shell environment output for PHP tracks**

Add a `PrintRecipeEnv` CLI command:

```rust
PrintRecipeEnv {
    #[arg(long)]
    php: Utf8PathBuf,
    #[arg(long)]
    resource: String,
    #[arg(long)]
    track: String,
    #[arg(long)]
    platform: String,
},
```

For `resource=php`, print shell assignments:

```text
PV_RESOURCE=php
PV_TRACK=8.4
PV_PLATFORM=darwin-arm64
PV_PHP_VERSION=8.4.20
PV_UPSTREAM_VERSION=8.4.20
PV_ARTIFACT_VERSION=8.4.20-pv1
PV_SOURCE_URL=https://www.php.net/distributions/php-8.4.20.tar.gz
PV_SOURCE_SHA256=a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d
PV_BUILD_EXTENSIONS=bcmath,curl,intl,mbstring,openssl,pcntl,pdo_mysql,pdo_pgsql,pdo_sqlite,redis,sodium,zip
PV_EXPECTED_EXTENSIONS=bcmath,ctype,curl,dom,fileinfo,filter,hash,iconv,intl,json,libxml,mbstring,openssl,pcntl,pcre,pdo,pdo_mysql,pdo_pgsql,pdo_sqlite,phar,posix,redis,session,simplexml,sodium,sqlite3,tokenizer,xml,xmlreader,xmlwriter,zip,zlib
PV_MINIMUM_PV_VERSION=0.1.0
PV_PV_BUILD_REVISION=pv1
```

For `resource=frankenphp`, print the same track and expected extension values with:

```text
PV_RESOURCE=frankenphp
PV_PHP_VERSION=8.4.20
PV_UPSTREAM_VERSION=8.4.20-frankenphp1.12.3
PV_SOURCE_URL=https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz
PV_SOURCE_SHA256=2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363
```

- [ ] **Step 2: Test `print-recipe-env`**

Add a test in `crates/pv-release/tests/recipe_metadata.rs` that calls the Rust helper behind `PrintRecipeEnv` for `php` and `frankenphp` track `8.4` and snapshots the output.

Run: `cargo nextest run -p pv-release -E 'test(print_recipe_env)'`

Expected: PASS after accepting intended snapshots.

- [ ] **Step 3: Create PHP/FrankenPHP build script**

Create executable `release/artifacts/recipes/php/build.sh`:

```sh
#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)
. "$ROOT/release/artifacts/recipes/common.sh"

RESOURCE=${PV_RECIPE_RESOURCE:-php}
TRACK=${PV_RECIPE_TRACK:-8.4}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
PV_COMMIT=${PV_COMMIT:-$(git -C "$ROOT" rev-parse HEAD)}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-php}

case "$RESOURCE" in
  php|frankenphp) ;;
  *) die "PV_RECIPE_RESOURCE must be php or frankenphp, got $RESOURCE" ;;
esac

need cargo
need curl
need shasum
need tar
need spc

env_file="$OUT_DIR/work/$RESOURCE-$TRACK-$PLATFORM.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --php "$ROOT/release/artifacts/recipes/php/tracks.toml" \
  --resource "$RESOURCE" \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
. "$env_file"
export PV_EXPECTED_EXTENSIONS
export PV_PHP_VERSION
export PV_UPSTREAM_VERSION

work_dir="$OUT_DIR/work/$RESOURCE-$PV_ARTIFACT_VERSION-$PV_PLATFORM"
root_dir="$work_dir/$RESOURCE-$PV_ARTIFACT_VERSION"
archive="$OUT_DIR/$RESOURCE-$PV_ARTIFACT_VERSION-$PV_PLATFORM.tar.gz"
record="$RECORD_DIR/$RESOURCE/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/$RESOURCE-$PV_ARTIFACT_VERSION-$PV_PLATFORM.json"
object_key="resources/$RESOURCE/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/$RESOURCE-$PV_ARTIFACT_VERSION-$PV_PLATFORM.tar.gz"

rm -rf "$work_dir"
mkdir -p "$root_dir/bin" "$OUT_DIR/sources"

source_archive="$OUT_DIR/sources/$RESOURCE-$PV_ARTIFACT_VERSION-source.tar.gz"
curl -L --fail --show-error --silent "$PV_SOURCE_URL" -o "$source_archive"
require_sha256 "$source_archive" "$PV_SOURCE_SHA256"

export MACOSX_DEPLOYMENT_TARGET=13.0

case "$RESOURCE" in
  php)
    spc download --with-php="$PV_PHP_VERSION" --for-extensions="$PV_BUILD_EXTENSIONS"
    spc build "$PV_BUILD_EXTENSIONS" --build-cli --output-dir "$root_dir/bin"
    [ -f "$root_dir/bin/php" ] || die "static PHP build did not produce bin/php"
    ;;
  frankenphp)
    spc download --with-php="$PV_PHP_VERSION" --for-extensions="$PV_BUILD_EXTENSIONS"
    spc build frankenphp,"$PV_BUILD_EXTENSIONS" --build-cli --output-dir "$root_dir/bin"
    [ -f "$root_dir/bin/frankenphp" ] || die "FrankenPHP build did not produce bin/frankenphp"
    ;;
esac

cat >"$root_dir/LICENSE" <<'TEXT'
PV packages upstream PHP and FrankenPHP sources. See upstream project licenses for full terms.
TEXT
cat >"$root_dir/NOTICE" <<TEXT
Resource: $RESOURCE
Track: $PV_TRACK
Artifact version: $PV_ARTIFACT_VERSION
TEXT

tar -czf "$archive" -C "$work_dir" "$RESOURCE-$PV_ARTIFACT_VERSION"
write_record "$record" "$RESOURCE" "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/php/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION"

cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$ROOT/release/artifacts/recipes/php/smoke.sh"
printf '%s\n' "$archive"
```

- [ ] **Step 4: Create PHP/FrankenPHP smoke script**

Create executable `release/artifacts/recipes/php/smoke.sh`:

```sh
#!/bin/sh
set -eu

artifact_root=$1
expected_extensions=${PV_EXPECTED_EXTENSIONS:-}
expected_version=${PV_UPSTREAM_VERSION%%-frankenphp*}

if [ -x "$artifact_root/bin/php" ]; then
  php_binary="$artifact_root/bin/php"
  "$php_binary" -v | grep "PHP $expected_version" >/dev/null
  actual_extensions=$("$php_binary" -m | awk 'NF {print $0}' | sort | tr '\n' ',')
  IFS=,
  for extension in $expected_extensions; do
    printf '%s' "$actual_extensions" | grep "$extension," >/dev/null || {
      printf '%s\n' "missing PHP extension: $extension" >&2
      exit 43
    }
  done
  exit 0
fi

if [ -x "$artifact_root/bin/frankenphp" ]; then
  frankenphp_binary="$artifact_root/bin/frankenphp"
  "$frankenphp_binary" php-cli -v | grep "PHP $expected_version" >/dev/null
  site_dir=$(mktemp -d)
  port_file="$site_dir/port"
  cat >"$site_dir/index.php" <<'PHP'
<?php echo "pv-frankenphp-ok";
PHP
  port=48123
  "$frankenphp_binary" php-server --listen "127.0.0.1:$port" --root "$site_dir" &
  pid=$!
  trap 'kill "$pid" 2>/dev/null || true; rm -rf "$site_dir"' EXIT
  printf '%s\n' "$port" >"$port_file"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl --fail --silent "http://127.0.0.1:$port/" | grep pv-frankenphp-ok >/dev/null; then
      exit 0
    fi
    sleep 1
  done
  printf '%s\n' "FrankenPHP loopback smoke failed" >&2
  exit 44
fi

printf '%s\n' "artifact root has neither bin/php nor bin/frankenphp" >&2
exit 45
```

- [ ] **Step 5: Mark scripts executable and run shellcheck**

Run:

```bash
chmod +x release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh
shellcheck release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh
```

Expected: shellcheck exits 0.

- [ ] **Step 6: Run recipe-env tests**

Run: `cargo nextest run -p pv-release -E 'test(print_recipe_env)'`

Expected: PASS.

- [ ] **Step 7: Commit Task 6**

```bash
git add crates/pv-release/src crates/pv-release/tests release/artifacts/recipes/php
git commit -m "feat(release): add PHP and FrankenPHP artifact recipes"
```

## Task 7: Add Manual Native Artifact Workflow

**Files:**
- Create: `.github/workflows/artifact-recipes.yml`

- [ ] **Step 1: Add manual workflow**

Create `.github/workflows/artifact-recipes.yml`:

```yaml
name: Artifact Recipes

on:
  workflow_dispatch:
    inputs:
      resource:
        description: "Resource to build: all, php, frankenphp, composer"
        required: true
        default: "all"
        type: choice
        options:
          - all
          - php
          - frankenphp
          - composer
      track:
        description: "Track to build: all, 8.2, 8.3, 8.4, 2"
        required: true
        default: "all"
        type: string
      platform:
        description: "Artifact platform"
        required: true
        default: "darwin-arm64"
        type: choice
        options:
          - darwin-arm64
          - darwin-amd64

jobs:
  build:
    runs-on: ${{ inputs.platform == 'darwin-amd64' && 'macos-13' || 'macos-14' }}
    steps:
      - name: Checkout
        uses: actions/checkout@v6

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install release tooling dependencies
        run: |
          brew update
          brew install shellcheck php
          curl -L --fail --show-error --silent https://dl.static-php.dev/static-php-cli/spc-bin/nightly/spc-macos-$(uname -m) -o /usr/local/bin/spc
          chmod +x /usr/local/bin/spc

      - name: Validate recipe metadata
        run: |
          cargo run -p pv-release -- generate-recipe-fixtures \
            --php release/artifacts/recipes/php/tracks.toml \
            --composer release/artifacts/recipes/composer/composer.toml \
            --archives /tmp/pv-recipe-fixtures/archives \
            --records /tmp/pv-recipe-fixtures/records \
            --pv-commit "$(git rev-parse HEAD)" \
            --build-run-id "${{ github.run_id }}"
          cargo run -p pv-release -- generate-manifest \
            --records /tmp/pv-recipe-fixtures/records \
            --revocations release/artifacts/revocations \
            --defaults release/artifacts/default-tracks.toml \
            --output /tmp/pv-recipe-fixtures/manifest.json \
            --base-url https://artifacts.example.test

      - name: Build selected artifacts
        env:
          PV_RECIPE_RESOURCE: ${{ inputs.resource }}
          PV_RECIPE_TRACK: ${{ inputs.track }}
          PV_RECIPE_PLATFORM: ${{ inputs.platform }}
          PV_ARTIFACT_OUT_DIR: ${{ runner.temp }}/pv-artifacts
          PV_ARTIFACT_RECORD_DIR: ${{ runner.temp }}/pv-records
          PV_COMMIT: ${{ github.sha }}
          PV_BUILD_RUN_ID: ${{ github.run_id }}
        run: |
          set -eu
          resources="$PV_RECIPE_RESOURCE"
          tracks="$PV_RECIPE_TRACK"
          if [ "$resources" = all ]; then
            resources="php frankenphp composer"
          fi
          if [ "$tracks" = all ]; then
            tracks="8.2 8.3 8.4"
          fi
          for resource in $resources; do
            if [ "$resource" = composer ]; then
              PV_RECIPE_RESOURCE=composer PV_COMPOSER_SMOKE_PHP="$(command -v php)" release/artifacts/recipes/composer/build.sh
              continue
            fi
            for track in $tracks; do
              PV_RECIPE_RESOURCE="$resource" PV_RECIPE_TRACK="$track" release/artifacts/recipes/php/build.sh
            done
          done

      - name: Generate manifest from records
        run: |
          cargo run -p pv-release -- generate-manifest \
            --records "${{ runner.temp }}/pv-records" \
            --revocations release/artifacts/revocations \
            --defaults release/artifacts/default-tracks.toml \
            --output "${{ runner.temp }}/pv-artifacts/manifest.json" \
            --base-url https://artifacts.example.test

      - name: Upload artifact archives and records
        uses: actions/upload-artifact@v7
        with:
          name: pv-artifact-recipes-${{ inputs.resource }}-${{ inputs.track }}-${{ inputs.platform }}-${{ github.run_id }}
          path: |
            ${{ runner.temp }}/pv-artifacts
            ${{ runner.temp }}/pv-records
          if-no-files-found: error
```

- [ ] **Step 2: Commit Task 7**

```bash
git add .github/workflows/artifact-recipes.yml
git commit -m "ci: add manual artifact recipe workflow"
```

## Task 8: Add Cheap Recipe Validation to Normal CI

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `release/artifacts/README.md`

- [ ] **Step 1: Add shellcheck and fixture validation to CI**

Modify `.github/workflows/ci.yml` after `Install cargo-shear`:

```yaml
      - name: Install shellcheck
        run: brew install shellcheck
```

Add after `Check unused dependencies`:

```yaml
      - name: Check artifact recipe scripts
        run: shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh

      - name: Validate artifact recipe metadata and fixtures
        run: |
          cargo run -p pv-release -- generate-recipe-fixtures \
            --php release/artifacts/recipes/php/tracks.toml \
            --composer release/artifacts/recipes/composer/composer.toml \
            --archives /tmp/pv-recipe-fixtures/archives \
            --records /tmp/pv-recipe-fixtures/records \
            --pv-commit "$(git rev-parse HEAD)" \
            --build-run-id ci
          cargo run -p pv-release -- generate-manifest \
            --records /tmp/pv-recipe-fixtures/records \
            --revocations release/artifacts/revocations \
            --defaults release/artifacts/default-tracks.toml \
            --output /tmp/pv-recipe-fixtures/manifest.json \
            --base-url https://artifacts.example.test
```

- [ ] **Step 2: Document local commands**

Append to `release/artifacts/README.md`:

```markdown
## Local Validation

Local validation does not build real PHP or FrankenPHP:

```shell
cargo run -p pv-release -- generate-recipe-fixtures \
  --php release/artifacts/recipes/php/tracks.toml \
  --composer release/artifacts/recipes/composer/composer.toml \
  --archives /tmp/pv-recipe-fixtures/archives \
  --records /tmp/pv-recipe-fixtures/records \
  --pv-commit "$(git rev-parse HEAD)" \
  --build-run-id local

cargo run -p pv-release -- generate-manifest \
  --records /tmp/pv-recipe-fixtures/records \
  --revocations release/artifacts/revocations \
  --defaults release/artifacts/default-tracks.toml \
  --output /tmp/pv-recipe-fixtures/manifest.json \
  --base-url https://artifacts.example.test
```

Real PHP/FrankenPHP artifacts are built only by the manual `Artifact Recipes` GitHub Actions workflow on native macOS runners.
```

- [ ] **Step 3: Run cheap validation locally**

Run:

```bash
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh
cargo run -p pv-release -- generate-recipe-fixtures --php release/artifacts/recipes/php/tracks.toml --composer release/artifacts/recipes/composer/composer.toml --archives /tmp/pv-recipe-fixtures/archives --records /tmp/pv-recipe-fixtures/records --pv-commit "$(git rev-parse HEAD)" --build-run-id local
cargo run -p pv-release -- generate-manifest --records /tmp/pv-recipe-fixtures/records --revocations release/artifacts/revocations --defaults release/artifacts/default-tracks.toml --output /tmp/pv-recipe-fixtures/manifest.json --base-url https://artifacts.example.test
```

Expected: all commands exit 0.

- [ ] **Step 4: Commit Task 8**

```bash
git add .github/workflows/ci.yml release/artifacts/README.md
git commit -m "ci: validate artifact recipes cheaply"
```

## Task 9: Final Verification and Roadmap Hygiene

**Files:**
- Modify only if implementation changes require it: `IMPLEMENTATION.md`

- [ ] **Step 1: Run formatting**

Run: `cargo fmt --all --check`

Expected: PASS.

- [ ] **Step 2: Run focused release tests**

Run: `cargo nextest run -p pv-release --locked`

Expected: all `pv-release` tests pass.

- [ ] **Step 3: Run relevant resources tests**

Run: `cargo nextest run -p resources -E 'test(manifest)' --locked`

Expected: manifest-related `resources` tests pass.

- [ ] **Step 4: Run shellcheck**

Run:

```bash
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh
```

Expected: shellcheck exits 0.

- [ ] **Step 5: Run cheap recipe validation**

Run:

```bash
rm -rf /tmp/pv-recipe-fixtures
cargo run -p pv-release -- generate-recipe-fixtures --php release/artifacts/recipes/php/tracks.toml --composer release/artifacts/recipes/composer/composer.toml --archives /tmp/pv-recipe-fixtures/archives --records /tmp/pv-recipe-fixtures/records --pv-commit "$(git rev-parse HEAD)" --build-run-id local
cargo run -p pv-release -- generate-manifest --records /tmp/pv-recipe-fixtures/records --revocations release/artifacts/revocations --defaults release/artifacts/default-tracks.toml --output /tmp/pv-recipe-fixtures/manifest.json --base-url https://artifacts.example.test
```

Expected: both commands exit 0 and `/tmp/pv-recipe-fixtures/manifest.json` parses through `resources`.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`

Expected: PASS.

- [ ] **Step 7: Run full workspace tests if focused checks pass**

Run: `cargo nextest run --workspace --locked`

Expected: all tests pass.

- [ ] **Step 8: Review roadmap row**

Open `IMPLEMENTATION.md` and confirm PR 24 row still says:

```markdown
| PR 24  | PHP/FrankenPHP and Composer artifact recipes | PV-112, PV-113 | PR 23 | Yes, blocks public setup artifacts | No |
```

Do not mark the row done until the PR is merged.

- [ ] **Step 9: Commit final verification updates if any**

If verification caused snapshot acceptance or doc corrections:

```bash
git add <changed-files>
git commit -m "test(release): verify artifact recipe outputs"
```

If there are no changes, do not create an empty commit.

## Manual CI Check Before PR Review

After pushing the branch, run the manual `Artifact Recipes` workflow for:

- `resource=composer`, `track=2`, `platform=darwin-arm64`
- `resource=php`, `track=8.4`, `platform=darwin-arm64`
- `resource=frankenphp`, `track=8.4`, `platform=darwin-arm64`

If those pass, run `resource=all`, `track=all`, `platform=darwin-arm64`. Run `darwin-amd64` when GitHub-hosted Intel macOS capacity is available for this repository. Uploads remain GitHub Actions artifacts and are not public release publication.
