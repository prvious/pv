#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
TRACK=${PV_RECIPE_TRACK:-1}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-mailpit}
recipe_dir="$ROOT/release/artifacts/recipes/mailpit"

need cargo
need curl
need git
need shasum
need tar

if [ -z "$PV_COMMIT" ]; then
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

env_file="$OUT_DIR/work/mailpit-$TRACK-$PLATFORM.env"
mkdir -p "$(dirname "$env_file")" "$OUT_DIR/sources"
cargo run -p pv-release -- print-recipe-env \
  --mailpit "$recipe_dir/recipe.toml" \
  --resource mailpit \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"

source_archive="$OUT_DIR/sources/mailpit-$PV_UPSTREAM_VERSION-$PV_PLATFORM.tar.gz"
curl -L --fail --show-error --silent \
  --retry 3 --retry-delay 2 --retry-all-errors \
  --connect-timeout 20 --max-time 300 \
  "$PV_SOURCE_URL" -o "$source_archive"
require_sha256 "$source_archive" "$PV_SOURCE_SHA256"

artifact_basename=$(artifact_basename mailpit "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
work_dir="$OUT_DIR/work/$artifact_basename"
extract_dir="$work_dir/source"
root_dir="$work_dir/$artifact_basename"
archive="$OUT_DIR/$artifact_basename.tar.gz"
record=$(artifact_record_path "$RECORD_DIR" mailpit "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
object_key=$(artifact_object_key mailpit "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")

rm -rf "$work_dir"
mkdir -p "$extract_dir" "$root_dir/bin"
tar -xzf "$source_archive" -C "$extract_dir"
[ -f "$extract_dir/mailpit" ] || die "Mailpit upstream archive did not contain mailpit"
cp "$extract_dir/mailpit" "$root_dir/bin/mailpit"
chmod 755 "$root_dir/bin/mailpit"
cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
{
  cat "$recipe_dir/NOTICE"
  printf '\nArtifact build metadata:\n'
  printf 'Resource: %s\n' "$PV_RESOURCE"
  printf 'Track: %s\n' "$PV_TRACK"
  printf 'Artifact version: %s\n' "$PV_ARTIFACT_VERSION"
  printf 'Upstream version: %s\n' "$PV_UPSTREAM_VERSION"
  printf 'Source URL: %s\n' "$PV_SOURCE_URL"
  printf 'Source SHA-256: %s\n' "$PV_SOURCE_SHA256"
} >"$root_dir/NOTICE"

mkdir -p "$OUT_DIR" "$(dirname "$record")"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"
write_record "$record" mailpit "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/mailpit/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION"

PV_UPSTREAM_VERSION="$PV_UPSTREAM_VERSION" \
  cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$recipe_dir/smoke.sh"
printf '%s\n' "$archive"
