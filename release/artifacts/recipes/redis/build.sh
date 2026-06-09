#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
TRACK=${PV_RECIPE_TRACK:-8.2}
PLATFORM=${PV_RECIPE_PLATFORM:-}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-redis}
REDIS_DEPLOYMENT_TARGET=13.0

[ -n "$PLATFORM" ] || die "PV_RECIPE_PLATFORM is required"

need cargo
need cat
need codesign
need curl
need awk
need lipo
need make
need otool
need shasum
need sysctl
need tar
need uname

if [ -z "$PV_COMMIT" ]; then
  need git
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

case "$PLATFORM:$(uname -s):$(uname -m)" in
  darwin-arm64:Darwin:arm64 | darwin-amd64:Darwin:x86_64) ;;
  *) die "Redis artifacts must be built on a native runner for $PLATFORM" ;;
esac

export MACOSX_DEPLOYMENT_TARGET="$REDIS_DEPLOYMENT_TARGET"

expected_arch_for_platform() {
  case "$1" in
    darwin-arm64) printf '%s\n' arm64 ;;
    darwin-amd64) printf '%s\n' x86_64 ;;
    *) die "unsupported native Redis artifact platform: $1" ;;
  esac
}

macho_minimum_os() {
  otool -l "$1" | awk '
    $1 == "cmd" && $2 == "LC_BUILD_VERSION" {
      in_build_version = 1
      in_version_min = 0
      next
    }
    $1 == "cmd" && $2 == "LC_VERSION_MIN_MACOSX" {
      in_build_version = 0
      in_version_min = 1
      next
    }
    $1 == "cmd" {
      in_build_version = 0
      in_version_min = 0
      next
    }
    in_build_version && $1 == "minos" {
      print $2
      exit
    }
    in_version_min && $1 == "version" {
      print $2
      exit
    }
  '
}

version_lte() {
  actual_version=$1
  maximum_version=$2
  awk -v actual="$actual_version" -v maximum="$maximum_version" '
    BEGIN {
      split(actual, actual_parts, ".")
      split(maximum, maximum_parts, ".")
      for (part_index = 1; part_index <= 3; part_index++) {
        actual_part = actual_parts[part_index] == "" ? 0 : actual_parts[part_index] + 0
        maximum_part = maximum_parts[part_index] == "" ? 0 : maximum_parts[part_index] + 0
        if (actual_part < maximum_part) {
          exit 0
        }
        if (actual_part > maximum_part) {
          exit 1
        }
      }
      exit 0
    }
  '
}

reject_unmanaged_macho_runtime_path() {
  binary=$1
  metadata_kind=$2
  runtime_path=$3

  case "$runtime_path" in
    /usr/lib/* | /System/Library/* | @rpath/* | @loader_path/* | @executable_path/*)
      ;;
    @loader_path | @executable_path)
      [ "$metadata_kind" = "rpath" ] || die "$binary Mach-O $metadata_kind references unmanaged runtime path $runtime_path"
      ;;
    *) die "$binary Mach-O $metadata_kind references unmanaged runtime path $runtime_path" ;;
  esac
}

macho_rpaths() {
  otool -l "$1" | awk '
    $1 == "cmd" && $2 == "LC_RPATH" {
      in_rpath = 1
      next
    }
    $1 == "cmd" {
      in_rpath = 0
      next
    }
    in_rpath && $1 == "path" {
      print $2
      in_rpath = 0
      next
    }
  '
}

validate_macho_runtime_paths() {
  binary=$1
  linked_libraries=$(otool -L "$binary")
  printf '%s\n' "$linked_libraries" | awk 'NR > 1 && NF > 0 { print $1 }' | while IFS= read -r linked_library; do
    reject_unmanaged_macho_runtime_path "$binary" "linked library" "$linked_library"
  done

  macho_rpaths "$binary" | while IFS= read -r macho_rpath; do
    reject_unmanaged_macho_runtime_path "$binary" "rpath" "$macho_rpath"
  done
}

validate_macho_binary() {
  binary=$1
  expected_arch=$(expected_arch_for_platform "$PV_PLATFORM")
  binary_archs=$(lipo -archs "$binary")
  [ "$binary_archs" = "$expected_arch" ] || die "$binary Mach-O architecture $binary_archs does not match expected $expected_arch for $PV_PLATFORM"

  binary_minos=$(macho_minimum_os "$binary")
  [ -n "$binary_minos" ] || die "$binary Mach-O minimum macOS version not found"
  version_lte "$binary_minos" "$REDIS_DEPLOYMENT_TARGET" || die "$binary Mach-O minimum macOS $binary_minos is newer than deployment target $REDIS_DEPLOYMENT_TARGET"

  validate_macho_runtime_paths "$binary"
}

sign_macho_binary() {
  binary=$1
  codesign --force --sign - "$binary"
  codesign --verify "$binary"
}

append_redis_legal_file() {
  source_path=$1
  label=$2

  [ -f "$source_path" ] || die "missing Redis third-party legal file: $source_path"
  printf '\n%s\n' "==== $label ===="
  cat "$source_path"
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
    append_redis_legal_file "$source_dir/deps/fast_float/README.md" "deps/fast_float/README.md"
    append_redis_legal_file "$source_dir/deps/linenoise/README.markdown" "deps/linenoise/README.markdown"
    append_redis_legal_file "$source_dir/deps/jemalloc/COPYING" "deps/jemalloc/COPYING"
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
make -C "$source_dir" BUILD_TLS=no -j"$(sysctl -n hw.ncpu)"
validate_macho_binary "$source_dir/src/redis-server"
validate_macho_binary "$source_dir/src/redis-cli"
cp "$source_dir/src/redis-server" "$root_dir/bin/redis-server"
cp "$source_dir/src/redis-cli" "$root_dir/bin/redis-cli"
pv_recipe_ad_hoc_sign_macho_tree "$root_dir"
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
