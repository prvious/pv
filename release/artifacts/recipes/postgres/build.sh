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
BUILD_JOBS=${PV_BUILD_JOBS:-}
DEPLOYMENT_TARGET=13.0
recipe_dir="$ROOT/release/artifacts/recipes/postgres"

need cargo
need curl
need dirname
need find
need git
need make
need readlink
need shasum
need tar

case "$PLATFORM" in
  darwin-arm64 | darwin-amd64) ;;
  *) die "unsupported Postgres artifact platform: $PLATFORM" ;;
esac

if [ -z "$PV_COMMIT" ]; then
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

if [ -z "$BUILD_JOBS" ]; then
  BUILD_JOBS=$(sysctl -n hw.ncpu 2>/dev/null || printf '%s\n' 2)
fi

download_source() {
  source_archive=$1

  mkdir -p "$(dirname "$source_archive")"
  curl -L --fail --show-error --silent \
    --retry 3 --retry-delay 2 --retry-all-errors \
    --connect-timeout 20 --max-time 600 \
    "$PV_SOURCE_URL" -o "$source_archive"
  require_sha256 "$source_archive" "$PV_SOURCE_SHA256"
}

extract_source() {
  source_archive=$1
  source_extract_dir=$2

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
  [ "$source_entry_count" -eq 1 ] || die "Postgres source archive must contain exactly one top-level source directory"
  [ -d "$source_dir" ] || die "Postgres source archive top-level entry is not a directory"
  printf '%s\n' "$source_dir"
}

copy_install_tree() {
  install_dir=$1
  root_dir=$2

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
  rewrite_macho_install_names "$root_dir" "$install_dir"
  pv_recipe_ad_hoc_sign_macho_tree "$root_dir"
  for binary in postgres initdb pg_ctl psql; do
    [ -x "$root_dir/bin/$binary" ] || die "Postgres artifact missing bin/$binary"
    pv_recipe_validate_macho_binary "$root_dir/bin/$binary" "$PLATFORM" "$DEPLOYMENT_TARGET"
  done
  if [ -d "$root_dir/lib" ]; then
    find "$root_dir/lib" -type f \( -name '*.dylib' -o -name '*.so' \) | while IFS= read -r library; do
      pv_recipe_validate_macho_binary "$library" "$PLATFORM" "$DEPLOYMENT_TARGET"
    done
  fi
  cp "$recipe_dir/LICENSE" "$root_dir/LICENSE"
  cp "$recipe_dir/NOTICE" "$root_dir/NOTICE"
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
work_dir="$OUT_DIR/work/$artifact_basename"
source_archive="$OUT_DIR/sources/postgresql-$PV_UPSTREAM_VERSION.tar.gz"
source_extract_dir="$OUT_DIR/sources/postgresql-$PV_UPSTREAM_VERSION-source"
build_dir="$work_dir/build"
install_dir="$work_dir/install"
root_dir="$work_dir/$artifact_basename"
archive="$OUT_DIR/$artifact_basename.tar.gz"
record=$(artifact_record_path "$RECORD_DIR" postgres "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
object_key=$(artifact_object_key postgres "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")

rm -rf "$work_dir"
mkdir -p "$work_dir" "$build_dir" "$install_dir" "$OUT_DIR"
download_source "$source_archive"
source_dir=$(extract_source "$source_archive" "$source_extract_dir")

export CFLAGS="${CFLAGS:-} -mmacosx-version-min=$DEPLOYMENT_TARGET"
export LDFLAGS="${LDFLAGS:-} -mmacosx-version-min=$DEPLOYMENT_TARGET"
(
  cd "$source_dir"
  ./configure \
    --prefix="$install_dir" \
    --without-icu \
    --without-llvm \
    --without-lz4 \
    --without-readline \
    --without-zlib \
    --without-zstd
  make -j "$BUILD_JOBS"
  make install
)

copy_install_tree "$install_dir" "$root_dir"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"
write_record "$record" postgres "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/postgres/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION"

PV_UPSTREAM_VERSION="$PV_UPSTREAM_VERSION" \
  cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$recipe_dir/smoke.sh"
printf '%s\n' "$archive"
