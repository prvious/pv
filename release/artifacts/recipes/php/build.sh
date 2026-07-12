#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

TRACK=${PV_RECIPE_TRACK:-8.5}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-php}
recipe_dir="$ROOT/release/artifacts/recipes/php"

need awk
need cargo
need curl
need git
need lipo
need otool
need patch
need shasum
need spc
need tar

if [ -z "$PV_COMMIT" ]; then
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

download_source() {
  source_name=$1
  source_version=$2
  source_url=$3
  source_sha256=$4
  source_archive="$OUT_DIR/sources/$source_name-$source_version-source.tar.gz"
  curl -L --fail --show-error --silent \
    --retry 3 --retry-delay 2 --retry-all-errors \
    --connect-timeout 20 --max-time 600 \
    "$source_url" -o "$source_archive"
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

prepare_staticphp_php83_frankenphp_patch_context() {
  php_source_dir=$1
  frankenphp_source_dir=$2

  case "$PHP_PHP_VERSION" in
    8.3.*)
      php_m4="$php_source_dir/build/php.m4"
      staticphp_patch="$recipe_dir/patches/staticphp/spc_fix_avx512_cache_before_80400.patch"

      [ -f "$php_m4" ] || die "PHP 8.3 StaticPHP patch context missing $php_m4"
      [ -f "$staticphp_patch" ] || die "PHP 8.3 StaticPHP patch file missing $staticphp_patch"

      mkdir -p "$frankenphp_source_dir/build"
      cp "$php_m4" "$frankenphp_source_dir/build/php.m4"
      patch -d "$frankenphp_source_dir" -p1 <"$staticphp_patch"
      ;;
  esac
}

csv_contains() {
  list=$1
  item=$2

  case ",$list," in
    *",$item,"*) return 0 ;;
    *) return 1 ;;
  esac
}

csv_without() {
  list=$1
  shift

  result=
  old_ifs=$IFS
  IFS=,
  for item in $list; do
    [ -n "$item" ] || continue
    skip=
    for excluded in "$@"; do
      if [ "$item" = "$excluded" ]; then
        skip=1
        break
      fi
    done
    [ -z "$skip" ] || continue
    if [ -z "$result" ]; then
      result=$item
    else
      result=$result,$item
    fi
  done
  IFS=$old_ifs
  printf '%s\n' "$result"
}

ensure_pkg_config() {
  if [ -z "${PKG_CONFIG:-}" ]; then
    if command -v pkg-config >/dev/null 2>&1; then
      PKG_CONFIG=pkg-config
    elif command -v pkgconf >/dev/null 2>&1; then
      PKG_CONFIG=pkgconf
    else
      die "pkg-config is required to build the shared imagick extension"
    fi
  fi
  if [ -z "${PKG_CONFIG_LIBDIR:-}" ]; then
    PKG_CONFIG_LIBDIR="$spc_work_dir/buildroot/lib/pkgconfig"
  fi
  export PKG_CONFIG PKG_CONFIG_LIBDIR
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
    @loader_path | @executable_path)
      [ "$metadata_kind" = "rpath" ] || die "$binary Mach-O $metadata_kind references unmanaged runtime path $runtime_path"
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

delete_known_stale_macho_rpaths() {
  binary=$1

  macho_rpaths "$binary" | while IFS= read -r macho_rpath; do
    case "$macho_rpath" in
      /usr/local/lib | "$spc_work_dir/buildroot/lib")
        need install_name_tool
        install_name_tool -delete_rpath "$macho_rpath" "$binary"
        ;;
    esac
  done
}

validate_macho_binary() {
  binary=$1
  expected_arch=$(expected_arch_for_platform "$PV_PLATFORM")
  binary_archs=$(lipo -archs "$binary")
  [ "$binary_archs" = "$expected_arch" ] || die "$binary Mach-O architecture $binary_archs does not match expected $expected_arch for $PV_PLATFORM"

  binary_minos=$(macho_minimum_os "$binary")
  [ -n "$binary_minos" ] || die "$binary Mach-O minimum macOS version not found"
  version_lte "$binary_minos" "$PHP_DEPLOYMENT_TARGET" || die "$binary Mach-O minimum macOS $binary_minos is newer than deployment target $PHP_DEPLOYMENT_TARGET"

  validate_macho_runtime_paths "$binary"
}

