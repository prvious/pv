# PR 24 Paired StaticPHP Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Change PR 24's PHP-family artifact recipe so one StaticPHP v3 build per PHP track/platform produces both PV `php` and `frankenphp` artifacts.

**Architecture:** Keep PHP/FrankenPHP recipe metadata in `release/artifacts/recipes/php/tracks.toml`, but make `release/artifacts/recipes/php/build.sh` pair-oriented instead of resource-oriented. The script will load both PHP and FrankenPHP env blocks, download verified PHP and FrankenPHP sources, run one combined StaticPHP v3 build, then package and validate two normalized archives and two release records from the same buildroot. The manual workflow treats `resource=php` as the PHP-family pair and keeps Composer independent.

**Tech Stack:** POSIX shell, `pv-release` recipe env/record/archive helpers, StaticPHP v3 `spc build:php`, native macOS GitHub Actions runners, `cargo nextest`, `insta`, `shellcheck`.

---

## File Structure

- Modify `release/artifacts/recipes/php/build.sh`: remove independent `PV_RECIPE_RESOURCE=php|frankenphp` build branching and package both artifacts from one buildroot.
- Modify `crates/pv-release/tests/smoke.rs`: update the fake build harness to prove one StaticPHP invocation creates both `php` and `frankenphp` artifacts and records.
- Modify `crates/pv-release/tests/snapshots/smoke__*.snap`: accept intentional snapshot changes from the paired build behavior.
- Modify `.github/workflows/artifact-recipes.yml`: change workflow resource selection to `all`, `php`, and `composer`; download StaticPHP v3 from the v3 path; build PHP-family pairs once per selected PHP track.
- Modify `release/artifacts/README.md`: document that PHP recipe builds are paired and Composer remains independent.
- Modify `docs/superpowers/plans/2026-06-07-pr-24-php-frankenphp-composer-artifact-recipes.md`: append a short amendment pointing to this follow-up plan so future reviewers do not follow stale independent-build steps.

## Task 1: Add Failing Pair-Build Harness Coverage

**Files:**
- Modify: `crates/pv-release/tests/smoke.rs`
- Snapshot updates expected under: `crates/pv-release/tests/snapshots/`

- [ ] **Step 1: Replace independent command-shape tests with a pair-build test**

In `crates/pv-release/tests/smoke.rs`, replace `php_build_smoke_uses_combined_staticphp_command_and_verified_source_for_cli` and `frankenphp_build_smoke_uses_combined_staticphp_command_and_verified_source` with this single test:

```rust
#[test]
fn php_pair_build_smoke_builds_cli_and_frankenphp_from_one_staticphp_buildroot() -> Result<()> {
    let run = run_php_build_recipe_smoke()?;
    let php_source_dir = format!("{}/sources/php-8.4.20-source/php-source", run.out_dir);
    let frankenphp_source_dir = format!(
        "{}/sources/frankenphp-8.4.20-frankenphp1.12.3-pv1-source/frankenphp-source",
        run.out_dir
    );
    let expected_log = format!(
        "pwd={}/work/php-pair-8.4-darwin-arm64/staticphp\n\
argv=[build:php][json][--build-cli][--build-frankenphp][--enable-zts][--dl-with-php=8.4.20][--dl-custom-local][php-src:{php_source_dir}][--dl-custom-local][frankenphp:{frankenphp_source_dir}]\n",
        run.out_dir
    );

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert_eq!(run.spc_log, expected_log);
    assert!(run.php_record_json.is_some(), "PHP record was not written");
    assert!(
        run.frankenphp_record_json.is_some(),
        "FrankenPHP record was not written"
    );
    assert_debug_snapshot!(build_recipe_record_provenance(
        run.php_record_json.as_deref()
    )?);
    assert_debug_snapshot!(build_recipe_record_provenance(
        run.frankenphp_record_json.as_deref()
    )?);
    assert_debug_snapshot!(build_recipe_notice_source_lines(
        run.frankenphp_notice.as_deref()
    )?);

    Ok(())
}
```

