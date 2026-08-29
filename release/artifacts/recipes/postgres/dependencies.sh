#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/postgres/openssl.env"

command=${1:-}
if [ -n "$command" ]; then
  shift
fi

PLATFORM=${PV_RECIPE_PLATFORM:-darwin-arm64}
BUILD_JOBS=${PV_BUILD_JOBS:-2}
SOURCE_CACHE_DIR=${PV_POSTGRES_SOURCE_CACHE_DIR:-"$ROOT/release/artifacts/out/source-cache"}
WORK_DIR=${PV_POSTGRES_DEPENDENCY_WORK_DIR:-"$ROOT/release/artifacts/out/postgres-dependencies/$PLATFORM"}

need awk
need basename
need cmp
need codesign
need curl
need dirname
need file
need find
need install_name_tool
need lipo
need make
need mktemp
need mv
need otool
need perl
need sed
need shasum
need tar
need uname
need xcrun
need xcodebuild

case "$PLATFORM" in
  darwin-arm64 | darwin-amd64) ;;
  *) die "unsupported Postgres dependency platform: $PLATFORM" ;;
esac

case "$BUILD_JOBS" in
  '' | *[!0-9]*) die "Postgres dependency build jobs must be an integer from 1 through 4" ;;
esac
[ "$BUILD_JOBS" -ge 1 ] && [ "$BUILD_JOBS" -le 4 ] || die "Postgres dependency build jobs must be an integer from 1 through 4"

expected_arch=$(expected_arch_for_platform "$PLATFORM")
native_arch=$(uname -m)
[ "$native_arch" = "$expected_arch" ] || die "Postgres dependency runner architecture $native_arch does not match $PLATFORM"

sdk_version=$(xcrun --sdk macosx --show-sdk-version) || die "failed to identify the macOS SDK version"
sdk_path=$(xcrun --sdk macosx --show-sdk-path) || die "failed to identify the macOS SDK path"
sdk_root=${SDKROOT:-$sdk_path}

compiler=${CC:-}
if [ -z "$compiler" ]; then
  compiler=$(xcrun --find clang) || die "failed to locate the macOS C compiler"
