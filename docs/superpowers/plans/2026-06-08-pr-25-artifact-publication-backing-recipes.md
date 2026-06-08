# PR 25 Artifact Publication and Backing Resource Recipes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Cloudflare R2 publication for validated Managed Resource artifact recipe outputs and add backing resource recipes for Redis, MySQL, Postgres, Mailpit, and RustFS.

**Architecture:** Keep artifact building and artifact publication as separate manual GitHub Actions workflows. Extend `pv-release` with shared recipe metadata, fixture, archive staging, publication-plan, and manifest validation helpers; keep each backing resource in its own recipe directory with resource-specific build and smoke scripts. Use Cloudflare R2 only through S3-compatible CLI operations from the publication workflow, with stable `manifest.json` overwritten last.

**Tech Stack:** Rust 2024, `pv-release`, `resources::ArtifactManifest`, `toml`, `serde`, `serde_json`, `insta`, POSIX shell, `shellcheck`, GitHub Actions, `gh run download`, AWS CLI S3 commands against Cloudflare R2, native macOS runners.

---

## Accepted Product Decisions

- Publication target: Cloudflare R2.
- Upload endpoint: `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`.
- R2 signing region: `auto`.
- Stable client entrypoint: direct full manifest JSON at `manifest.json`.
- Build workflow: remains manual and uploads GitHub Actions artifacts only.
- Publication workflow: separate manual workflow, accepts a source artifact recipe workflow run ID.
- Backing resource default tracks:
  - `mysql`: `8.4`
  - `postgres`: `18`
  - `redis`: `8.2`
  - `mailpit`: `1`
  - `rustfs`: `1`
- Target backing resource matrix:
  - `redis`: `darwin-arm64`, `darwin-amd64`
  - `mysql`: `darwin-arm64`, `darwin-amd64`
  - `postgres`: `darwin-arm64`, `darwin-amd64`
  - `mailpit`: `darwin-arm64`, `darwin-amd64`
  - `rustfs`: `darwin-arm64`, `darwin-amd64`

## Branch Graph

Do not implement on `main`. Use the existing PR25 planning worktree or create sibling worktrees from `origin/main`.

```text
origin/main
  └─ feat/pr25-publication-foundation
       ├─ feat/pr25-redis-recipe
       ├─ feat/pr25-sql-recipes
       ├─ feat/pr25-mailpit-rustfs-recipes
       └─ feat/pr25-publish-matrix
```

Implementation branches are stacked, but the resource lanes should be independently reviewable after the foundation branch is reviewable. `feat/pr25-publish-matrix` is the convergence branch that merges the resource lanes and proves the full matrix plus R2 publication path.

## File Structure

- Modify `crates/pv-release/src/lib.rs`: export new publication module.
- Modify `crates/pv-release/src/error.rs`: add typed publication errors.
- Modify `crates/pv-release/src/cli.rs`: add publication staging CLI and backing recipe metadata path options.
- Modify `crates/pv-release/src/fixture.rs`: generate backing resource fixtures through shared artifact identity helpers.
- Modify `crates/pv-release/src/recipe.rs`: add generic backing resource recipe metadata support while retaining PHP and Composer validation.
- Create `crates/pv-release/src/publication.rs`: validate downloaded artifacts, re-key flat archives into object-key paths, generate a versioned manifest, and write an upload plan.
- Create `crates/pv-release/tests/publication.rs`: publication staging snapshots and fail-closed coverage.
- Modify `crates/pv-release/tests/recipe_metadata.rs`: committed backing recipe/default-track metadata coverage.
- Modify `crates/pv-release/tests/recipe_fixtures.rs`: full cheap fixture matrix coverage.
- Modify `crates/pv-release/tests/smoke.rs`: backing resource script smoke harness coverage where scripts can be faked locally.
- Modify `release/artifacts/recipes/common.sh`: add shared single-root packaging helpers used by backing resources.
- Modify `release/artifacts/default-tracks.toml`: add accepted backing defaults.
- Create `.github/workflows/artifact-publication.yml`: manual R2 publication workflow.
- Modify `.github/workflows/artifact-recipes.yml`: add backing resources to workflow inputs and build matrix logic.
- Modify `.github/workflows/ci.yml`: extend shellcheck and cheap fixture validation to all recipe dirs.
- Modify `release/artifacts/README.md`: document backing recipes, local checks, and R2 publication workflow.
- Create `release/artifacts/recipes/redis/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`.
- Create `release/artifacts/recipes/mysql/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`.
- Create `release/artifacts/recipes/postgres/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`.
- Create `release/artifacts/recipes/mailpit/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`.
- Create `release/artifacts/recipes/rustfs/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`.

## Task 1: Foundation Publication Staging

**Branch:** `feat/pr25-publication-foundation`

**Files:**
- Modify: `crates/pv-release/src/lib.rs`
- Modify: `crates/pv-release/src/error.rs`
- Modify: `crates/pv-release/src/cli.rs`
- Create: `crates/pv-release/src/publication.rs`
- Create: `crates/pv-release/tests/publication.rs`

- [ ] **Step 1: Write the failing publication staging test**

Create `crates/pv-release/tests/publication.rs` with a fixture that has one flat archive and one release record whose `object_key` is nested under `resources/redis/8.2/...`. The test must call a new `pv_release::publication::prepare_publication` API and snapshot the generated upload plan.