print_php_env() {
  resource=$1
  env_file=$2
  cargo run -p pv-release -- print-recipe-env \
    --php "$recipe_dir/tracks.toml" \
    --resource "$resource" \
    --track "$TRACK" \
    --platform "$PLATFORM" >"$env_file"
}

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
# print-recipe-env writes these PV_* assignments; ShellCheck cannot infer generated env files.
# shellcheck disable=SC2153
{
  PHP_UPSTREAM_VERSION=$PV_UPSTREAM_VERSION
  PHP_ARTIFACT_VERSION=$PV_ARTIFACT_VERSION
  PHP_SOURCE_URL=$PV_SOURCE_URL
  PHP_SOURCE_SHA256=$PV_SOURCE_SHA256
  PHP_PHP_VERSION=$PV_PHP_VERSION
  PHP_DEFAULT_EXTENSIONS=$PV_DEFAULT_EXTENSIONS
  PHP_OPTIONAL_EXTENSIONS=$PV_OPTIONAL_EXTENSIONS
  PHP_BUILD_EXTENSIONS=$PV_BUILD_EXTENSIONS
  PHP_EXPECTED_EXTENSIONS=$PV_EXPECTED_EXTENSIONS
  PHP_DEPLOYMENT_TARGET=$PV_DEPLOYMENT_TARGET
  PHP_MINIMUM_PV_VERSION=$PV_MINIMUM_PV_VERSION
  PHP_PV_BUILD_REVISION=$PV_PV_BUILD_REVISION
}

print_php_env frankenphp "$frankenphp_env_file"
# shellcheck source=/dev/null
. "$frankenphp_env_file"
# print-recipe-env writes these PV_* assignments; ShellCheck cannot infer generated env files.
# shellcheck disable=SC2153
{
  FRANKENPHP_UPSTREAM_VERSION=$PV_UPSTREAM_VERSION
  FRANKENPHP_ARTIFACT_VERSION=$PV_ARTIFACT_VERSION
  FRANKENPHP_SOURCE_URL=$PV_SOURCE_URL
  FRANKENPHP_SOURCE_SHA256=$PV_SOURCE_SHA256
  FRANKENPHP_MINIMUM_PV_VERSION=$PV_MINIMUM_PV_VERSION
  FRANKENPHP_PV_BUILD_REVISION=$PV_PV_BUILD_REVISION
}

# shellcheck disable=SC2153
{
  [ "$PV_PHP_VERSION" = "$PHP_PHP_VERSION" ] || die "PHP pair metadata mismatch: php env has $PHP_PHP_VERSION but frankenphp env has $PV_PHP_VERSION"
  [ "$PV_DEFAULT_EXTENSIONS" = "$PHP_DEFAULT_EXTENSIONS" ] || die "PHP pair metadata mismatch: default extension sets differ"
  [ "$PV_OPTIONAL_EXTENSIONS" = "$PHP_OPTIONAL_EXTENSIONS" ] || die "PHP pair metadata mismatch: optional extension sets differ"
  [ "$PV_BUILD_EXTENSIONS" = "$PHP_BUILD_EXTENSIONS" ] || die "PHP pair metadata mismatch: extension build sets differ"
  [ "$PV_EXPECTED_EXTENSIONS" = "$PHP_EXPECTED_EXTENSIONS" ] || die "PHP pair metadata mismatch: expected extension sets differ"
  [ "$PV_DEPLOYMENT_TARGET" = "$PHP_DEPLOYMENT_TARGET" ] || die "PHP pair metadata mismatch: deployment targets differ"
}

PHP_ARTIFACT_OPTIONAL_EXTENSIONS=$PHP_OPTIONAL_EXTENSIONS
# StaticPHP builds SQL Server modules for php-cli, but FrankenPHP's embedded
# php-cli does not report them as loaded. Do not advertise broken optional modules.
FRANKENPHP_ARTIFACT_OPTIONAL_EXTENSIONS=$(csv_without "$PHP_OPTIONAL_EXTENSIONS" sqlsrv pdo_sqlsrv)

export MACOSX_DEPLOYMENT_TARGET="$PHP_DEPLOYMENT_TARGET"

php_source_dir=$(download_source php "$PHP_PHP_VERSION" "$PHP_SOURCE_URL" "$PHP_SOURCE_SHA256")
frankenphp_source_dir=$(download_source frankenphp "$FRANKENPHP_ARTIFACT_VERSION" "$FRANKENPHP_SOURCE_URL" "$FRANKENPHP_SOURCE_SHA256")
prepare_staticphp_php83_frankenphp_patch_context "$php_source_dir" "$frankenphp_source_dir"

