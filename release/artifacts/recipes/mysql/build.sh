#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

TRACK=${PV_RECIPE_TRACK:-8.4}
PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
OUT_DIR=${PV_ARTIFACT_OUT_DIR:-"$ROOT/release/artifacts/out"}
RECORD_DIR=${PV_ARTIFACT_RECORD_DIR:-"$ROOT/release/artifacts/records"}
PV_COMMIT=${PV_COMMIT:-}
BUILD_RUN_ID=${PV_BUILD_RUN_ID:-local-mysql}
BUILD_JOBS=${PV_BUILD_JOBS:-}
BISON_EXECUTABLE=${PV_MYSQL_BISON_EXECUTABLE:-}
OPENSSL_PREFIX=${PV_MYSQL_OPENSSL_PREFIX:-}
OPENSSL_VERSION=${PV_MYSQL_OPENSSL_VERSION:-3.5.7}
OPENSSL_SOURCE_URL=${PV_MYSQL_OPENSSL_SOURCE_URL:-"https://github.com/openssl/openssl/releases/download/openssl-$OPENSSL_VERSION/openssl-$OPENSSL_VERSION.tar.gz"}
OPENSSL_SOURCE_SHA256=${PV_MYSQL_OPENSSL_SOURCE_SHA256:-a8c0d28a529ca480f9f36cf5792e2cd21984552a3c8e4aa11a24aa31aeac98e8}
DEPLOYMENT_TARGET=13.0
recipe_dir="$ROOT/release/artifacts/recipes/mysql"

need brew
need cargo
need cmake
need curl
need dirname
need find
need git
need make
need perl
need readlink
need shasum
need tar

case "$PLATFORM" in
  darwin-arm64 | darwin-amd64) ;;
  *) die "unsupported MySQL artifact platform: $PLATFORM" ;;
esac

if [ -z "$PV_COMMIT" ]; then
  PV_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
fi

if [ -z "$BUILD_JOBS" ]; then
  BUILD_JOBS=$(sysctl -n hw.ncpu 2>/dev/null || printf '%s\n' 2)
fi

if [ -z "$BISON_EXECUTABLE" ]; then
  BISON_PREFIX=$(brew --prefix bison 2>/dev/null || true)
  if [ -z "$BISON_PREFIX" ] || [ ! -x "$BISON_PREFIX/bin/bison" ]; then
    brew install bison
    BISON_PREFIX=$(brew --prefix bison)
  fi
  BISON_EXECUTABLE="$BISON_PREFIX/bin/bison"
fi
[ -x "$BISON_EXECUTABLE" ] || die "Bison executable not found: $BISON_EXECUTABLE"

download_source() {
  source_archive=$1
  source_url=$2
  source_sha256=$3

  mkdir -p "$(dirname "$source_archive")"
  curl -L --fail --show-error --silent \
    --retry 3 --retry-delay 2 --retry-all-errors \
    --connect-timeout 20 --max-time 1200 \
    "$source_url" -o "$source_archive"
  require_sha256 "$source_archive" "$source_sha256"
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

openssl_configure_target_for_platform() {
  case "$1" in
    darwin-arm64) printf '%s\n' darwin64-arm64-cc ;;
    darwin-amd64) printf '%s\n' darwin64-x86_64-cc ;;
    *) die "unsupported MySQL artifact platform: $1" ;;
  esac
}

