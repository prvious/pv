#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

TRACK=${PV_RECIPE_TRACK:-18}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-postgres}
BUILD_JOBS=${PV_BUILD_JOBS:-2}
SOURCE_CACHE_DIR=${PV_POSTGRES_SOURCE_CACHE_DIR:-"$OUT_DIR/sources"}
OPENSSL_BUNDLE_ARCHIVE=${PV_POSTGRES_OPENSSL_BUNDLE_ARCHIVE:-}
recipe_dir="$ROOT/release/artifacts/recipes/postgres"
# shellcheck source=/dev/null
. "$recipe_dir/openssl.env"
DEPLOYMENT_TARGET=$PV_POSTGRES_DEPLOYMENT_TARGET

need cargo
need curl
need diff
need dirname
need find
need git
need make
need perl
need readlink
need shasum
need sort
need tar

case "$PLATFORM" in
  darwin-arm64 | darwin-amd64) ;;
  *) die "unsupported Postgres artifact platform: $PLATFORM" ;;
esac

if [ -z "$PV_COMMIT" ]; then
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

case "$BUILD_JOBS" in
  '' | *[!0-9]*) die "Postgres build jobs must be an integer from 1 through 4" ;;
esac
[ "$BUILD_JOBS" -ge 1 ] && [ "$BUILD_JOBS" -le 4 ] || die "Postgres build jobs must be an integer from 1 through 4"

download_source() {
  source_archive=$1
  source_url=$2
  source_sha256=$3

  if [ -f "$source_archive" ]; then
    require_sha256 "$source_archive" "$source_sha256"
    return
  fi

  mkdir -p "$(dirname "$source_archive")"
  download_tmp="$source_archive.tmp.$$"
  rm -f "$download_tmp"
  (
    trap 'rm -f "$download_tmp"' 0
    curl -L --fail --show-error --silent \
      --retry 3 --retry-delay 2 --retry-all-errors \
      --connect-timeout 20 --max-time 1200 \
      "$source_url" -o "$download_tmp"
    require_sha256 "$download_tmp" "$source_sha256"
    mv "$download_tmp" "$source_archive"
    trap - 0
  )
}

extract_source() {
  source_name=$1
  source_archive=$2
  source_extract_dir=$3

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

copy_install_tree() {
  install_dir=$1
  root_dir=$2
  openssl_prefix=$3
  extension_catalog=$4
  actual_extension_catalog=$5

  mkdir -p "$root_dir"
  tar -cf - -C "$install_dir" . | tar -xf - -C "$root_dir"
  find "$root_dir" -type l -exec sh -c '
    for path do
      target=$(readlink "$path") || exit 1
      case "$target" in
        /*) source=$target ;;
        *) source=$(dirname "$path")/$target ;;
      esac
      tmp=$path.pv-copy.$$
      rm "$path" || exit 1
      cp -p "$source" "$tmp" || exit 1
      mv "$tmp" "$path" || exit 1
    done
  ' sh {} +
  find "$root_dir" -type f -links +1 -exec sh -c '
    for path do
      tmp=$path.pv-copy.$$
      cp -p "$path" "$tmp" || exit 1
      mv "$tmp" "$path" || exit 1
    done
  ' sh {} +
  mkdir -p "$root_dir/lib"
  cp "$openssl_prefix/lib/libssl.3.dylib" "$root_dir/lib/libssl.3.dylib"
  cp "$openssl_prefix/lib/libcrypto.3.dylib" "$root_dir/lib/libcrypto.3.dylib"

  extension_controls=$(find "$root_dir/share/extension" -type f -name '*.control') || die "failed to inspect installed Postgres extension controls"
  unsorted_extension_catalog="$actual_extension_catalog.unsorted"
  : >"$unsorted_extension_catalog"
  if [ -n "$extension_controls" ]; then
    printf '%s\n' "$extension_controls" | while IFS= read -r control_file; do
      control_name=${control_file##*/}
      printf '%s\n' "${control_name%.control}"
    done >"$unsorted_extension_catalog"
  fi
  LC_ALL=C sort "$unsorted_extension_catalog" >"$actual_extension_catalog" || die "failed to sort installed Postgres extension controls"
  if diff -u "$extension_catalog" "$actual_extension_catalog"; then
    :
  else
    diff_status=$?
    [ "$diff_status" -eq 1 ] || die "failed to compare Postgres extension catalogs"
    die "Postgres supplied extension catalog does not match track $PV_TRACK"
  fi

  pv_recipe_cleanup_macho_rpaths_tree "$root_dir"
  rewrite_macho_install_names "$root_dir" "$install_dir" "$openssl_prefix"
  pv_recipe_ad_hoc_sign_macho_tree "$root_dir"
  for binary in postgres initdb pg_ctl psql; do
    [ -x "$root_dir/bin/$binary" ] || die "Postgres artifact missing bin/$binary"
    pv_recipe_validate_macho_binary "$root_dir/bin/$binary" "$PLATFORM" "$DEPLOYMENT_TARGET"
  done
  for library in \
    lib/libcrypto.3.dylib \
    lib/libssl.3.dylib \
    lib/postgresql/pg_trgm.dylib \
    lib/postgresql/pgcrypto.dylib \
    lib/postgresql/sslinfo.dylib; do
    [ -f "$root_dir/$library" ] || die "Postgres artifact missing $library"
    pv_recipe_validate_macho_binary "$root_dir/$library" "$PLATFORM" "$DEPLOYMENT_TARGET"
  done
  for macho_dir in "$root_dir/bin" "$root_dir/lib"; do
    find "$macho_dir" -type f | while IFS= read -r macho; do
      pv_recipe_is_macho "$macho" || continue
      pv_recipe_validate_macho_binary "$macho" "$PLATFORM" "$DEPLOYMENT_TARGET"
    done
  done
  cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
  cp "$recipe_dir/NOTICE" "$root_dir/NOTICE"
  cp "$openssl_prefix/LICENSE.txt" "$root_dir/OPENSSL-LICENSE"
  cp "$recipe_dir/THIRD-PARTY-NOTICES" "$root_dir/THIRD-PARTY-NOTICES"
}