```rust
use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::publication::{prepare_publication, PublicationRequest};

#[test]
fn publication_stage_rekeys_flat_archives_and_writes_upload_plan() -> Result<()> {
    let tempdir = tempdir()?;
    let source_archives = tempdir.path().join("downloaded/archives");
    let candidate_records = tempdir.path().join("downloaded/records");
    let published_records = tempdir.path().join("published/records");
    let published_revocations = tempdir.path().join("published/revocations");
    let defaults = tempdir.path().join("default-tracks.toml");
    let stage = tempdir.path().join("stage");

    create_dir_all(&source_archives)?;
    create_dir_all(&candidate_records)?;
    create_dir_all(&published_records)?;
    create_dir_all(&published_revocations)?;
    write_file(&defaults, REDIS_DEFAULT_TRACK)?;
    write_archive(&source_archives.join("redis-8.2.1-pv1-darwin-arm64.tar.gz"))?;
    write_file(
        &candidate_records.join("redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json"),
        REDIS_RECORD,
    )?;

    prepare_publication(&PublicationRequest {
        source_archives: source_archives.clone(),
        candidate_records,
        published_records,
        published_revocations,
        defaults,
        stage: stage.clone(),
        base_url: "https://artifacts.example.test".to_string(),
        versioned_manifest_key: "manifests/runs/123456789/manifest.json".to_string(),
        stable_manifest_key: "manifest.json".to_string(),
    })?;

    assert!(path_exists(&stage.join("archives/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz")));
    assert!(path_exists(&stage.join("records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json")));
    assert!(path_exists(&stage.join("manifests/runs/123456789/manifest.json")));
    assert!(path_exists(&stage.join("manifest.json")));
    assert_snapshot!(read_to_string(&stage.join("publication-plan.json"))?);

    Ok(())
}

#[test]
fn publication_stage_rejects_missing_archive_before_manifest_write() -> Result<()> {
    let tempdir = tempdir()?;
    let source_archives = tempdir.path().join("downloaded/archives");
    let candidate_records = tempdir.path().join("downloaded/records");
    let published_records = tempdir.path().join("published/records");
    let published_revocations = tempdir.path().join("published/revocations");
    let defaults = tempdir.path().join("default-tracks.toml");
    let stage = tempdir.path().join("stage");

    create_dir_all(&source_archives)?;
    create_dir_all(&candidate_records)?;
    create_dir_all(&published_records)?;
    create_dir_all(&published_revocations)?;
    write_file(&defaults, REDIS_DEFAULT_TRACK)?;
    write_file(
        &candidate_records.join("redis-8.2.1-pv1-darwin-arm64.json"),
        REDIS_RECORD,
    )?;

    let error = prepare_publication(&PublicationRequest {
        source_archives,
        candidate_records,
        published_records,
        published_revocations,
        defaults,
        stage: stage.clone(),
        base_url: "https://artifacts.example.test".to_string(),
        versioned_manifest_key: "manifests/runs/123456789/manifest.json".to_string(),
        stable_manifest_key: "manifest.json".to_string(),
    })
    .expect_err("missing archive should fail publication staging");

    assert!(!path_exists(&stage.join("manifest.json")));
    assert_debug_snapshot!(error);

    Ok(())
}
```

Use helper functions copied from nearby `pv-release` tests for `create_dir_all`, `write_file`, `read_to_string`, and `path_exists`. Use `tar`/`flate2` helper style from `archive_validation.rs` or `fixture.rs` for `write_archive`, with a single root containing `LICENSE`, `NOTICE`, and `bin/redis-server`. `REDIS_RECORD` must use the archive checksum and size produced by the helper; if that makes the test too brittle, compute the checksum in the helper and format the JSON record before writing it.

Run: `cargo nextest run -p pv-release --locked publication_stage_rekeys_flat_archives_and_writes_upload_plan`

Expected: FAIL because `pv_release::publication` does not exist.

- [ ] **Step 2: Add typed publication errors**

Add these variants to `crates/pv-release/src/error.rs`:

```rust
#[error("invalid publication input `{path}`: {reason}")]
InvalidPublicationInput { path: String, reason: String },

#[error("publication would overwrite immutable object `{key}`")]
ImmutablePublicationObjectExists { key: String },
```

Run: `cargo nextest run -p pv-release --locked publication_stage_rekeys_flat_archives_and_writes_upload_plan`

Expected: FAIL because `publication` is still missing.

- [ ] **Step 3: Implement publication staging**

Create `crates/pv-release/src/publication.rs` with this public API:

```rust
use camino::Utf8PathBuf;

#[derive(Clone, Debug)]
pub struct PublicationRequest {
    pub source_archives: Utf8PathBuf,
    pub candidate_records: Utf8PathBuf,
    pub published_records: Utf8PathBuf,
    pub published_revocations: Utf8PathBuf,
    pub defaults: Utf8PathBuf,
    pub stage: Utf8PathBuf,
    pub base_url: String,
    pub versioned_manifest_key: String,
    pub stable_manifest_key: String,
}

pub fn prepare_publication(request: &PublicationRequest) -> crate::Result<()> {
    // Implementation outline:
    // 1. Load candidate records from request.candidate_records.
    // 2. Load existing published records from request.published_records.
    // 3. Fail when any candidate identity duplicates an existing published identity.
    // 4. For every candidate record, find a flat archive named from the last path segment of record.object_key().
    // 5. Validate each archive against its release record.
    // 6. Copy each archive under request.stage/archives joined with record.object_key().
    // 7. Derive the record object key from record.object_key() by replacing `.tar.gz`
    //    with `.json`, prefix it with `records/`, and copy each release record under
    //    request.stage joined with that key.
    // 8. Generate a manifest from staged plus published records and published revocations.
    // 9. Validate generated JSON with resources::ArtifactManifest::parse.
    // 10. Write both request.stage joined with versioned_manifest_key and stable_manifest_key.
    // 11. Write request.stage/publication-plan.json with immutable uploads first and stable manifest last.
    Ok(())
}
```

Use `record::load_release_records`, `archive::validate_archive_for_record_file`, and `manifest::generate_manifest_file_with_defaults` rather than duplicating validation rules. The upload plan JSON shape must be:

```json
{
  "immutable_uploads": [
    {
      "local_path": "archives/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz",
      "object_key": "resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.tar.gz"
    },
    {
      "local_path": "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json",
      "object_key": "records/resources/redis/8.2/8.2.1-pv1/darwin-arm64/redis-8.2.1-pv1-darwin-arm64.json"
    }
  ],
  "versioned_manifest": {
    "local_path": "manifests/runs/123456789/manifest.json",
    "object_key": "manifests/runs/123456789/manifest.json"
  },
  "stable_manifest": {
    "local_path": "manifest.json",
    "object_key": "manifest.json"
  }
}
```

Add `pub mod publication;` to `crates/pv-release/src/lib.rs`.

Run: `cargo nextest run -p pv-release --locked publication_stage_rekeys_flat_archives_and_writes_upload_plan publication_stage_rejects_missing_archive_before_manifest_write`

Expected: PASS after accepting new snapshots.

- [ ] **Step 4: Add CLI parsing for publication staging**

Add a `StagePublication` subcommand to `crates/pv-release/src/cli.rs`:

```rust
StagePublication {
    #[arg(long)]
    source_archives: Utf8PathBuf,
    #[arg(long)]
    candidate_records: Utf8PathBuf,
    #[arg(long)]
    published_records: Utf8PathBuf,
    #[arg(long)]
    published_revocations: Utf8PathBuf,
    #[arg(long)]
    defaults: Utf8PathBuf,
    #[arg(long)]
    stage: Utf8PathBuf,
    #[arg(long)]
    base_url: String,
    #[arg(long)]
    versioned_manifest_key: String,
    #[arg(long)]
    stable_manifest_key: String,
}
```

Dispatch it to `publication::prepare_publication`.

Add a parser test in `crates/pv-release/src/cli.rs` named `parses_stage_publication_arguments`.

Run: `cargo nextest run -p pv-release --locked parses_stage_publication_arguments`

Expected: PASS.