build_openssl_dependency() {
  openssl_prefix=$1
  openssl_source_archive=$2
  openssl_source_extract_dir=$3

  rm -rf "$openssl_prefix"
  download_source "$openssl_source_archive" "$OPENSSL_SOURCE_URL" "$OPENSSL_SOURCE_SHA256"
  openssl_source_dir=$(extract_source OpenSSL "$openssl_source_archive" "$openssl_source_extract_dir")
  openssl_configure_target=$(openssl_configure_target_for_platform "$PLATFORM")

  (
    cd "$openssl_source_dir"
    MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
      perl ./Configure "$openssl_configure_target" no-shared no-tests \
      --prefix="$openssl_prefix" \
      --openssldir="$openssl_prefix/ssl"
    MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" make -j "$BUILD_JOBS"
    MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" make install_sw
  )

  [ -f "$openssl_prefix/include/openssl/ssl.h" ] || die "OpenSSL headers not found under $openssl_prefix"
  [ -f "$openssl_prefix/lib/libssl.a" ] || die "OpenSSL static SSL library not found under $openssl_prefix"
  [ -f "$openssl_prefix/lib/libcrypto.a" ] || die "OpenSSL static crypto library not found under $openssl_prefix"
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
  rewrite_macho_install_names "$root_dir" "$install_dir" "$OPENSSL_PREFIX"
  pv_recipe_ad_hoc_sign_macho_tree "$root_dir"
  for binary in mysqld mysql mysqladmin; do
    [ -x "$root_dir/bin/$binary" ] || die "MySQL artifact missing bin/$binary"
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

env_file="$OUT_DIR/work/mysql-$TRACK-$PLATFORM.env"
mkdir -p "$(dirname "$env_file")"
cargo run -p pv-release -- print-recipe-env \
  --mysql "$recipe_dir/recipe.toml" \
  --resource mysql \
  --track "$TRACK" \
  --platform "$PLATFORM" >"$env_file"
# shellcheck source=/dev/null
. "$env_file"
export PV_UPSTREAM_VERSION

artifact_basename=$(artifact_basename mysql "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
work_dir="$OUT_DIR/work/$artifact_basename"
source_archive="$OUT_DIR/sources/mysql-$PV_UPSTREAM_VERSION.tar.gz"
source_extract_dir="$OUT_DIR/sources/mysql-$PV_UPSTREAM_VERSION-source"
build_dir="$work_dir/build"
install_dir="$work_dir/install"
root_dir="$work_dir/$artifact_basename"
openssl_source_archive="$OUT_DIR/sources/openssl-$OPENSSL_VERSION.tar.gz"
openssl_source_extract_dir="$OUT_DIR/sources/openssl-$OPENSSL_VERSION-source"
archive="$OUT_DIR/$artifact_basename.tar.gz"
record=$(artifact_record_path "$RECORD_DIR" mysql "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")
object_key=$(artifact_object_key mysql "$PV_TRACK" "$PV_ARTIFACT_VERSION" "$PV_PLATFORM")

rm -rf "$work_dir"
mkdir -p "$work_dir" "$OUT_DIR"
if [ -z "$OPENSSL_PREFIX" ]; then
  OPENSSL_PREFIX="$work_dir/openssl-$OPENSSL_VERSION"
  build_openssl_dependency "$OPENSSL_PREFIX" "$openssl_source_archive" "$openssl_source_extract_dir"
else
  [ -f "$OPENSSL_PREFIX/include/openssl/ssl.h" ] || die "OpenSSL headers not found under $OPENSSL_PREFIX"
fi
download_source "$source_archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256"
source_dir=$(extract_source MySQL "$source_archive" "$source_extract_dir")

export MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET"
cmake -S "$source_dir" -B "$build_dir" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$install_dir" \
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
  -DBISON_EXECUTABLE="$BISON_EXECUTABLE" \
  -DINSTALL_LAYOUT=STANDALONE \
  -DMYSQL_DATADIR="$install_dir/data" \
  -DWITH_EDITLINE=bundled \
  -DWITH_ICU=bundled \
  -DWITH_LZ4=bundled \
  -DWITH_NDB=OFF \
  -DOPENSSL_USE_STATIC_LIBS=TRUE \
  -DWITH_PROTOBUF=bundled \
  -DWITH_ROUTER=OFF \
  -DWITH_SSL="$OPENSSL_PREFIX" \
  -DWITH_UNIT_TESTS=OFF \
  -DWITH_ZLIB=bundled \
  -DWITH_ZSTD=bundled
cmake --build "$build_dir" --parallel "$BUILD_JOBS"
cmake --install "$build_dir"

copy_install_tree "$install_dir" "$root_dir"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work_dir" "$artifact_basename"
write_record "$record" mysql "$PV_TRACK" "$PV_UPSTREAM_VERSION" "$PV_PV_BUILD_REVISION" "$PV_PLATFORM" "$object_key" "$archive" "$PV_SOURCE_URL" "$PV_SOURCE_SHA256" release/artifacts/recipes/mysql/build.sh "$PV_COMMIT" "$BUILD_RUN_ID" "$PV_MINIMUM_PV_VERSION"

PV_UPSTREAM_VERSION="$PV_UPSTREAM_VERSION" \
  cargo run -p pv-release -- validate-archive --archive "$archive" --record "$record" --smoke-hook "$recipe_dir/smoke.sh"
printf '%s\n' "$archive"