(
  cd "$spc_work_dir"
  # StaticPHP dependency downloads default to no retries; GNU mirrors can return transient 5xxs.
  optional_shared_args=
  if [ -n "$PHP_OPTIONAL_EXTENSIONS" ]; then
    optional_shared_args="--build-shared=$PHP_OPTIONAL_EXTENSIONS"
  fi
  if csv_contains "$PHP_OPTIONAL_EXTENSIONS" imagick; then
    ensure_pkg_config
    # StaticPHP v3 finds this header but omits its parent include dir for shared imagick builds.
    imagick_include="-I$spc_work_dir/buildroot/include/ImageMagick-7"
    CFLAGS="${CFLAGS:+$CFLAGS }$imagick_include"
    CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$imagick_include"
    export CFLAGS CXXFLAGS
  fi
  # StaticPHP v3 selects mbregex separately from mbstring; Laravel uses mb_split().
  # shellcheck disable=SC2086
  spc build:php "$PHP_BUILD_EXTENSIONS,mbregex" \
    $optional_shared_args \
    --build-cli \
    --build-frankenphp \
    --enable-zts \
    --with-config-file-path=/var/empty/com.prvious.pv/php \
    --with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d \
    --dl-with-php="$PHP_PHP_VERSION" \
    --dl-retry=3 \
    --dl-custom-local "php-src:$php_source_dir" \
    --dl-custom-local "frankenphp:$frankenphp_source_dir"
)

[ -f "$spc_work_dir/buildroot/bin/php" ] || die "StaticPHP pair build did not produce buildroot/bin/php"
[ -f "$spc_work_dir/buildroot/bin/frankenphp" ] || die "StaticPHP pair build did not produce buildroot/bin/frankenphp"

stage_optional_php_extensions() {
  root_dir=$1
  optional_extensions=$2
  mkdir -p "$root_dir/lib/php/extensions" "$root_dir/share/pv"
  metadata="$root_dir/share/pv/php-extensions.json"
  printf '[' >"$metadata"
  first=1
  old_ifs=$IFS
  IFS=,
  for extension in $optional_extensions; do
    [ -n "$extension" ] || continue
    module=$(find "$spc_work_dir/buildroot" -type f -name "$extension.so" | head -n 1)
    [ -n "$module" ] || die "optional PHP extension $extension did not produce a shared module"
    staged_module="$root_dir/lib/php/extensions/$extension.so"
    cp "$module" "$staged_module"
    delete_known_stale_macho_rpaths "$staged_module"
    validate_macho_binary "$staged_module"
    load_kind=extension
    [ "$extension" = "xdebug" ] && load_kind=zend_extension
    [ "$first" -eq 1 ] || printf ',' >>"$metadata"
    first=0
    printf '{"name":"%s","load_kind":"%s","path":"lib/php/extensions/%s.so"}' "$extension" "$load_kind" "$extension" >>"$metadata"
  done
  IFS=$old_ifs
  printf ']\n' >>"$metadata"
}

stage_artifact() {
  resource=$1
  upstream_version=$2
  artifact_version=$3
  source_url=$4
  source_sha256=$5
  binary_name=$6
  optional_extensions=$7

  artifact_basename="$resource-$artifact_version-$PLATFORM"
  root_dir="$work_dir/$artifact_basename"

  mkdir -p "$root_dir/bin"
  cp "$spc_work_dir/buildroot/bin/$binary_name" "$root_dir/bin/$binary_name"
  [ -f "$root_dir/bin/$binary_name" ] || die "$resource artifact did not produce bin/$binary_name"
  stage_optional_php_extensions "$root_dir" "$optional_extensions"
  delete_known_stale_macho_rpaths "$root_dir/bin/$binary_name"
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
}

