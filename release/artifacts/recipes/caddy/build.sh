#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
TRACK=${PV_RECIPE_TRACK:-2}
PLATFORM=${PV_RECIPE_PLATFORM:-}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-caddy}
CADDY_DEPLOYMENT_TARGET=13.0
recipe_dir="$ROOT/release/artifacts/recipes/caddy"

[ -n "$PLATFORM" ] || die "PV_RECIPE_PLATFORM is required"

need awk
need cargo
need codesign
need curl
need lipo
need otool
need shasum
need tar
need uname

if [ -z "$PV_COMMIT" ]; then
  need git
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

case "$PLATFORM:$(uname -s):$(uname -m)" in
  darwin-arm64:Darwin:arm64 | darwin-amd64:Darwin:x86_64) ;;
  *) die "Caddy artifacts must be built on a native runner for $PLATFORM" ;;
esac

env_file="$OUT_DIR/work/caddy-$TRACK-$PLATFORM.env"
mkdir -p "$(dirname "$env_file")" "$OUT_DIR/sources"
cargo run -p pv-release -- print-recipe-env \
  --caddy "$recipe_dir/recipe.toml" \
  --resource caddy \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"

source_archive="$OUT_DIR/sources/caddy-$PV_UPSTREAM_VERSION-$PV_PLATFORM.tar.gz"
curl -L --fail --show-error --silent \
  --retry 3 --retry-delay 2 --retry-all-errors \
  --connect-timeout 20 --max-time 300 \
  "$PV_SOURCE_URL" -o "$source_archive"
require_sha256 "$source_archive" "$PV_SOURCE_SHA256"

artifact_basename=$(artifact_basename caddy "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
work_dir="$OUT_DIR/work/$artifact_basename"
extract_dir="$work_dir/source"
root_dir="$work_dir/$artifact_basename"
final_archive="$OUT_DIR/$artifact_basename.tar.gz"
staged_archive="$work_dir/staged-archives/$artifact_basename.tar.gz"
final_record=$(artifact_record_path "$RECORD_DIR" caddy "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
staged_record="$work_dir/staged-records/caddy/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/$artifact_basename.json"
object_key=$(artifact_object_key caddy "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")

rm -rf "$work_dir"
mkdir -p "$extract_dir" "$root_dir/bin"
tar -xzf "$source_archive" -C "$extract_dir"
[ -f "$extract_dir/caddy" ] || die "Caddy upstream archive did not contain caddy"
cp "$extract_dir/caddy" "$root_dir/bin/caddy"
chmod 755 "$root_dir/bin/caddy"
validate_macho_binary "$root_dir/bin/caddy" "$PV_PLATFORM" "$CADDY_DEPLOYMENT_TARGET"
pv_recipe_ad_hoc_sign_macho_tree "$root_dir"
cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
cp "$recipe_dir/NOTICE" "$root_dir/NOTICE"

mkdir -p "$(dirname "$staged_archive")" "$(dirname "$staged_record")"
COPYFILE_DISABLE=1 tar -czf "$staged_archive" -C "$work_dir" "$artifact_basename"
write_record "$staged_record" caddy "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$staged_archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/caddy/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION" --license-file LICENSE --notice-file NOTICE

PV_UPSTREAM_VERSION="$PV_UPSTREAM_VERSION" \
  cargo run -p pv-release -- validate-archive --archive "$staged_archive" --record "$staged_record" --smoke-hook "$recipe_dir/smoke.sh"
mkdir -p "$OUT_DIR" "$(dirname "$final_record")"
mv "$staged_archive" "$final_archive"
mv "$staged_record" "$final_record"
printf '%s\n' "$final_archive"