fi
case "$compiler" in
  */*) compiler_path=$compiler ;;
  *) compiler_path=$(command -v "$compiler") || die "failed to locate C compiler $compiler" ;;
esac
[ -x "$compiler_path" ] || die "C compiler is not executable: $compiler_path"
compiler_version_output=$("$compiler_path" --version) || die "failed to identify C compiler version: $compiler_path"
compiler_version=$(printf '%s\n' "$compiler_version_output" | sed -n '1p')
[ -n "$compiler_version" ] || die "C compiler version is empty: $compiler_path"
xcode_version_output=$(xcodebuild -version) || die "failed to identify the Xcode toolchain version"
xcode_version=$(printf '%s\n' "$xcode_version_output" | sed -n '1p')
xcode_build_version=$(printf '%s\n' "$xcode_version_output" | sed -n '2p')
[ -n "$xcode_version" ] && [ -n "$xcode_build_version" ] || die "Xcode toolchain version is incomplete"

openssl_configure_target=$(pv_recipe_openssl_configure_target_for_platform "$PLATFORM")
source_archive_name="openssl-$PV_POSTGRES_OPENSSL_VERSION.tar.gz"
contract_file="$WORK_DIR/openssl-contract.txt"
common_script_sha256=$(sha256_file "$ROOT/release/artifacts/recipes/common.sh")
dependencies_script_sha256=$(sha256_file "$ROOT/release/artifacts/recipes/postgres/dependencies.sh")

write_contract() {
  destination=$1
  carriage_return=$(printf '\r')

  for contract_value in \
    "$PV_POSTGRES_OPENSSL_VERSION" \
    "$PV_POSTGRES_OPENSSL_SOURCE_URL" \
    "$PV_POSTGRES_OPENSSL_SOURCE_SHA256" \
    "$PLATFORM" \
    "$native_arch" \
    "$sdk_version" \
    "$sdk_path" \
    "$sdk_root" \
    "$PV_POSTGRES_DEPLOYMENT_TARGET" \
    "$compiler_path" \
    "$compiler_version" \
    "$xcode_version" \
    "$xcode_build_version" \
    "$openssl_configure_target" \
    "$PV_POSTGRES_OPENSSL_DIR" \
    "$common_script_sha256" \
    "$dependencies_script_sha256" \
    "${CFLAGS:-}" \
    "${CPPFLAGS:-}" \
    "${LDFLAGS:-}" \
    "$BUILD_JOBS"; do
    case "$contract_value" in
      *'
'*) die "Postgres dependency build contract values must be single-line" ;;
    esac
    case "$contract_value" in
      *"$carriage_return"*) die "Postgres dependency build contract values must be single-line" ;;
    esac
  done

  mkdir -p "$(dirname "$destination")"
  {
    printf '%s\n' 'format=1'
    printf '%s\n' "source_version=$PV_POSTGRES_OPENSSL_VERSION"
    printf '%s\n' "source_url=$PV_POSTGRES_OPENSSL_SOURCE_URL"
    printf '%s\n' "source_sha256=$PV_POSTGRES_OPENSSL_SOURCE_SHA256"
    printf '%s\n' "platform=$PLATFORM"
    printf '%s\n' "architecture=$native_arch"
    printf '%s\n' "sdk_version=$sdk_version"
    printf '%s\n' "sdk_path=$sdk_path"
    printf '%s\n' "sdk_root=$sdk_root"
    printf '%s\n' "deployment_target=$PV_POSTGRES_DEPLOYMENT_TARGET"
    printf '%s\n' "compiler_path=$compiler_path"
    printf '%s\n' "compiler_version=$compiler_version"
    printf '%s\n' "xcode_version=$xcode_version"
    printf '%s\n' "xcode_build_version=$xcode_build_version"
    printf '%s\n' "configure_target=$openssl_configure_target"
    printf '%s\n' 'configure_flags=no-tests'
    printf '%s\n' "openssl_dir=$PV_POSTGRES_OPENSSL_DIR"
    printf '%s\n' "common_script_sha256=$common_script_sha256"
    printf '%s\n' "dependencies_script_sha256=$dependencies_script_sha256"
    printf '%s\n' "cflags=${CFLAGS:-}"
    printf '%s\n' "cppflags=${CPPFLAGS:-}"
    printf '%s\n' "ldflags=${LDFLAGS:-}"
    printf '%s\n' "build_jobs=$BUILD_JOBS"
  } >"$destination"
}

write_contract "$contract_file"
contract_sha256=$(sha256_file "$contract_file")
cache_key="postgres-openssl-v1-$PLATFORM-$contract_sha256"

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

validate_contract() (
  openssl_prefix=$1
  bundled_contract="$openssl_prefix/.pv-postgres-openssl-contract"
  expected_contract=${PV_POSTGRES_OPENSSL_CONTRACT_FILE:-$contract_file}

  [ -f "$bundled_contract" ] || die "reusable OpenSSL bundle is missing its build contract"
  [ -f "$expected_contract" ] || die "expected reusable OpenSSL build contract not found: $expected_contract"
  if cmp -s "$expected_contract" "$bundled_contract"; then
    return
  else
    compare_status=$?
  fi
  [ "$compare_status" -eq 1 ] || die "failed to compare reusable OpenSSL build contracts"
  die "reusable OpenSSL bundle does not match the expected build contract"
)

validate_prefix() (
  openssl_prefix=$1

  validate_contract "$openssl_prefix"
  pv_recipe_validate_openssl_prefix \
    "$openssl_prefix" \
    "$PLATFORM" \
    "$PV_POSTGRES_DEPLOYMENT_TARGET"
)

relocate_prefix() (
  openssl_prefix=$1
  build_prefix_file="$openssl_prefix/.pv-postgres-openssl-build-prefix"
  [ -f "$build_prefix_file" ] || die "reusable OpenSSL bundle is missing its build prefix"
  build_prefix=$(sed -n '1p' "$build_prefix_file")
  [ -n "$build_prefix" ] || die "reusable OpenSSL bundle build prefix is empty"

  for library in "$openssl_prefix/lib/libssl.3.dylib" "$openssl_prefix/lib/libcrypto.3.dylib"; do
    [ -f "$library" ] || die "reusable OpenSSL bundle is missing ${library##*/}"
    install_name_tool -id "$openssl_prefix/lib/${library##*/}" "$library"
    linked_libraries=$(otool -L "$library") || die "failed to inspect reusable OpenSSL library $library"
    linked_library_paths=$(awk 'NR > 1 && NF > 0 { print $1 }' <<EOF
$linked_libraries
EOF
    ) || die "failed to parse reusable OpenSSL linked libraries for $library"
    if [ -n "$linked_library_paths" ]; then
      while IFS= read -r linked_library; do
        case "$linked_library" in
          "$build_prefix"/lib/* | @rpath/libcrypto.3.dylib | @loader_path/libcrypto.3.dylib)
            install_name_tool -change "$linked_library" "$openssl_prefix/lib/${linked_library##*/}" "$library"
            ;;
        esac
      done <<EOF