write_staged_artifact() {
  resource=$1
  upstream_version=$2
  artifact_version=$3
  source_url=$4
  source_sha256=$5
  minimum_pv_version=$6
  pv_build_revision=$7
  expected_extensions=$8
  optional_extensions=$9
  shift 9

  artifact_basename="$resource-$artifact_version-$PLATFORM"
  archive="$work_dir/staged-archives/$artifact_basename.tar.gz"
  record="$work_dir/staged-records/$resource/$TRACK/$artifact_version/$PLATFORM/$artifact_basename.json"
  object_key="resources/$resource/$TRACK/$artifact_version/$PLATFORM/$artifact_basename.tar.gz"

  mkdir -p "$(dirname "$archive")"
  COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"

  old_ifs=$IFS
  IFS=,
  for extension in $optional_extensions; do
    [ -n "$extension" ] || continue
    load_kind=extension
    [ "$extension" = "xdebug" ] && load_kind=zend_extension
    set -- "$@" --php-extension "$extension" "$load_kind" "lib/php/extensions/$extension.so"
  done
  IFS=$old_ifs

  write_record "$record" "$resource" "$TRACK" "$upstream_version" "$pv_build_revision" "$PLATFORM" "$object_key" "$archive" "$source_url" "$source_sha256" release/artifacts/recipes/php/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$minimum_pv_version" "$@"

  PV_EXPECTED_EXTENSIONS="$expected_extensions" \
    PV_PHP_VERSION="$PHP_PHP_VERSION" \
    PV_UPSTREAM_VERSION="$upstream_version" \
    PV_DEPLOYMENT_TARGET="$PHP_DEPLOYMENT_TARGET" \
    cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$ROOT/release/artifacts/recipes/php/smoke.sh"
}

publish_artifact() {
  resource=$1
  artifact_version=$2

  artifact_basename="$resource-$artifact_version-$PLATFORM"
  staged_archive="$work_dir/staged-archives/$artifact_basename.tar.gz"
  staged_record="$work_dir/staged-records/$resource/$TRACK/$artifact_version/$PLATFORM/$artifact_basename.json"
  final_archive="$OUT_DIR/$artifact_basename.tar.gz"
  final_record="$RECORD_DIR/$resource/$TRACK/$artifact_version/$PLATFORM/$artifact_basename.json"

  mkdir -p "$(dirname "$final_archive")" "$(dirname "$final_record")"
  mv "$staged_archive" "$final_archive"
  mv "$staged_record" "$final_record"
}

stage_artifact php \
  "$PHP_UPSTREAM_VERSION" \
  "$PHP_ARTIFACT_VERSION" \
  "$PHP_SOURCE_URL" \
  "$PHP_SOURCE_SHA256" \
  php \
  "$PHP_ARTIFACT_OPTIONAL_EXTENSIONS"

stage_artifact frankenphp \
  "$FRANKENPHP_UPSTREAM_VERSION" \
  "$FRANKENPHP_ARTIFACT_VERSION" \
  "$FRANKENPHP_SOURCE_URL" \
  "$FRANKENPHP_SOURCE_SHA256" \
  frankenphp \
  "$FRANKENPHP_ARTIFACT_OPTIONAL_EXTENSIONS"

write_staged_artifact php \
  "$PHP_UPSTREAM_VERSION" \
  "$PHP_ARTIFACT_VERSION" \
  "$PHP_SOURCE_URL" \
  "$PHP_SOURCE_SHA256" \
  "$PHP_MINIMUM_PV_VERSION" \
  "$PHP_PV_BUILD_REVISION" \
  "$PHP_EXPECTED_EXTENSIONS" \
  "$PHP_ARTIFACT_OPTIONAL_EXTENSIONS"

write_staged_artifact frankenphp \
  "$FRANKENPHP_UPSTREAM_VERSION" \
  "$FRANKENPHP_ARTIFACT_VERSION" \
  "$FRANKENPHP_SOURCE_URL" \
  "$FRANKENPHP_SOURCE_SHA256" \
  "$FRANKENPHP_MINIMUM_PV_VERSION" \
  "$FRANKENPHP_PV_BUILD_REVISION" \
  "$PHP_EXPECTED_EXTENSIONS" \
  "$FRANKENPHP_ARTIFACT_OPTIONAL_EXTENSIONS" \
  --source-input frankenphp "$FRANKENPHP_SOURCE_URL" "$FRANKENPHP_SOURCE_SHA256" \
  --source-input php "$PHP_SOURCE_URL" "$PHP_SOURCE_SHA256"

publish_artifact php "$PHP_ARTIFACT_VERSION"
publish_artifact frankenphp "$FRANKENPHP_ARTIFACT_VERSION"

printf '%s\n' "$OUT_DIR/php-$PHP_ARTIFACT_VERSION-$PLATFORM.tar.gz"
printf '%s\n' "$OUT_DIR/frankenphp-$FRANKENPHP_ARTIFACT_VERSION-$PLATFORM.tar.gz"