env_file="$OUT_DIR/work/postgres-$TRACK-$PLATFORM.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --postgres "$recipe_dir/recipe.toml" \
  --resource postgres \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"
export PV_UPSTREAM_VERSION

artifact_basename=$(artifact_basename postgres "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
work_dir="$OUT_DIR/work/postgres-$PV_TRACK-$artifact_basename"
source_archive="$SOURCE_CACHE_DIR/postgresql-$PV_UPSTREAM_VERSION.tar.gz"
source_extract_dir="$work_dir/postgresql-source"
openssl_prefix=${PV_POSTGRES_OPENSSL_PREFIX:-"$work_dir/openssl-$PV_POSTGRES_OPENSSL_VERSION"}
local_openssl_bundle="$work_dir/postgres-openssl.tar.gz"
install_dir="$work_dir/install"
root_dir="$work_dir/$artifact_basename"
extension_catalog="$recipe_dir/catalogs/$PV_TRACK.txt"
actual_extension_catalog="$work_dir/actual-extension-catalog.txt"
archive="$OUT_DIR/$artifact_basename.tar.gz"
record=$(artifact_record_path "$RECORD_DIR" postgres "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
object_key=$(artifact_object_key postgres "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")

rm -rf "$work_dir"
mkdir -p "$work_dir" "$install_dir" "$OUT_DIR"
[ -f "$extension_catalog" ] || die "missing Postgres extension catalog for track $PV_TRACK"
dependency_command=build
dependency_bundle=$local_openssl_bundle
if [ -n "$OPENSSL_BUNDLE_ARCHIVE" ]; then
  dependency_command=use
  dependency_bundle=$OPENSSL_BUNDLE_ARCHIVE
fi
PV_RECIPE_PLATFORM="$PLATFORM" \
  PV_BUILD_JOBS="$BUILD_JOBS" \
  PV_POSTGRES_DEPENDENCY_WORK_DIR="$work_dir/dependency-work" \
  PV_POSTGRES_SOURCE_CACHE_DIR="$SOURCE_CACHE_DIR" \
  "$recipe_dir/dependencies.sh" "$dependency_command" "$dependency_bundle" "$openssl_prefix"
download_source "$source_archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256"
source_dir=$(extract_source Postgres "$source_archive" "$source_extract_dir")

export CFLAGS="${CFLAGS:-} -mmacosx-version-min=$DEPLOYMENT_TARGET"
export LDFLAGS="${LDFLAGS:-} -mmacosx-version-min=$DEPLOYMENT_TARGET"
(
  cd "$source_dir"
  ./configure \
    --prefix="$install_dir" \
    --with-ssl=openssl \
    --with-includes="$openssl_prefix/include" \
    --with-libraries="$openssl_prefix/lib" \
    --without-icu \
    --without-llvm \
    --without-lz4 \
    --without-readline \
    --without-zlib \
    --without-zstd
  make -j "$BUILD_JOBS" world-bin pkglibdir="$install_dir/lib/postgresql"
  make install-world-bin pkglibdir="$install_dir/lib/postgresql"
)

copy_install_tree "$install_dir" "$root_dir" "$openssl_prefix" "$extension_catalog" "$actual_extension_catalog"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"
write_record \
  "$record" \
  postgres \
  "$PV_TRACK" \
  "$PV_UPSTREAM_VERSION" \
  "$PV_PV_BUILD_REVISION" \
  "$PV_PLATFORM" \
  "$object_key" \
  "$archive" \
  "$PV_SOURCE_URL" \
  "$PV_SOURCE_SHA256" \
  release/artifacts/recipes/postgres/build.sh \
  "$PV_COMMIT" \
  "$BUILD_RUN_ID" \
  "$PV_MINIMUM_PV_VERSION" \
  --license-file LICENSE \
  --license-file OPENSSL-LICENSE \
  --notice-file NOTICE \
  --notice-file THIRD-PARTY-NOTICES \
  --source-input openssl "$PV_POSTGRES_OPENSSL_SOURCE_URL" "$PV_POSTGRES_OPENSSL_SOURCE_SHA256"

PV_POSTGRES_EXTENSION_CATALOG="$extension_catalog" \
  PV_POSTGRES_PLATFORM="$PV_PLATFORM" \
  PV_POSTGRES_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
  PV_UPSTREAM_VERSION="$PV_UPSTREAM_VERSION" \
  cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$recipe_dir/smoke.sh"
printf '%s\n' "$archive"
