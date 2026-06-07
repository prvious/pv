#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
TRACK=${PV_RECIPE_TRACK:-2}
PLATFORM=${PV_RECIPE_PLATFORM:-any}
PV_COMMIT=${PV_COMMIT:-$(git -C "$ROOT" rev-parse HEAD)}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-composer}

need cargo
need curl
need shasum
need tar

env_file="$OUT_DIR/work/composer.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --composer "$ROOT/release/artifacts/recipes/composer/composer.toml" \
  --resource composer \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"
export PV_UPSTREAM_VERSION

work_dir="$OUT_DIR/work/composer-$PV_ARTIFACT_VERSION"
root_dir="$work_dir/composer-$PV_ARTIFACT_VERSION"
archive="$OUT_DIR/composer-$PV_ARTIFACT_VERSION.tar.gz"
record="$RECORD_DIR/composer/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/composer-$PV_ARTIFACT_VERSION-$PV_PLATFORM.json"
object_key="resources/composer/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/composer-$PV_ARTIFACT_VERSION-$PV_PLATFORM.tar.gz"

rm -rf "$work_dir"
mkdir -p "$root_dir"
curl -L --fail --show-error --silent "$PV_SOURCE_URL" -o "$root_dir/composer.phar"
require_sha256 "$root_dir/composer.phar" "$PV_SOURCE_SHA256"
cp "$ROOT/release/artifacts/recipes/composer/LICENSE" "$root_dir/LICENSE"
cp "$ROOT/release/artifacts/recipes/composer/NOTICE" "$root_dir/NOTICE"
mkdir -p "$OUT_DIR"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "composer-$PV_ARTIFACT_VERSION"

write_record "$record" composer "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/composer/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION"

cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$ROOT/release/artifacts/recipes/composer/smoke.sh"
printf '%s\n' "$archive"