- [ ] **Step 5: Verify and commit foundation publication staging**

Run:

```shell
cargo fmt --all --check
cargo nextest run -p pv-release --locked publication parses_stage_publication_arguments
cargo insta test --accept --test-runner nextest -p pv-release -- publication
```

Commit:

```shell
git add crates/pv-release/src/lib.rs crates/pv-release/src/error.rs crates/pv-release/src/cli.rs crates/pv-release/src/publication.rs crates/pv-release/tests/publication.rs crates/pv-release/tests/snapshots
git commit -m "feat(release): stage artifact publication uploads"
```

## Task 2: Foundation R2 Publication Workflow

**Branch:** `feat/pr25-publication-foundation`

**Files:**
- Create: `.github/workflows/artifact-publication.yml`
- Modify: `release/artifacts/README.md`

- [ ] **Step 1: Add the manual workflow**

Create `.github/workflows/artifact-publication.yml`:

```yaml
name: Artifact Publication

on:
  workflow_dispatch:
    inputs:
      source_run_id:
        description: "Artifact Recipes workflow run ID to publish"
        required: true
        type: string
      stable_manifest_key:
        description: "Stable manifest key"
        required: true
        default: "manifest.json"
        type: string
      versioned_manifest_prefix:
        description: "Versioned manifest key prefix"
        required: true
        default: "manifests/runs"
        type: string

permissions:
  actions: read
  contents: read

jobs:
  publish:
    runs-on: macos-14

    env:
      R2_ENDPOINT: https://${{ secrets.CLOUDFLARE_ACCOUNT_ID }}.r2.cloudflarestorage.com
      R2_BUCKET: ${{ vars.R2_BUCKET }}
      R2_PUBLIC_BASE_URL: ${{ vars.R2_PUBLIC_BASE_URL }}
      AWS_ACCESS_KEY_ID: ${{ secrets.R2_ACCESS_KEY_ID }}
      AWS_SECRET_ACCESS_KEY: ${{ secrets.R2_SECRET_ACCESS_KEY }}
      AWS_DEFAULT_REGION: auto

    steps:
      - name: Checkout
        uses: actions/checkout@v6

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install AWS CLI
        run: |
          set -eu
          if ! command -v aws >/dev/null 2>&1; then
            brew install awscli
          fi
          if ! command -v jq >/dev/null 2>&1; then
            brew install jq
          fi
          aws --version
          jq --version

      - name: Validate publication configuration
        run: |
          set -eu
          for name in R2_ENDPOINT R2_BUCKET R2_PUBLIC_BASE_URL AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY; do
            eval "value=\${$name:-}"
            if [ -z "$value" ]; then
              printf '%s\n' "missing publication configuration: $name" >&2
              exit 1
            fi
          done

      - name: Download source recipe artifacts
        env:
          GH_TOKEN: ${{ github.token }}
          SOURCE_RUN_ID: ${{ inputs.source_run_id }}
        run: |
          set -eu
          rm -rf "$RUNNER_TEMP/pv-publication-download"
          mkdir -p "$RUNNER_TEMP/pv-publication-download"
          gh run download "$SOURCE_RUN_ID" --dir "$RUNNER_TEMP/pv-publication-download"
          find "$RUNNER_TEMP/pv-publication-download" -type f -print | sed -n '1,200p'

      - name: Download existing published records
        run: |
          set -eu
          rm -rf "$RUNNER_TEMP/pv-published"
          mkdir -p "$RUNNER_TEMP/pv-published/records" "$RUNNER_TEMP/pv-published/revocations"
          aws s3 sync "s3://$R2_BUCKET/records" "$RUNNER_TEMP/pv-published/records" \
            --endpoint-url "$R2_ENDPOINT"
          aws s3 sync "s3://$R2_BUCKET/revocations" "$RUNNER_TEMP/pv-published/revocations" \
            --endpoint-url "$R2_ENDPOINT"

      - name: Stage publication
        env:
          SOURCE_RUN_ID: ${{ inputs.source_run_id }}
          STABLE_MANIFEST_KEY: ${{ inputs.stable_manifest_key }}
          VERSIONED_MANIFEST_PREFIX: ${{ inputs.versioned_manifest_prefix }}
        run: |
          set -eu
          source_archives="$RUNNER_TEMP/pv-publication-download"
          candidate_records="$RUNNER_TEMP/pv-publication-download"
          stage="$RUNNER_TEMP/pv-publication-stage"
          versioned_manifest_key="$VERSIONED_MANIFEST_PREFIX/$SOURCE_RUN_ID/manifest.json"

          rm -rf "$stage"
          cargo run -p pv-release -- stage-publication \
            --source-archives "$source_archives" \
            --candidate-records "$candidate_records" \
            --published-records "$RUNNER_TEMP/pv-published/records" \
            --published-revocations "$RUNNER_TEMP/pv-published/revocations" \
            --defaults release/artifacts/default-tracks.toml \
            --stage "$stage" \
            --base-url "$R2_PUBLIC_BASE_URL" \
            --versioned-manifest-key "$versioned_manifest_key" \
            --stable-manifest-key "$STABLE_MANIFEST_KEY"

      - name: Upload immutable artifacts and records
        run: |
          set -eu
          cd "$RUNNER_TEMP/pv-publication-stage"
          jq -r '.immutable_uploads[] | [.local_path, .object_key] | @tsv' publication-plan.json | while IFS="$(printf '\t')" read -r local_path object_key; do
            if aws s3api head-object --bucket "$R2_BUCKET" --key "$object_key" --endpoint-url "$R2_ENDPOINT" >/dev/null 2>&1; then
              printf '%s\n' "immutable object already exists: $object_key" >&2
              exit 1
            fi
            aws s3 cp "$local_path" "s3://$R2_BUCKET/$object_key" --endpoint-url "$R2_ENDPOINT"
          done

      - name: Upload versioned manifest
        run: |
          set -eu
          cd "$RUNNER_TEMP/pv-publication-stage"
          local_path=$(jq -r '.versioned_manifest.local_path' publication-plan.json)
          object_key=$(jq -r '.versioned_manifest.object_key' publication-plan.json)
          aws s3 cp "$local_path" "s3://$R2_BUCKET/$object_key" --endpoint-url "$R2_ENDPOINT"

      - name: Publish stable manifest
        run: |
          set -eu
          cd "$RUNNER_TEMP/pv-publication-stage"
          local_path=$(jq -r '.stable_manifest.local_path' publication-plan.json)
          object_key=$(jq -r '.stable_manifest.object_key' publication-plan.json)
          aws s3 cp "$local_path" "s3://$R2_BUCKET/$object_key" --endpoint-url "$R2_ENDPOINT"
```

The workflow intentionally uses secrets for credentials and repository variables for bucket/public URL:

- `CLOUDFLARE_ACCOUNT_ID`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_BUCKET`
- `R2_PUBLIC_BASE_URL`

- [ ] **Step 2: Add publication docs**

Update `release/artifacts/README.md` with:

```markdown
## Cloudflare R2 Publication

`Artifact Publication` is a manual workflow. It publishes outputs from a prior `Artifact Recipes` workflow run by run ID.

Required configuration:

- Secret `CLOUDFLARE_ACCOUNT_ID`
- Secret `R2_ACCESS_KEY_ID`
- Secret `R2_SECRET_ACCESS_KEY`
- Variable `R2_BUCKET`
- Variable `R2_PUBLIC_BASE_URL`

Publication downloads the selected workflow run artifacts, validates archives and release records again, stages immutable archive and record uploads, writes a versioned manifest under `manifests/runs/$SOURCE_RUN_ID/manifest.json`, and overwrites stable `manifest.json` last. Failed validation stops before the stable manifest update.
```

- [ ] **Step 3: Verify and commit publication workflow**

Run:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh
cargo nextest run -p pv-release --locked publication
```

Commit:

```shell
git add .github/workflows/artifact-publication.yml release/artifacts/README.md
git commit -m "feat(release): publish artifacts to R2"
```

## Task 3: Foundation Shared Backing Recipe Model

**Branch:** `feat/pr25-publication-foundation`

**Files:**
- Modify: `crates/pv-release/src/recipe.rs`
- Modify: `crates/pv-release/src/fixture.rs`
- Modify: `crates/pv-release/src/cli.rs`
- Modify: `crates/pv-release/tests/recipe_metadata.rs`
- Modify: `crates/pv-release/tests/recipe_fixtures.rs`
- Modify: `release/artifacts/recipes/common.sh`

- [ ] **Step 1: Add failing generic backing recipe metadata tests**

In `crates/pv-release/tests/recipe_metadata.rs`, add tests for a generic Redis-style backing recipe:

```rust
use pv_release::recipe::{BackingRecipe, BackingRecipeKind};

#[test]
fn backing_recipe_metadata_parses_common_shape() -> Result<()> {
    let recipe = BackingRecipe::from_toml(
        Utf8Path::new("redis/recipe.toml"),
        BackingRecipeKind::Redis,
        VALID_REDIS_TOML,
    )?;

    assert_debug_snapshot!((
        recipe.resource().as_str(),
        recipe.default_track().as_str(),
        recipe.platforms().iter().map(|platform| platform.as_str()).collect::<Vec<_>>(),
        recipe.tracks().iter().map(|track| (track.name().as_str(), track.upstream_version())).collect::<Vec<_>>(),
        recipe.payload_paths(),
    ));

    Ok(())
}
```

Add a `VALID_REDIS_TOML` constant:

```rust
const VALID_REDIS_TOML: &str = r#"
[recipe]
resources = ["redis"]
default_track = "8.2"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/redis-server", "bin/redis-cli"]

[[tracks]]
name = "8.2"
upstream_version = "8.2.1"
source_url = "https://download.redis.io/releases/redis-8.2.1.tar.gz"
source_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;
```

Run: `cargo nextest run -p pv-release --locked backing_recipe_metadata_parses_common_shape`

Expected: FAIL because `BackingRecipe` and `BackingRecipeKind` do not exist.

- [ ] **Step 2: Add `BackingRecipe` parsing**

In `crates/pv-release/src/recipe.rs`, keep existing PHP and Composer parsing, make `RecipeHeader` accessors reusable, and add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackingRecipeKind {
    Redis,
    Mysql,
    Postgres,
    Mailpit,
    Rustfs,
}

#[derive(Clone, Debug)]
pub struct BackingRecipe {
    path: Utf8PathBuf,
    kind: BackingRecipeKind,
    header: RecipeHeader,
    artifact: BackingArtifact,
    tracks: Vec<BackingTrack>,
}

