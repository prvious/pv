#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

RESOURCE=${PV_RECIPE_RESOURCE:-php}
TRACK=${PV_RECIPE_TRACK:-8.4}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
PV_COMMIT=${PV_COMMIT:-$(git -C "$ROOT" rev-parse HEAD)}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-php}
recipe_dir="$ROOT/release/artifacts/recipes/php"

case "$RESOURCE" in
  php | frankenphp) ;;
  *) die "PV_RECIPE_RESOURCE must be php or frankenphp, got $RESOURCE" ;;
esac

need awk
need cargo
need curl
need git
need lipo
need otool
need shasum
need spc
need tar

download_source() {
  source_name=$1
  source_version=$2
  source_url=$3
  source_sha256=$4
  source_archive="$OUT_DIR/sources/$source_name-$source_version-source.tar.gz"
  curl -L --fail --show-error --silent "$source_url" -o "$source_archive"
  require_sha256 "$source_archive" "$source_sha256"

  source_extract_dir="$OUT_DIR/sources/$source_name-$source_version-source"
  rm -rf "$source_extract_dir"
  mkdir -p "$source_extract_dir"
  tar -xzf "$source_archive" -C "$source_extract_dir"

  source_entry_count=0
  source_dir=
  for source_entry in "$source_extract_dir"/* "$source_extract_dir"/.[!.]* "$source_extract_dir"/..?*; do
    [ -d "$source_entry" ] || [ -f "$source_entry" ] || [ -L "$source_entry" ] || continue
    source_entry_count=$((source_entry_count + 1))
    source_dir=$source_entry
  done
  [ "$source_entry_count" -eq 1 ] || die "$source_name source archive must contain exactly one top-level source directory"
  [ -d "$source_dir" ] || die "$source_name source archive top-level entry is not a directory"
  printf '%s\n' "$source_dir"
}

expected_arch_for_platform() {
  case "$1" in
    darwin-arm64) printf '%s\n' arm64 ;;
    darwin-amd64) printf '%s\n' x86_64 ;;
    *) die "unsupported native PHP artifact platform: $1" ;;
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
    *)
      die "$binary Mach-O $metadata_kind references unmanaged runtime path $runtime_path"
      ;;
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
  version_lte "$binary_minos" "$PV_DEPLOYMENT_TARGET" || die "$binary Mach-O minimum macOS $binary_minos is newer than deployment target $PV_DEPLOYMENT_TARGET"

  validate_macho_runtime_paths "$binary"
}

env_file="$OUT_DIR/work/$RESOURCE-$TRACK-$PLATFORM.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --php "$recipe_dir/tracks.toml" \
  --resource "$RESOURCE" \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"
export PV_EXPECTED_EXTENSIONS
export PV_PHP_VERSION
export PV_UPSTREAM_VERSION
export PV_DEPLOYMENT_TARGET

artifact_basename="$RESOURCE-$PV_ARTIFACT_VERSION-$PV_PLATFORM"
work_dir="$OUT_DIR/work/$artifact_basename"
spc_work_dir="$work_dir/staticphp"
root_dir="$work_dir/$artifact_basename"
archive="$OUT_DIR/$artifact_basename.tar.gz"
record="$RECORD_DIR/$RESOURCE/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/$artifact_basename.json"
object_key="resources/$RESOURCE/$PV_TRACK/$PV_ARTIFACT_VERSION/$PV_PLATFORM/$artifact_basename.tar.gz"

rm -rf "$work_dir"
mkdir -p "$root_dir/bin" "$spc_work_dir" "$OUT_DIR/sources"

export MACOSX_DEPLOYMENT_TARGET="$PV_DEPLOYMENT_TARGET"
record_source_inputs_json=

case "$RESOURCE" in
  php)
    source_dir=$(download_source "$RESOURCE" "$PV_ARTIFACT_VERSION" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256")
    (
      cd "$spc_work_dir"
      spc build:php "$PV_BUILD_EXTENSIONS" --build-cli --dl-with-php="$PV_PHP_VERSION" --dl-custom-local "php-src:$source_dir"
    )
    [ -f "$spc_work_dir/buildroot/bin/php" ] || die "static PHP build did not produce buildroot/bin/php"
    cp "$spc_work_dir/buildroot/bin/php" "$root_dir/bin/php"
    [ -f "$root_dir/bin/php" ] || die "static PHP build did not produce bin/php"
    validate_macho_binary "$root_dir/bin/php"
    ;;
  frankenphp)
    frankenphp_source_dir=$(download_source "$RESOURCE" "$PV_ARTIFACT_VERSION" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256")
    php_source_dir=$(download_source php "$PV_PHP_VERSION" "$PV_PHP_SOURCE_URL" "$PV_PHP_SOURCE_SHA256")
    (
      cd "$spc_work_dir"
      spc build:php "$PV_BUILD_EXTENSIONS" --build-frankenphp --enable-zts --dl-with-php="$PV_PHP_VERSION" --dl-custom-local "php-src:$php_source_dir" --dl-custom-local "frankenphp:$frankenphp_source_dir"
    )
    [ -f "$spc_work_dir/buildroot/bin/frankenphp" ] || die "FrankenPHP build did not produce buildroot/bin/frankenphp"
    cp "$spc_work_dir/buildroot/bin/frankenphp" "$root_dir/bin/frankenphp"
    [ -f "$root_dir/bin/frankenphp" ] || die "FrankenPHP build did not produce bin/frankenphp"
    validate_macho_binary "$root_dir/bin/frankenphp"
    record_source_inputs_json=$(cat <<JSON
[
      {
        "name": "frankenphp",
        "source_url": "$PV_SOURCE_URL",
        "source_sha256": "$PV_SOURCE_SHA256"
      },
      {
        "name": "php",
        "source_url": "$PV_PHP_SOURCE_URL",
        "source_sha256": "$PV_PHP_SOURCE_SHA256"
      }
    ]
JSON
)
    ;;
esac

cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
{
  cat "$recipe_dir/NOTICE"
  printf '\nArtifact build metadata:\n'
  printf 'Resource: %s\n' "$RESOURCE"
  printf 'Track: %s\n' "$PV_TRACK"
  printf 'Artifact version: %s\n' "$PV_ARTIFACT_VERSION"
  printf 'Upstream version: %s\n' "$PV_UPSTREAM_VERSION"
  if [ "$RESOURCE" = "frankenphp" ]; then
    printf 'FrankenPHP source URL: %s\n' "$PV_SOURCE_URL"
    printf 'FrankenPHP source SHA-256: %s\n' "$PV_SOURCE_SHA256"
    printf 'PHP source URL: %s\n' "$PV_PHP_SOURCE_URL"
    printf 'PHP source SHA-256: %s\n' "$PV_PHP_SOURCE_SHA256"
  else
    printf 'Source URL: %s\n' "$PV_SOURCE_URL"
    printf 'Source SHA-256: %s\n' "$PV_SOURCE_SHA256"
  fi
} >"$root_dir/NOTICE"

COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"
write_record "$record" "$RESOURCE" "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/php/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION" "$record_source_inputs_json"

cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$ROOT/release/artifacts/recipes/php/smoke.sh"
printf '%s\n' "$archive"