$linked_library_paths
EOF
    fi
    sign_macho_binary "$library"
  done
  printf '%s\n' "$openssl_prefix" >"$build_prefix_file"
)

build_bundle() {
  bundle_archive=$1
  openssl_prefix=$2
  source_archive="$SOURCE_CACHE_DIR/$source_archive_name"
  source_extract_dir="$WORK_DIR/openssl-source"

  export CC="$compiler_path"
  export SDKROOT="$sdk_root"
  pv_recipe_build_openssl_dependency \
    "$openssl_prefix" \
    "$source_archive" \
    "$source_extract_dir" \
    "$PV_POSTGRES_OPENSSL_SOURCE_URL" \
    "$PV_POSTGRES_OPENSSL_SOURCE_SHA256" \
    "$PLATFORM" \
    "$PV_POSTGRES_DEPLOYMENT_TARGET" \
    "$BUILD_JOBS" \
    "$PV_POSTGRES_OPENSSL_DIR"
  cp "$contract_file" "$openssl_prefix/.pv-postgres-openssl-contract"
  printf '%s\n' "$openssl_prefix" >"$openssl_prefix/.pv-postgres-openssl-build-prefix"
  validate_prefix "$openssl_prefix"

  mkdir -p "$(dirname "$bundle_archive")"
  bundle_archive_tmp="$bundle_archive.tmp.$$"
  rm -f "$bundle_archive_tmp"
  COPYFILE_DISABLE=1 tar -czf "$bundle_archive_tmp" -C "$(dirname "$openssl_prefix")" "$(basename "$openssl_prefix")"
  mv "$bundle_archive_tmp" "$bundle_archive"
}

use_bundle() (
  bundle_archive=$1
  openssl_prefix=$2

  [ -f "$bundle_archive" ] || die "reusable OpenSSL bundle not found: $bundle_archive"
  extract_dir=$(mktemp -d "$WORK_DIR/openssl-bundle.XXXXXX")
  trap 'rm -rf "$extract_dir"' 0
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
  tar -xzf "$bundle_archive" -C "$extract_dir"
  extracted_entry_count=0
  extracted_prefix=
  for extracted_entry in "$extract_dir"/* "$extract_dir"/.[!.]* "$extract_dir"/..?*; do
    [ -d "$extracted_entry" ] || [ -f "$extracted_entry" ] || [ -L "$extracted_entry" ] || continue
    extracted_entry_count=$((extracted_entry_count + 1))
    extracted_prefix=$extracted_entry
  done
  [ "$extracted_entry_count" -eq 1 ] || die "reusable OpenSSL bundle must contain exactly one top-level prefix"
  [ -d "$extracted_prefix" ] || die "reusable OpenSSL bundle top-level entry is not a directory"
  validate_contract "$extracted_prefix"

  rm -rf "$openssl_prefix"
  mkdir -p "$(dirname "$openssl_prefix")"
  mv "$extracted_prefix" "$openssl_prefix"
  relocate_prefix "$openssl_prefix"
  validate_prefix "$openssl_prefix"
)

case "$command" in
  describe)
    printf '%s\n' "cache_key=$cache_key"
    printf '%s\n' "source_archive_name=$source_archive_name"
    printf '%s\n' "source_sha256=$PV_POSTGRES_OPENSSL_SOURCE_SHA256"
    ;;
  build)
    [ "$#" -eq 2 ] || die "usage: dependencies.sh build <bundle-archive> <openssl-prefix>"
    build_bundle "$1" "$2"
    ;;
  use)
    [ "$#" -eq 2 ] || die "usage: dependencies.sh use <bundle-archive> <openssl-prefix>"
    use_bundle "$1" "$2"
    ;;
  *) die "usage: dependencies.sh <describe|build|use>" ;;
esac