- [ ] **Step 2: Update `BuildRecipeRun` to hold both artifacts**

Replace the current `BuildRecipeRun` struct with:

```rust
struct BuildRecipeRun {
    out_dir: String,
    output: Output,
    php_record_json: Option<String>,
    frankenphp_record_json: Option<String>,
    php_notice: Option<String>,
    frankenphp_notice: Option<String>,
    spc_log: String,
}
```

- [ ] **Step 3: Remove resource selection from the smoke helper options**

Replace `BuildRecipeOptions` with:

```rust
struct BuildRecipeOptions<'a> {
    lipo_archs: &'a str,
    macho_minos: &'a str,
    macho_libraries: &'a str,
    macho_rpaths: &'a str,
}
```

Replace `run_php_build_recipe_smoke(resource: &str)` with:

```rust
fn run_php_build_recipe_smoke() -> Result<BuildRecipeRun> {
    run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        lipo_archs: "arm64",
        macho_minos: "13.0",
        macho_libraries: "",
        macho_rpaths: "",
    })
}
```

- [ ] **Step 4: Update failure tests to call the pair build once**

For every test that currently constructs `BuildRecipeOptions` with a `resource` field, remove the `resource` field and leave the existing `lipo_archs`, `macho_minos`, `macho_libraries`, and `macho_rpaths` fields. Rename the FrankenPHP rpath tests to make clear they validate the paired build output, for example:

```rust
#[test]
fn php_pair_build_smoke_rejects_homebrew_rpath_on_frankenphp_binary() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        lipo_archs: "arm64",
        macho_minos: "13.0",
        macho_libraries: "\t@rpath/libphp.dylib (compatibility version 1.0.0, current version 1.0.0)",
        macho_rpaths: "/usr/local/opt/openssl@3/lib",
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}
```

- [ ] **Step 5: Update `run_php_build_recipe_smoke_with_options` paths**

Inside `run_php_build_recipe_smoke_with_options`, remove `.env("PV_RECIPE_RESOURCE", options.resource)` and set only:

```rust
.env("PV_RECIPE_TRACK", "8.4")
```

After command execution, read both expected records and notices:

```rust
let php_artifact_version = "8.4.20-pv1";
let php_artifact_basename = "php-8.4.20-pv1-darwin-arm64";
let php_record = record_dir
    .join("php")
    .join("8.4")
    .join(php_artifact_version)
    .join("darwin-arm64")
    .join(format!("{php_artifact_basename}.json"));
let php_notice = out_dir
    .join("work")
    .join("php-pair-8.4-darwin-arm64")
    .join(php_artifact_basename)
    .join("NOTICE");

let frankenphp_artifact_version = "8.4.20-frankenphp1.12.3-pv1";
let frankenphp_artifact_basename = "frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64";
let frankenphp_record = record_dir
    .join("frankenphp")
    .join("8.4")
    .join(frankenphp_artifact_version)
    .join("darwin-arm64")
    .join(format!("{frankenphp_artifact_basename}.json"));
let frankenphp_notice = out_dir
    .join("work")
    .join("php-pair-8.4-darwin-arm64")
    .join(frankenphp_artifact_basename)
    .join("NOTICE");

let (php_record_json, frankenphp_record_json, php_notice, frankenphp_notice) =
    if output.status.success() {
        (
            Some(read_file(&php_record)?),
            Some(read_file(&frankenphp_record)?),
            Some(read_file(&php_notice)?),
            Some(read_file(&frankenphp_notice)?),
        )
    } else {
        (None, None, None, None)
    };
```

Return the updated struct:

```rust
Ok(BuildRecipeRun {
    out_dir: out_dir.to_string(),
    output,
    php_record_json,
    frankenphp_record_json,
    php_notice,
    frankenphp_notice,
    spc_log: read_file(&spc_log)?,
})
```

- [ ] **Step 6: Update record helper signatures**

Replace `build_recipe_record_provenance` with:

```rust
fn build_recipe_record_provenance(record_json: Option<&str>) -> Result<Value> {
    let record_json =
        record_json.ok_or_else(|| anyhow::anyhow!("build recipe did not produce a record"))?;
    let record: Value = serde_json::from_str(record_json)?;
    record
        .get("provenance")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("build recipe record did not contain provenance"))
}
```

Replace `build_recipe_notice_source_lines` with:

```rust
fn build_recipe_notice_source_lines(notice: Option<&str>) -> Result<Vec<&str>> {
    let notice = notice.ok_or_else(|| anyhow::anyhow!("build recipe did not produce NOTICE"))?;
    Ok(notice
        .lines()
        .filter(|line| line.contains("source"))
        .collect())
}
```

- [ ] **Step 7: Update fake cargo to emit both env blocks by requested resource**

Keep `write_fake_cargo` resource-sensitive, because the production script will call `print-recipe-env` once for `php` and once for `frankenphp`. Ensure the fake does not depend on `PV_RECIPE_RESOURCE`; parse the `--resource` value instead:

```sh
resource=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --resource)
      shift
      resource=$1
      ;;
  esac
  shift || true
done
case "$resource" in
  php)
    upstream_version=8.4.20
    artifact_version=8.4.20-pv1
    source_url=https://sources.example.test/php.tar.gz
    source_sha256=$PV_TEST_PHP_SOURCE_SHA256
    php_source_env=
    ;;
  frankenphp)
    upstream_version=8.4.20-frankenphp1.12.3
    artifact_version=8.4.20-frankenphp1.12.3-pv1
    source_url=https://sources.example.test/frankenphp.tar.gz
    source_sha256=$PV_TEST_SOURCE_SHA256
    php_source_env="PV_PHP_SOURCE_URL=https://sources.example.test/php.tar.gz
PV_PHP_SOURCE_SHA256=$PV_TEST_PHP_SOURCE_SHA256"
    ;;
  *) exit 77 ;;
esac
```

- [ ] **Step 8: Run focused test and verify failure**

Run:

```bash
cargo nextest run -p pv-release -E 'test(php_pair_build_smoke)'
```

Expected: FAIL because `release/artifacts/recipes/php/build.sh` still builds one resource per invocation and does not write both records.

## Task 2: Refactor PHP Build Recipe To Produce A Pair

**Files:**
- Modify: `release/artifacts/recipes/php/build.sh`

- [ ] **Step 1: Replace resource selection with pair semantics**

Remove:

```sh
RESOURCE=${PV_RECIPE_RESOURCE:-php}
```

Remove the `case "$RESOURCE"` validation block. Keep:

```sh
TRACK=${PV_RECIPE_TRACK:-8.4}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
```

- [ ] **Step 2: Add helper functions for env loading**

After `validate_macho_binary()`, add:

```sh
print_php_env() {
  resource=$1
  env_file=$2
  cargo run -p pv-release -- print-recipe-env \
    --php "$recipe_dir/tracks.toml" \
    --resource "$resource" \
    --track "$TRACK" \
    --platform "$PLATFORM" >"$env_file"
}
```

- [ ] **Step 3: Replace one-resource env loading with two env loads**

Replace the current block that starts with `env_file="$OUT_DIR/work/$RESOURCE-$TRACK-$PLATFORM.env"` and ends after `. "$env_file"` with:

```sh
pair_name="php-pair-$TRACK-$PLATFORM"
work_dir="$OUT_DIR/work/$pair_name"
spc_work_dir="$work_dir/staticphp"
php_env_file="$work_dir/php.env"
frankenphp_env_file="$work_dir/frankenphp.env"

rm -rf "$work_dir"
mkdir -p "$spc_work_dir" "$OUT_DIR/sources"

print_php_env php "$php_env_file"
# shellcheck source=/dev/null
. "$php_env_file"
PHP_UPSTREAM_VERSION=$PV_UPSTREAM_VERSION
PHP_ARTIFACT_VERSION=$PV_ARTIFACT_VERSION
PHP_SOURCE_URL=$PV_SOURCE_URL
PHP_SOURCE_SHA256=$PV_SOURCE_SHA256
PHP_PHP_VERSION=$PV_PHP_VERSION
PHP_BUILD_EXTENSIONS=$PV_BUILD_EXTENSIONS
PHP_EXPECTED_EXTENSIONS=$PV_EXPECTED_EXTENSIONS
PHP_DEPLOYMENT_TARGET=$PV_DEPLOYMENT_TARGET
PHP_MINIMUM_PV_VERSION=$PV_MINIMUM_PV_VERSION
PHP_PV_BUILD_REVISION=$PV_PV_BUILD_REVISION

print_php_env frankenphp "$frankenphp_env_file"
# shellcheck source=/dev/null
. "$frankenphp_env_file"
FRANKENPHP_UPSTREAM_VERSION=$PV_UPSTREAM_VERSION
FRANKENPHP_ARTIFACT_VERSION=$PV_ARTIFACT_VERSION
FRANKENPHP_SOURCE_URL=$PV_SOURCE_URL
FRANKENPHP_SOURCE_SHA256=$PV_SOURCE_SHA256
FRANKENPHP_MINIMUM_PV_VERSION=$PV_MINIMUM_PV_VERSION
FRANKENPHP_PV_BUILD_REVISION=$PV_PV_BUILD_REVISION

[ "$PV_PHP_VERSION" = "$PHP_PHP_VERSION" ] || die "PHP pair metadata mismatch: php env has $PHP_PHP_VERSION but frankenphp env has $PV_PHP_VERSION"
[ "$PV_BUILD_EXTENSIONS" = "$PHP_BUILD_EXTENSIONS" ] || die "PHP pair metadata mismatch: extension build sets differ"
[ "$PV_EXPECTED_EXTENSIONS" = "$PHP_EXPECTED_EXTENSIONS" ] || die "PHP pair metadata mismatch: expected extension sets differ"
[ "$PV_DEPLOYMENT_TARGET" = "$PHP_DEPLOYMENT_TARGET" ] || die "PHP pair metadata mismatch: deployment targets differ"
```

- [ ] **Step 4: Build both binaries with one StaticPHP invocation**

Replace the current `case "$RESOURCE"` build block with:

```sh
export MACOSX_DEPLOYMENT_TARGET="$PHP_DEPLOYMENT_TARGET"

php_source_dir=$(download_source php "$PHP_PHP_VERSION" "$PHP_SOURCE_URL" "$PHP_SOURCE_SHA256")
frankenphp_source_dir=$(download_source frankenphp "$FRANKENPHP_ARTIFACT_VERSION" "$FRANKENPHP_SOURCE_URL" "$FRANKENPHP_SOURCE_SHA256")

(
  cd "$spc_work_dir"
  spc build:php "$PHP_BUILD_EXTENSIONS" \
    --build-cli \
    --build-frankenphp \
    --enable-zts \
    --dl-with-php="$PHP_PHP_VERSION" \
    --dl-custom-local "php-src:$php_source_dir" \
    --dl-custom-local "frankenphp:$frankenphp_source_dir"
)

[ -f "$spc_work_dir/buildroot/bin/php" ] || die "StaticPHP pair build did not produce buildroot/bin/php"
[ -f "$spc_work_dir/buildroot/bin/frankenphp" ] || die "StaticPHP pair build did not produce buildroot/bin/frankenphp"
```

- [ ] **Step 5: Add a packaging helper**

Before the packaging section, add:

```sh
package_artifact() {
  resource=$1
  upstream_version=$2
  artifact_version=$3
  source_url=$4
  source_sha256=$5
  minimum_pv_version=$6
  pv_build_revision=$7
  binary_name=$8
  source_inputs_json=${9:-}

  artifact_basename="$resource-$artifact_version-$PLATFORM"
  root_dir="$work_dir/$artifact_basename"
  archive="$OUT_DIR/$artifact_basename.tar.gz"
  record="$RECORD_DIR/$resource/$TRACK/$artifact_version/$PLATFORM/$artifact_basename.json"
  object_key="resources/$resource/$TRACK/$artifact_version/$PLATFORM/$artifact_basename.tar.gz"

  mkdir -p "$root_dir/bin"
  cp "$spc_work_dir/buildroot/bin/$binary_name" "$root_dir/bin/$binary_name"
  [ -f "$root_dir/bin/$binary_name" ] || die "$resource artifact did not produce bin/$binary_name"
  validate_macho_binary "$root_dir/bin/$binary_name"

  cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
  {
    cat "$recipe_dir/NOTICE"
    printf '\nArtifact build metadata:\n'
    printf 'Resource: %s\n' "$resource"
    printf 'Track: %s\n' "$TRACK"
    printf 'Artifact version: %s\n' "$artifact_version"
    printf 'Upstream version: %s\n' "$upstream_version"
    printf 'Source URL: %s\n' "$source_url"
    printf 'Source SHA-256: %s\n' "$source_sha256"
    if [ "$resource" = "frankenphp" ]; then
      printf 'PHP source URL: %s\n' "$PHP_SOURCE_URL"
      printf 'PHP source SHA-256: %s\n' "$PHP_SOURCE_SHA256"
    fi
  } >"$root_dir/NOTICE"

  COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"
  write_record "$record" "$resource" "$TRACK" "$upstream_version" "$pv_build_revision" "$PLATFORM" "$object_key" "$archive" "$source_url" "$source_sha256" release/artifacts/recipes/php/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$minimum_pv_version" "$source_inputs_json"

  cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$ROOT/release/artifacts/recipes/php/smoke.sh"
  printf '%s\n' "$archive"
}
```

- [ ] **Step 6: Package PHP and FrankenPHP from the shared buildroot**

Replace the existing final packaging block with:

```sh
package_artifact php \
  "$PHP_UPSTREAM_VERSION" \
  "$PHP_ARTIFACT_VERSION" \
  "$PHP_SOURCE_URL" \
  "$PHP_SOURCE_SHA256" \
  "$PHP_MINIMUM_PV_VERSION" \
  "$PHP_PV_BUILD_REVISION" \
  php

frankenphp_source_inputs_json=$(cat <<JSON
[
      {
        "name": "frankenphp",
        "source_url": "$FRANKENPHP_SOURCE_URL",
        "source_sha256": "$FRANKENPHP_SOURCE_SHA256"
      },
      {
        "name": "php",
        "source_url": "$PHP_SOURCE_URL",
        "source_sha256": "$PHP_SOURCE_SHA256"
      }
    ]
JSON
)

package_artifact frankenphp \
  "$FRANKENPHP_UPSTREAM_VERSION" \
  "$FRANKENPHP_ARTIFACT_VERSION" \
  "$FRANKENPHP_SOURCE_URL" \
  "$FRANKENPHP_SOURCE_SHA256" \
  "$FRANKENPHP_MINIMUM_PV_VERSION" \
  "$FRANKENPHP_PV_BUILD_REVISION" \
  frankenphp \
  "$frankenphp_source_inputs_json"
```

- [ ] **Step 7: Run focused pair-build test and accept snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -p pv-release --test smoke php_pair_build_smoke
```

Expected: PASS and updated smoke snapshots only for the changed pair-build output.

- [ ] **Step 8: Run the full smoke test file**

Run:

```bash
cargo nextest run -p pv-release --test smoke --locked
```

Expected: PASS.

- [ ] **Step 9: Commit the build script and smoke harness**

Run:

```bash
git add release/artifacts/recipes/php/build.sh crates/pv-release/tests/smoke.rs crates/pv-release/tests/snapshots
git commit -m "fix(release): build php artifacts as a StaticPHP pair"
```

## Task 3: Update Manual Workflow For PHP-Family Pairs And StaticPHP v3

**Files:**
- Modify: `.github/workflows/artifact-recipes.yml`

- [ ] **Step 1: Update workflow input choices**

Change the resource description and options to:

```yaml
resource:
  description: "Resource to build: all, php, composer"
  required: true
  default: "all"
  type: choice
  options:
    - all
    - php
    - composer