#[derive(Clone, Debug)]
pub struct BackingArtifact {
    payload_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BackingTrack {
    name: TrackName,
    upstream_version: String,
    source_url: String,
    source_sha256: Sha256Digest,
}
```

Validation rules:

- `resources` must contain exactly the one resource for the kind.
- `platforms` must contain exactly `darwin-arm64` and `darwin-amd64`.
- `default_track` must match a `[[tracks]]` name.
- `payload_paths` must be non-empty and relative.
- `source_url` must be HTTPS.
- `source_sha256` must parse as `Sha256Digest`.
- unknown TOML fields must be rejected through `serde(deny_unknown_fields)`.

Run: `cargo nextest run -p pv-release --locked backing_recipe_metadata_parses_common_shape recipe_metadata_rejects_unknown_fields`

Expected: PASS after accepting snapshots.

- [ ] **Step 3: Extend fixture generation to backing recipes**

Change `generate-recipe-fixtures` so existing `--php` and `--composer` remain supported and add optional path arguments:

```rust
#[arg(long)]
redis: Option<Utf8PathBuf>,
#[arg(long)]
mysql: Option<Utf8PathBuf>,
#[arg(long)]
postgres: Option<Utf8PathBuf>,
#[arg(long)]
mailpit: Option<Utf8PathBuf>,
#[arg(long)]
rustfs: Option<Utf8PathBuf>,
```

In `fixture.rs`, add a shared `write_backing_fixture` that uses each `BackingRecipe` track/platform pair and writes archives containing `LICENSE`, `NOTICE`, and all `payload_paths`.

The generated object key must be the equivalent of this Rust format string:

```text
format!("resources/{resource}/{track}/{artifact_version}/{platform}/{resource}-{artifact_version}-{platform}.tar.gz")
```

The generated record key must be the equivalent of this Rust format string:

```text
format!("resources/{resource}/{track}/{artifact_version}/{platform}/{resource}-{artifact_version}-{platform}.json")
```

Run: `cargo nextest run -p pv-release --locked recipe_fixture_generation_validates_archives_records_and_manifest`

Expected: PASS with old PHP/Composer fixtures unchanged until committed backing recipe paths are supplied.

- [ ] **Step 4: Add shared shell packaging helpers**

In `release/artifacts/recipes/common.sh`, add helpers used by backing scripts:

```sh
artifact_basename() {
  resource=$1
  artifact_version=$2
  platform=$3
  printf '%s\n' "$resource-$artifact_version-$platform"
}

artifact_object_key() {
  resource=$1
  track=$2
  artifact_version=$3
  platform=$4
  basename=$(artifact_basename "$resource" "$artifact_version" "$platform")
  printf '%s\n' "resources/$resource/$track/$artifact_version/$platform/$basename.tar.gz"
}

artifact_record_path() {
  record_dir=$1
  resource=$2
  track=$3
  artifact_version=$4
  platform=$5
  basename=$(artifact_basename "$resource" "$artifact_version" "$platform")
  printf '%s\n' "$record_dir/$resource/$track/$artifact_version/$platform/$basename.json"
}
```

Refactor PHP/Composer only if this can be done without behavior changes. Otherwise leave their current key construction in place and use the helpers only for backing resources.

- [ ] **Step 5: Verify and commit shared backing model**

Run:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh
cargo fmt --all --check
cargo nextest run -p pv-release --locked recipe_metadata recipe_fixture_generation_validates_archives_records_and_manifest parses_generate_recipe_fixtures_arguments
cargo insta test --accept --test-runner nextest -p pv-release -- recipe_metadata recipe_fixture_generation_validates_archives_records_and_manifest
```

Commit:

```shell
git add crates/pv-release/src/recipe.rs crates/pv-release/src/fixture.rs crates/pv-release/src/cli.rs crates/pv-release/tests/recipe_metadata.rs crates/pv-release/tests/recipe_fixtures.rs crates/pv-release/tests/snapshots release/artifacts/recipes/common.sh
git commit -m "feat(release): add shared backing recipe metadata"
```

## Task 4: Redis Recipe Lane

**Branch:** `feat/pr25-redis-recipe`

**Blocked by:** Task 3

**Files:**
- Create: `release/artifacts/recipes/redis/recipe.toml`
- Create: `release/artifacts/recipes/redis/build.sh`
- Create: `release/artifacts/recipes/redis/smoke.sh`
- Create: `release/artifacts/recipes/redis/LICENSE`
- Create: `release/artifacts/recipes/redis/NOTICE`
- Modify: `release/artifacts/default-tracks.toml`
- Modify: `crates/pv-release/tests/recipe_metadata.rs`
- Modify: `crates/pv-release/tests/recipe_fixtures.rs`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Verify Redis upstream source and checksum**

Use the current official Redis 8.2 release source. Record evidence in the Solo scratchpad before editing metadata:

```shell
redis_archive=$(curl -fsSL https://download.redis.io/releases/ | rg -o 'redis-8\.2\.[0-9]+\.tar\.gz' | sort -V | tail -1)
redis_url="https://download.redis.io/releases/$redis_archive"
curl -L --fail --show-error --silent --retry 3 --retry-delay 2 --retry-all-errors "$redis_url" -o /tmp/pv-redis-source.tar.gz
shasum -a 256 /tmp/pv-redis-source.tar.gz
printf '%s\n' "$redis_url"
```

If official Redis 8.2 source cannot be fetched or checksummed, mark the Redis lane blocked with the command output and do not substitute an unofficial source.

- [ ] **Step 2: Add Redis metadata and legal files**

Create `release/artifacts/recipes/redis/recipe.toml` using the verified URL, checksum, and upstream version from Step 1. Generate the file with variables derived from the verified source archive:

```shell
redis_upstream_version=${redis_archive#redis-}
redis_upstream_version=${redis_upstream_version%.tar.gz}
redis_sha256=$(shasum -a 256 /tmp/pv-redis-source.tar.gz | awk '{print $1}')
cat > release/artifacts/recipes/redis/recipe.toml <<EOF
[recipe]
resources = ["redis"]
default_track = "8.2"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/redis-server", "bin/redis-cli"]

[[tracks]]
name = "8.2"
upstream_version = "$redis_upstream_version"
source_url = "$redis_url"
source_sha256 = "$redis_sha256"
EOF
```

Create `LICENSE` and `NOTICE` by copying Redis license/notice text from the verified source archive. Do not use placeholder legal text.

- [ ] **Step 3: Add Redis build and smoke scripts**

Create `release/artifacts/recipes/redis/build.sh`:

```sh
#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
. "$ROOT/release/artifacts/recipes/common.sh"

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
TRACK=${PV_RECIPE_TRACK:-8.2}
PLATFORM=${PV_RECIPE_PLATFORM:-}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-redis}

[ -n "$PLATFORM" ] || die "PV_RECIPE_PLATFORM is required"
if [ -z "$PV_COMMIT" ]; then
  need git
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

need cargo
need curl
need make
need shasum
need tar

recipe_dir="$ROOT/release/artifacts/recipes/redis"
env_file="$OUT_DIR/work/redis.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --redis "$recipe_dir/recipe.toml" \
  --resource redis \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
. "$env_file"

artifact_basename=$(artifact_basename redis "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
work_dir="$OUT_DIR/work/$artifact_basename"
source_archive="$OUT_DIR/sources/redis-$PV_UPSTREAM_VERSION.tar.gz"
source_dir="$work_dir/source"
root_dir="$work_dir/$artifact_basename"
archive="$OUT_DIR/$artifact_basename.tar.gz"
object_key=$(artifact_object_key redis "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
record=$(artifact_record_path "$RECORD_DIR" redis "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")

rm -rf "$work_dir"
mkdir -p "$OUT_DIR/sources" "$source_dir" "$root_dir/bin"
curl -L --fail --show-error --silent --retry 3 --retry-delay 2 --retry-all-errors "$PV_SOURCE_URL" -o "$source_archive"
require_sha256 "$source_archive" "$PV_SOURCE_SHA256"
tar -xzf "$source_archive" -C "$source_dir" --strip-components 1
make -C "$source_dir" BUILD_TLS=yes -j"$(sysctl -n hw.ncpu)"
cp "$source_dir/src/redis-server" "$root_dir/bin/redis-server"
cp "$source_dir/src/redis-cli" "$root_dir/bin/redis-cli"
cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
cp "$recipe_dir/NOTICE" "$root_dir/NOTICE"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"

write_record "$record" redis "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/redis/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION"
cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$recipe_dir/smoke.sh"
printf '%s\n' "$archive"
```

Create `release/artifacts/recipes/redis/smoke.sh` to extract the archive root supplied by `pv-release`, start `redis-server` on a temporary port, run `redis-cli ping`, require `PONG`, then stop the process.

- [ ] **Step 4: Wire Redis into tests and CI**

Add Redis default:

```toml
[[resource]]
name = "redis"
default_track = "8.2"
```

Update `committed_recipe_metadata_parses` to load `redis/recipe.toml` and assert default `8.2`.

Update `recipe_fixture_generation_validates_archives_records_and_manifest` expected archive roots to include Redis roots derived from the committed metadata:

```rust
let redis_upstream_version = redis.tracks()[0].upstream_version().to_string();
ArchiveRoot::new(
    "redis",
    "8.2",
    "darwin-amd64",
    &format!("redis-{redis_upstream_version}-pv1-darwin-amd64"),
);
ArchiveRoot::new(
    "redis",
    "8.2",
    "darwin-arm64",
    &format!("redis-{redis_upstream_version}-pv1-darwin-arm64"),
);
```

Update `.github/workflows/ci.yml` shellcheck and fixture command to include `release/artifacts/recipes/redis/*.sh` and `--redis release/artifacts/recipes/redis/recipe.toml`.

- [ ] **Step 5: Verify and commit Redis lane**

Run:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/redis/*.sh
cargo fmt --all --check
cargo nextest run -p pv-release --locked committed_recipe_metadata_parses recipe_fixture_generation_validates_archives_records_and_manifest
cargo insta test --accept --test-runner nextest -p pv-release -- committed_recipe_metadata_parses recipe_fixture_generation_validates_archives_records_and_manifest
```

If running on macOS with build dependencies available, run:

```shell
PV_RECIPE_PLATFORM=darwin-arm64 release/artifacts/recipes/redis/build.sh
```

Commit:

```shell
git add release/artifacts/recipes/redis release/artifacts/default-tracks.toml crates/pv-release/tests/recipe_metadata.rs crates/pv-release/tests/recipe_fixtures.rs crates/pv-release/tests/snapshots .github/workflows/ci.yml
git commit -m "feat(release): add Redis artifact recipe"
```

## Task 5: SQL Recipe Lane

**Branch:** `feat/pr25-sql-recipes`

**Blocked by:** Task 3

**Files:**
- Create: `release/artifacts/recipes/mysql/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`
- Create: `release/artifacts/recipes/postgres/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`
- Modify: `release/artifacts/default-tracks.toml`
- Modify: `crates/pv-release/tests/recipe_metadata.rs`
- Modify: `crates/pv-release/tests/recipe_fixtures.rs`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Verify upstream SQL sources and platform strategy**

Before editing metadata, record in the Solo scratchpad:

- MySQL 8.4 official source or official macOS binary URL.
- MySQL 8.4 checksum evidence.
- Postgres 18 official source or official macOS binary URL.
- Postgres 18 checksum evidence.
- Whether native macOS official binaries are suitable for `darwin-arm64` and `darwin-amd64`.

If an official binary is not suitable, build from official source in the recipe. If source or checksum evidence cannot be established from official upstream channels, mark the SQL lane blocked with exact links and command output.

- [ ] **Step 2: Add MySQL metadata and recipe files**

Create `release/artifacts/recipes/mysql/recipe.toml` from the official upstream values recorded in Step 1:

```shell
cat > release/artifacts/recipes/mysql/recipe.toml <<EOF
[recipe]
resources = ["mysql"]
default_track = "8.4"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/mysqld", "bin/mysql", "bin/mysqladmin"]

[[tracks]]
name = "8.4"
upstream_version = "$mysql_upstream_version"
source_url = "$mysql_source_url"
source_sha256 = "$mysql_source_sha256"
EOF
```

Create `build.sh` using the same shape as Redis:

- load env with `print-recipe-env --mysql`.
- download and checksum the verified source/binary.
- package `mysqld`, `mysql`, and `mysqladmin`.
- write the release record with recipe path `release/artifacts/recipes/mysql/build.sh`.
- validate with `smoke.sh`.

Create `smoke.sh`:

- initialize a temporary data directory.
- start `mysqld` with socket and port under the temp directory.
- run `mysqladmin ping`.
- run `mysql -e 'SELECT 1'`.
- stop with `mysqladmin shutdown`.
- fail if any step fails.

- [ ] **Step 3: Add Postgres metadata and recipe files**

Create `release/artifacts/recipes/postgres/recipe.toml` from the official upstream values recorded in Step 1:

```shell
cat > release/artifacts/recipes/postgres/recipe.toml <<EOF
[recipe]
resources = ["postgres"]
default_track = "18"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/postgres", "bin/initdb", "bin/pg_ctl", "bin/psql"]

[[tracks]]
name = "18"
upstream_version = "$postgres_upstream_version"
source_url = "$postgres_source_url"
source_sha256 = "$postgres_source_sha256"
EOF
```

Create `build.sh`:

- load env with `print-recipe-env --postgres`.
- download and checksum the verified source/binary.
- package `postgres`, `initdb`, `pg_ctl`, and `psql`.
- write the release record with recipe path `release/artifacts/recipes/postgres/build.sh`.
- validate with `smoke.sh`.

Create `smoke.sh`:

- initialize a temporary data directory with `initdb`.
- start with `pg_ctl`.
- run `psql -c 'SELECT 1'`.
- stop with `pg_ctl stop`.
- fail if any step fails.

- [ ] **Step 4: Wire SQL defaults, tests, and CI**

Add defaults:

```toml
[[resource]]
name = "mysql"
default_track = "8.4"

[[resource]]
name = "postgres"
default_track = "18"
```

Update committed metadata tests, fixture expected roots, shellcheck paths, and `generate-recipe-fixtures` arguments for both SQL recipes.

- [ ] **Step 5: Verify and commit SQL lane**

Run:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh
cargo fmt --all --check
cargo nextest run -p pv-release --locked committed_recipe_metadata_parses recipe_fixture_generation_validates_archives_records_and_manifest
cargo insta test --accept --test-runner nextest -p pv-release -- committed_recipe_metadata_parses recipe_fixture_generation_validates_archives_records_and_manifest
```

If running on macOS with build dependencies available, run both:

```shell
PV_RECIPE_PLATFORM=darwin-arm64 release/artifacts/recipes/mysql/build.sh
PV_RECIPE_PLATFORM=darwin-arm64 release/artifacts/recipes/postgres/build.sh
```

Commit:

```shell
git add release/artifacts/recipes/mysql release/artifacts/recipes/postgres release/artifacts/default-tracks.toml crates/pv-release/tests/recipe_metadata.rs crates/pv-release/tests/recipe_fixtures.rs crates/pv-release/tests/snapshots .github/workflows/ci.yml
git commit -m "feat(release): add SQL artifact recipes"
```

## Task 6: Mailpit and RustFS Recipe Lane

**Branch:** `feat/pr25-mailpit-rustfs-recipes`

**Blocked by:** Task 3

**Files:**
- Create: `release/artifacts/recipes/mailpit/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`
- Create: `release/artifacts/recipes/rustfs/{recipe.toml,build.sh,smoke.sh,LICENSE,NOTICE}`
- Modify: `release/artifacts/default-tracks.toml`
- Modify: `crates/pv-release/tests/recipe_metadata.rs`
- Modify: `crates/pv-release/tests/recipe_fixtures.rs`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Verify upstream static release binaries**

Before editing metadata, record in the Solo scratchpad:

- Mailpit official release URL for both macOS architectures or a single universal binary.
- Mailpit checksum evidence.
- RustFS official release URL for both macOS architectures or a single universal binary.
- RustFS checksum evidence.

If upstream does not publish a checksum, download from the official release URL and record the computed SHA-256 plus the release URL, tag, and Git commit evidence. If an official binary cannot be verified enough for release use, mark the lane blocked.

- [ ] **Step 2: Add Mailpit recipe**

Create `release/artifacts/recipes/mailpit/recipe.toml` from the official upstream values recorded in Step 1:

```shell
cat > release/artifacts/recipes/mailpit/recipe.toml <<EOF
[recipe]
resources = ["mailpit"]
default_track = "1"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/mailpit"]

[[tracks]]
name = "1"
upstream_version = "$mailpit_upstream_version"
source_url = "$mailpit_source_url"
source_sha256 = "$mailpit_source_sha256"
EOF
```

Create `build.sh` that downloads the platform-specific binary archive, verifies checksum, installs `bin/mailpit`, copies legal files, writes a release record, and validates with `smoke.sh`.

Create `smoke.sh` that starts Mailpit with temporary SMTP and HTTP listen addresses, checks HTTP readiness, checks SMTP port binding, and shuts the process down.

- [ ] **Step 3: Add RustFS recipe**

Create `release/artifacts/recipes/rustfs/recipe.toml` from the official upstream values recorded in Step 1:

```shell
cat > release/artifacts/recipes/rustfs/recipe.toml <<EOF
[recipe]
resources = ["rustfs"]
default_track = "1"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/rustfs"]

[[tracks]]
name = "1"
upstream_version = "$rustfs_upstream_version"
source_url = "$rustfs_source_url"
source_sha256 = "$rustfs_source_sha256"
EOF
```

Create `build.sh` that downloads the platform-specific binary archive, verifies checksum, installs `bin/rustfs`, copies legal files, writes a release record, and validates with `smoke.sh`.

Create `smoke.sh` that starts RustFS in a temporary data directory, waits for S3 readiness, creates a test bucket using an available S3-compatible CLI or HTTP request pattern supported by RustFS, lists the bucket, then shuts the process down.

- [ ] **Step 4: Wire Mailpit/RustFS defaults, tests, and CI**

Add defaults:

```toml
[[resource]]
name = "mailpit"
default_track = "1"

[[resource]]
name = "rustfs"
default_track = "1"
```

Update committed metadata tests, fixture expected roots, shellcheck paths, and `generate-recipe-fixtures` arguments for both recipes.

- [ ] **Step 5: Verify and commit Mailpit/RustFS lane**

Run:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
cargo fmt --all --check
cargo nextest run -p pv-release --locked committed_recipe_metadata_parses recipe_fixture_generation_validates_archives_records_and_manifest
cargo insta test --accept --test-runner nextest -p pv-release -- committed_recipe_metadata_parses recipe_fixture_generation_validates_archives_records_and_manifest
```

If running on macOS, run:

```shell
PV_RECIPE_PLATFORM=darwin-arm64 release/artifacts/recipes/mailpit/build.sh
PV_RECIPE_PLATFORM=darwin-arm64 release/artifacts/recipes/rustfs/build.sh
```

Commit:

```shell
git add release/artifacts/recipes/mailpit release/artifacts/recipes/rustfs release/artifacts/default-tracks.toml crates/pv-release/tests/recipe_metadata.rs crates/pv-release/tests/recipe_fixtures.rs crates/pv-release/tests/snapshots .github/workflows/ci.yml
git commit -m "feat(release): add Mailpit and RustFS artifact recipes"
```

## Task 7: Full Recipe Workflow Matrix

**Branch:** `feat/pr25-publish-matrix`

**Blocked by:** Tasks 4, 5, and 6

**Files:**
- Modify: `.github/workflows/artifact-recipes.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `release/artifacts/README.md`

- [ ] **Step 1: Extend Artifact Recipes workflow inputs**

In `.github/workflows/artifact-recipes.yml`, update `resource` choices:

```yaml
options:
  - all
  - php
  - composer
  - redis
  - mysql
  - postgres
  - mailpit
  - rustfs
```

Update the input description to:

```yaml
description: "Resource to build: all, php, composer, redis, mysql, postgres, mailpit, rustfs"
```

Update the `track` description:

```yaml
description: "Track to build: all, 8.2, 8.3, 8.4, 18, 2, 1"
```

- [ ] **Step 2: Validate resource/track combinations**

Replace the workflow validation case with:

```sh
case "$PV_RECIPE_RESOURCE:$PV_RECIPE_TRACK" in
  all:all | all:8.2 | all:8.3 | all:8.4 | all:18 | all:2 | all:1) ;;
  php:all | php:8.2 | php:8.3 | php:8.4) ;;
  composer:all | composer:2) ;;
  redis:all | redis:8.2) ;;
  mysql:all | mysql:8.4) ;;
  postgres:all | postgres:18) ;;
  mailpit:all | mailpit:1) ;;
  rustfs:all | rustfs:1) ;;
  *)
    printf '%s\n' "unsupported resource/track combination: $PV_RECIPE_RESOURCE/$PV_RECIPE_TRACK" >&2
    exit 1
    ;;
esac
```

- [ ] **Step 3: Build selected backing resources**

In the `Build selected artifacts` step, add booleans for each backing resource and call each resource script. For `all`, build every backing resource with its default track on the selected platform. Composer remains `platform=any`.

Use this mapping:

```sh
redis_track_list=8.2
mysql_track_list=8.4
postgres_track_list=18
mailpit_track_list=1
rustfs_track_list=1
```

When a specific backing resource is selected with `track=all`, resolve to that resource default from the mapping above.

- [ ] **Step 4: Generate manifest defaults from selected resources**

In the workflow manifest-generation step, write defaults for every selected resource:

```sh
write_default_track php 8.4
write_default_track frankenphp 8.4
write_default_track composer 2
write_default_track redis 8.2
write_default_track mysql 8.4
write_default_track postgres 18
write_default_track mailpit 1
write_default_track rustfs 1
```

When a single resource is selected, write only that resource's default, except `php`, which writes both `php` and `frankenphp`.

- [ ] **Step 5: Update cheap CI validation**

In `.github/workflows/ci.yml`, update shellcheck:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
```

Update fixture generation to pass every backing metadata path:

```shell
cargo run -p pv-release -- generate-recipe-fixtures \
  --php release/artifacts/recipes/php/tracks.toml \
  --composer release/artifacts/recipes/composer/composer.toml \
  --redis release/artifacts/recipes/redis/recipe.toml \
  --mysql release/artifacts/recipes/mysql/recipe.toml \
  --postgres release/artifacts/recipes/postgres/recipe.toml \
  --mailpit release/artifacts/recipes/mailpit/recipe.toml \
  --rustfs release/artifacts/recipes/rustfs/recipe.toml \
  --archives /tmp/pv-recipe-fixtures/archives \
  --records /tmp/pv-recipe-fixtures/records \
  --pv-commit "$(git rev-parse HEAD)" \
  --build-run-id ci
```

- [ ] **Step 6: Verify and commit workflow matrix**

Run:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
rm -rf /tmp/pv-recipe-fixtures
cargo run -p pv-release -- generate-recipe-fixtures \
  --php release/artifacts/recipes/php/tracks.toml \
  --composer release/artifacts/recipes/composer/composer.toml \
  --redis release/artifacts/recipes/redis/recipe.toml \
  --mysql release/artifacts/recipes/mysql/recipe.toml \
  --postgres release/artifacts/recipes/postgres/recipe.toml \
  --mailpit release/artifacts/recipes/mailpit/recipe.toml \
  --rustfs release/artifacts/recipes/rustfs/recipe.toml \
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
cargo fmt --all --check
cargo nextest run -p pv-release --locked
```

Commit:

```shell
git add .github/workflows/artifact-recipes.yml .github/workflows/ci.yml release/artifacts/README.md
git commit -m "feat(release): build full backing resource matrix"
```

## Task 8: Final Native Build and R2 Publication Verification

**Branch:** `feat/pr25-publish-matrix`

**Blocked by:** Task 7

**Files:**
- Modify only if verification exposes a concrete bug in prior tasks.

- [ ] **Step 1: Run local cheap release checks**

Run:

```shell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
```

If `shellcheck` is unavailable locally, record that gap and rely on CI for the shellcheck lane.

- [ ] **Step 2: Run full fixture manifest check**

Run:

```shell
rm -rf /tmp/pv-recipe-fixtures
cargo run -p pv-release -- generate-recipe-fixtures \
  --php release/artifacts/recipes/php/tracks.toml \
  --composer release/artifacts/recipes/composer/composer.toml \
  --redis release/artifacts/recipes/redis/recipe.toml \
  --mysql release/artifacts/recipes/mysql/recipe.toml \
  --postgres release/artifacts/recipes/postgres/recipe.toml \
  --mailpit release/artifacts/recipes/mailpit/recipe.toml \
  --rustfs release/artifacts/recipes/rustfs/recipe.toml \
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

Inspect the manifest and confirm it includes `php`, `frankenphp`, `composer`, `redis`, `mysql`, `postgres`, `mailpit`, and `rustfs`.

- [ ] **Step 3: Dispatch native Artifact Recipes workflow**

From the GitHub UI or `gh`, run `Artifact Recipes` for:

```text
resource=all
track=all
platform=darwin-arm64
```

Then run:

```text
resource=all
track=all
platform=darwin-amd64
```

If either platform fails for a resource, record the failed lane, workflow URL, and log evidence in the Solo scratchpad. Do not remove the failed resource from the target matrix silently.

- [ ] **Step 4: Dispatch Artifact Publication workflow against R2**

After a successful recipe workflow run, dispatch `Artifact Publication` with:

```text
source_run_id=123456789
stable_manifest_key=manifest.json
versioned_manifest_prefix=manifests/runs
```

Verify with AWS CLI:

```shell
aws s3 cp "s3://$R2_BUCKET/manifest.json" /tmp/pv-published-manifest.json --endpoint-url "https://$CLOUDFLARE_ACCOUNT_ID.r2.cloudflarestorage.com"
cargo run -p pv-release -- generate-manifest \
  --records /tmp/pv-recipe-fixtures/records \
  --revocations release/artifacts/revocations \
  --defaults release/artifacts/default-tracks.toml \
  --output /tmp/pv-local-compare-manifest.json \
  --base-url "$R2_PUBLIC_BASE_URL"
```

Confirm the stable manifest parses through `resources::ArtifactManifest::parse` and includes the published backing resources. If immutable object upload fails because a key exists, treat that as a failed publication attempt and investigate identity/versioning before retrying.

- [ ] **Step 5: Commit final verification evidence if docs changed**

If only workflow logs changed, do not create an empty commit. If docs were updated with final commands or configuration clarifications:

```shell
git add release/artifacts/README.md
git commit -m "docs(release): document artifact publication verification"
```

## Task 9: Review-Ready Handoff

**Branch:** all PR25 branches

**Blocked by:** Task 8

**Files:**
- Modify: `IMPLEMENTATION.md`
- Optional modify: `release/artifacts/README.md`

- [ ] **Step 1: Mark roadmap row complete only after verification**

After all PR25 branches have landed or the final stacked PR is ready, update the PR25 row in `IMPLEMENTATION.md` from Done `No` to Done with the PR number or branch reference used by the project convention.

- [ ] **Step 2: Run final checks**

Run:

```shell
git status --short --branch
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
```

Run `shellcheck` if available:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
```

- [ ] **Step 3: Commit roadmap update**

Commit:

```shell
git add IMPLEMENTATION.md release/artifacts/README.md
git commit -m "docs: mark PR 25 artifact publication complete"
```

## Solo Coordination Map

Create Solo tasks from this plan with these blocker relationships:

- `PR25 foundation: publication staging` has no blockers.
- `PR25 foundation: R2 workflow` blocks on `PR25 foundation: publication staging`.
- `PR25 foundation: shared backing recipe model` blocks on `PR25 foundation: publication staging`.
- `PR25 Redis recipe lane` blocks on `PR25 foundation: shared backing recipe model`.
- `PR25 SQL recipe lane` blocks on `PR25 foundation: shared backing recipe model`.
- `PR25 Mailpit/RustFS recipe lane` blocks on `PR25 foundation: shared backing recipe model`.
- `PR25 full workflow matrix` blocks on all three resource lanes.
- `PR25 native build and R2 verification` blocks on `PR25 full workflow matrix`.
- `PR25 review-ready handoff` blocks on `PR25 native build and R2 verification`.

Each resource lane must write upstream version, URL, checksum evidence, native build status, and smoke status into the PR25 scratchpad before it is marked complete.

## Self-Review

- Spec coverage: publication has a separate manual workflow; R2 secrets and variables are named; stable direct `manifest.json` is updated last; resource recipes are separate directories with shared metadata/fixture helpers; full backing matrix is included; failed native lanes are recorded instead of silently removed.
- Placeholder scan: recipe metadata snippets are generated from variables that must be populated by upstream verification steps before the file is written. The final committed tree must contain exact upstream versions, source URLs, and SHA-256 values.
- Type consistency: `PublicationRequest`, `prepare_publication`, `BackingRecipe`, and `BackingRecipeKind` are introduced before later tasks refer to them.
- Test coverage: focused `pv-release` tests, snapshots, shellcheck, cheap fixture manifest generation, and manual native workflow verification are specified.
