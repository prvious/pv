#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
TRACK=${PV_RECIPE_TRACK:-8.8}
PLATFORM=${PV_RECIPE_PLATFORM:-}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-redis}
REDIS_DEPLOYMENT_TARGET=13.0

[ -n "$PLATFORM" ] || die "PV_RECIPE_PLATFORM is required"

need cargo
need cat
need curl
need awk
need make
need tar
need uname

if [ -z "$PV_COMMIT" ]; then
  need git
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

case "$PLATFORM:$(uname -s):$(uname -m)" in
  darwin-arm64:Darwin:arm64 | darwin-amd64:Darwin:x86_64 | linux-amd64:Linux:x86_64 | linux-arm64:Linux:aarch64) ;;
  *) die "Redis artifacts must be built on a native runner for $PLATFORM" ;;
esac

case "$PLATFORM" in
  darwin-arm64 | darwin-amd64)
    need codesign
    need lipo
    need otool
    export MACOSX_DEPLOYMENT_TARGET="$REDIS_DEPLOYMENT_TARGET"
    ;;
  linux-amd64 | linux-arm64)
    need file
    need readelf
    ;;
esac

append_redis_legal_file() {
  source_path=$1
  label=$2

  [ -f "$source_path" ] || die "missing Redis third-party legal file: $source_path"
  printf '\n%s\n' "==== $label ===="
  cat "$source_path"
}

append_redis_legal_header() {
  source_path=$1
  label=$2

  [ -f "$source_path" ] || die "missing Redis third-party legal file: $source_path"
  legal_header=$(awk '
    /^[[:space:]]*\/\*/ { in_comment = 1 }
    in_comment { print }
    in_comment && /\*\// { found = 1; exit }
    END { if (!found) exit 1 }
  ' "$source_path") || die "missing Redis legal header in: $source_path"
  [ -n "$legal_header" ] || die "missing Redis legal header in: $source_path"

  printf '\n%s\n' "==== $label ===="
  printf '%s\n' "$legal_header"
}

write_third_party_notices() {
  output=$1

  {
    printf '%s\n' "Third-party notices for Redis $PV_UPSTREAM_VERSION"
    printf '%s\n' "Generated from legal files bundled in the Redis source archive."
    append_redis_legal_file "$source_dir/deps/hiredis/COPYING" "deps/hiredis/COPYING"
    append_redis_legal_file "$source_dir/deps/lua/COPYRIGHT" "deps/lua/COPYRIGHT"
    append_redis_legal_file "$source_dir/deps/hdr_histogram/LICENSE.txt" "deps/hdr_histogram/LICENSE.txt"
    append_redis_legal_file "$source_dir/deps/hdr_histogram/COPYING.txt" "deps/hdr_histogram/COPYING.txt"
    append_redis_legal_file "$source_dir/deps/fpconv/LICENSE.txt" "deps/fpconv/LICENSE.txt"
    append_redis_legal_header "$source_dir/src/fast_float_strtod.c" "src/fast_float_strtod.c"
    append_redis_legal_file "$source_dir/deps/linenoise/README.markdown" "deps/linenoise/README.markdown"
    append_redis_legal_file "$source_dir/deps/jemalloc/COPYING" "deps/jemalloc/COPYING"
    append_redis_legal_file "$source_dir/deps/tre/LICENSE" "deps/tre/LICENSE"
    append_redis_legal_file "$source_dir/deps/xxhash/LICENSE" "deps/xxhash/LICENSE"
  } >"$output"
}

recipe_dir="$ROOT/release/artifacts/recipes/redis"
env_file="$OUT_DIR/work/redis.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --redis "$recipe_dir/recipe.toml" \
  --resource redis \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"

artifact_basename=$(artifact_basename redis "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
work_dir="$OUT_DIR/work/$artifact_basename"
source_archive="$OUT_DIR/sources/redis-$PV_UPSTREAM_VERSION.tar.gz"
source_dir="$work_dir/source"
root_dir="$work_dir/$artifact_basename"
final_archive="$OUT_DIR/$artifact_basename.tar.gz"
staged_archive="$work_dir/staged-archives/$artifact_basename.tar.gz"
object_key=$(artifact_object_key redis "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
final_record=$(artifact_record_path "$RECORD_DIR" redis "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
staged_record="$work_dir/staged-records/redis/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/$artifact_basename.json"

rm -rf "$work_dir"
mkdir -p "$OUT_DIR/sources" "$source_dir" "$root_dir/bin"
curl -L --fail --show-error --silent \
  --retry 3 --retry-delay 2 --retry-all-errors \
  --connect-timeout 20 --max-time 600 \
  "$PV_SOURCE_URL" -o "$source_archive"
require_sha256 "$source_archive" "$PV_SOURCE_SHA256"
tar -xzf "$source_archive" -C "$source_dir" --strip-components 1
build_jobs=$(cpu_count)
make -C "$source_dir" BUILD_TLS=no -j"$build_jobs"
cp "$source_dir/src/redis-server" "$root_dir/bin/redis-server"
cp "$source_dir/src/redis-cli" "$root_dir/bin/redis-cli"
case "$PV_PLATFORM" in
  darwin-arm64 | darwin-amd64)
    validate_macho_binary "$root_dir/bin/redis-server" "$PV_PLATFORM" "$REDIS_DEPLOYMENT_TARGET"
    validate_macho_binary "$root_dir/bin/redis-cli" "$PV_PLATFORM" "$REDIS_DEPLOYMENT_TARGET"
    pv_recipe_ad_hoc_sign_macho_tree "$root_dir"
    ;;
  linux-amd64 | linux-arm64)
    pv_recipe_validate_elf_tree "$root_dir" "$PV_PLATFORM"
    ;;
esac
cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
cp "$recipe_dir/NOTICE" "$root_dir/NOTICE"
write_third_party_notices "$root_dir/THIRD-PARTY-NOTICES"
mkdir -p "$(dirname "$staged_archive")"
COPYFILE_DISABLE=1 tar -czf "$staged_archive" -C "$work_dir" "$artifact_basename"

write_record "$staged_record" redis "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$staged_archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/redis/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION" --license-file LICENSE --notice-file NOTICE --notice-file THIRD-PARTY-NOTICES
cargo run -p pv-release -- validate-archive --archive "$staged_archive" --record "$staged_record" --smoke-hook "$recipe_dir/smoke.sh"
mkdir -p "$(dirname "$final_archive")" "$(dirname "$final_record")"
mv "$staged_archive" "$final_archive"
mv "$staged_record" "$final_record"
printf '%s\n' "$final_archive"