```

Keep `track` as `all, 8.2, 8.3, 8.4, 2`.

- [ ] **Step 2: Update input validation**

Replace the resource/track case block with:

```sh
case "$PV_RECIPE_RESOURCE:$PV_RECIPE_TRACK" in
  all:all | all:8.2 | all:8.3 | all:8.4) ;;
  php:all | php:8.2 | php:8.3 | php:8.4) ;;
  composer:all | composer:2) ;;
  *)
    printf '%s\n' "unsupported resource/track combination: $PV_RECIPE_RESOURCE/$PV_RECIPE_TRACK" >&2
    exit 1
    ;;
esac
```

- [ ] **Step 3: Download StaticPHP v3 with bounded curl**

Replace the `spc` download line with:

```sh
curl -L --fail --show-error --silent \
  --retry 3 --retry-delay 2 --retry-all-errors \
  --connect-timeout 20 --max-time 600 \
  "https://dl.static-php.dev/v3/spc-bin/nightly/spc-macos-$static_php_architecture" \
  -o "$RUNNER_TEMP/pv-tools/spc"
```

Immediately after `chmod +x`, add:

```sh
"$RUNNER_TEMP/pv-tools/spc" --version
"$RUNNER_TEMP/pv-tools/spc" build:php --help | grep -- '--build-frankenphp' >/dev/null
"$RUNNER_TEMP/pv-tools/spc" build:php --help | grep -- '--dl-custom-local' >/dev/null
```

- [ ] **Step 4: Update the build loop to call PHP recipe once per track**

Replace the `resource_list` and nested resource loop with:

```sh
build_php_family=false
build_composer=false
case "$PV_RECIPE_RESOURCE" in
  all)
    build_php_family=true
    build_composer=true
    ;;
  php)
    build_php_family=true
    ;;
  composer)
    build_composer=true
    ;;
esac

case "$PV_RECIPE_TRACK" in
  all)
    php_track_list='8.2
8.3
8.4'
    ;;
  *)
    php_track_list=$PV_RECIPE_TRACK
    ;;
esac

if [ "$build_php_family" = true ]; then
  while IFS= read -r track; do
    [ -n "$track" ] || continue
    PV_RECIPE_TRACK="$track" \
      release/artifacts/recipes/php/build.sh
  done <<PHP_TRACKS
$php_track_list
PHP_TRACKS
fi

if [ "$build_composer" = true ]; then
  PV_RECIPE_TRACK=2 \
    PV_RECIPE_PLATFORM=any \
    PV_COMPOSER_SMOKE_PHP="$(command -v php)" \
    release/artifacts/recipes/composer/build.sh
fi
```

- [ ] **Step 5: Update default-track manifest generation**

Replace the resource case block with:

```sh
case "$PV_RECIPE_RESOURCE" in
  all)
    write_default_track php "$php_default_track"
    write_default_track frankenphp "$php_default_track"
    write_default_track composer 2
    ;;
  php)
    write_default_track php "$php_default_track"
    write_default_track frankenphp "$php_default_track"
    ;;
  composer)
    write_default_track composer 2
    ;;
esac
```

- [ ] **Step 6: Run YAML diff and shell syntax checks**

Run:

```bash
git diff -- .github/workflows/artifact-recipes.yml
```

Expected: no `frankenphp` workflow resource option remains, and the StaticPHP URL includes `/v3/`.

Run:

```bash
git diff --check
```

Expected: PASS.

- [ ] **Step 7: Commit the workflow update**

Run:

```bash
git add .github/workflows/artifact-recipes.yml
git commit -m "fix(ci): build php recipe pairs with StaticPHP v3"
```

## Task 4: Update Recipe Documentation

**Files:**
- Modify: `release/artifacts/README.md`
- Modify: `docs/superpowers/plans/2026-06-07-pr-24-php-frankenphp-composer-artifact-recipes.md`

- [ ] **Step 1: Update README recipe wording**

In `release/artifacts/README.md`, replace the PHP recipe paragraph with:

```markdown
`recipes/php/tracks.toml` is the data source for paired PHP and FrankenPHP artifact builds. Each selected PHP track/platform is built once with StaticPHP v3, producing both the standalone `php` binary and the matched `frankenphp` binary from the same buildroot. The recipe pins PHP tracks, source URLs, checksums, the expected extension set, the macOS deployment target, and the FrankenPHP source version used by the pair.
```

Add this sentence after the local validation block:

```markdown
The manual `Artifact Recipes` workflow treats `resource=php` as a PHP-family build: each selected PHP track/platform produces both `php` and `frankenphp` artifacts. Composer remains independently selectable as `resource=composer`.
```

- [ ] **Step 2: Add an amendment to the original plan**

At the top of `docs/superpowers/plans/2026-06-07-pr-24-php-frankenphp-composer-artifact-recipes.md`, after the header block and before `## File Structure`, add:

```markdown
## Paired StaticPHP Build Amendment

The original Task 6 and Task 9 steps described independently selected `php` and `frankenphp` native builds. That is superseded by `docs/superpowers/plans/2026-06-07-pr-24-paired-staticphp-build.md`.

PHP-family native builds are now pair-first: one StaticPHP v3 buildroot per PHP track/platform produces both `php` and `frankenphp` archives and release records. Composer remains a separate portable artifact path.
```

- [ ] **Step 3: Verify docs diff**

Run:

```bash
git diff -- release/artifacts/README.md docs/superpowers/plans/2026-06-07-pr-24-php-frankenphp-composer-artifact-recipes.md
```

Expected: docs mention pair-first PHP-family builds and no longer imply independent native PHP/FrankenPHP workflow selection.

- [ ] **Step 4: Commit docs**

Run:

```bash
git add release/artifacts/README.md docs/superpowers/plans/2026-06-07-pr-24-php-frankenphp-composer-artifact-recipes.md
git commit -m "docs: describe paired php artifact recipes"
```

## Task 5: Final Focused Verification

**Files:**
- No new files.
- Verify changes from Tasks 1-4.

- [ ] **Step 1: Run focused release tests**

Run:

```bash
cargo nextest run -p pv-release --test smoke --locked
```

Expected: PASS.

- [ ] **Step 2: Run recipe metadata tests**

Run:

```bash
cargo nextest run -p pv-release --test recipe_metadata --locked
```

Expected: PASS.

- [ ] **Step 3: Run cheap fixture validation tests**

Run:

```bash
cargo nextest run -p pv-release --test recipe_fixtures --locked
```

Expected: PASS.

- [ ] **Step 4: Run shell syntax checks**

Run:

```bash
sh -n release/artifacts/recipes/php/build.sh
sh -n release/artifacts/recipes/php/smoke.sh
sh -n release/artifacts/recipes/composer/build.sh
sh -n release/artifacts/recipes/composer/smoke.sh
```

Expected: all commands exit 0.

- [ ] **Step 5: Run formatting and clippy for touched Rust tests**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p pv-release --all-targets --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Run diff hygiene**

Run:

```bash
git diff --check HEAD~4..HEAD
```

Expected: PASS.

- [ ] **Step 7: Report manual workflow validation requirement**

Do not mark real artifact publication ready until the GitHub manual workflow has been run for at least:

```text
resource=php, track=8.4, platform=darwin-arm64
resource=composer, track=2, platform=darwin-arm64
```

Expected: both workflow runs upload archives, records, and a generated manifest artifact. The PHP-family run must upload a PHP archive named like `php-8.4.20-pv1-darwin-arm64.tar.gz` and a FrankenPHP archive named like `frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64.tar.gz` for the selected track/platform.

## Self-Review Notes

- Spec coverage: This plan implements the approved paired StaticPHP v3 build model, workflow pair semantics, StaticPHP v3 tool check, shared buildroot packaging, and paired verification.
- Scope: This plan intentionally does not cover unrelated review findings such as default-track strict deserialization or Composer smoke temp-file hardening. Those should be handled in a separate narrow fix plan or as a follow-up review-fix task.
- Test strategy: The first implementation task changes the fake recipe harness before production shell code, so the pair-build contract is visible before the script refactor.
