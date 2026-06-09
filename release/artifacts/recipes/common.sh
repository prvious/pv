#!/bin/sh
set -eu

die() {
  printf '%s\n' "error: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

require_sha256() {
  file=$1
  expected=$2
  actual=$(sha256_file "$file")
  [ "$actual" = "$expected" ] || die "$file checksum mismatch: expected $expected, got $actual"
}

expected_arch_for_platform() {
  case "$1" in
    darwin-arm64) printf '%s\n' arm64 ;;
    darwin-amd64) printf '%s\n' x86_64 ;;
    *) die "unsupported native artifact platform: $1" ;;
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
  platform=$2
  deployment_target=$3
  expected_arch=$(expected_arch_for_platform "$platform")
  binary_archs=$(lipo -archs "$binary")
  [ "$binary_archs" = "$expected_arch" ] || die "$binary Mach-O architecture $binary_archs does not match expected $expected_arch for $platform"

  binary_minos=$(macho_minimum_os "$binary")
  [ -n "$binary_minos" ] || die "$binary Mach-O minimum macOS version not found"
  version_lte "$binary_minos" "$deployment_target" || die "$binary Mach-O minimum macOS $binary_minos is newer than deployment target $deployment_target"

  validate_macho_runtime_paths "$binary"
}

sign_macho_binary() {
  binary=$1
  codesign --force --sign - "$binary"
  codesign --verify "$binary"
}

artifact_basename() {
  resource=$1
  artifact_version=$2
  platform=$3
  printf '%s\n' "$resource-$artifact_version-$platform"
}

artifact_object_key() {
  resource=$1
  track=$2
  artifact_version=$3
  platform=$4
  basename=$(artifact_basename "$resource" "$artifact_version" "$platform")
  printf '%s\n' "resources/$resource/$track/$artifact_version/$platform/$basename.tar.gz"
}

artifact_record_path() {
  record_dir=$1
  resource=$2
  track=$3
  artifact_version=$4
  platform=$5
  basename=$(artifact_basename "$resource" "$artifact_version" "$platform")
  printf '%s\n' "$record_dir/$resource/$track/$artifact_version/$platform/$basename.json"
}

write_record() {
  record_path=$1
  resource=$2
  track=$3
  upstream_version=$4
  pv_build_revision=$5
  platform=$6
  object_key=$7
  archive=$8
  source_url=$9
  source_sha256=${10}
  recipe=${11}
  pv_commit=${12}
  build_run_id=${13}
  minimum_pv_version=${14}
  shift 14

  published_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  cargo run -p pv-release -- write-release-record \
    --record "$record_path" \
    --archive "$archive" \
    --resource "$resource" \
    --track "$track" \
    --upstream-version "$upstream_version" \
    --pv-build-revision "$pv_build_revision" \
    --platform "$platform" \
    --object-key "$object_key" \
    --source-url "$source_url" \
    --source-sha256 "$source_sha256" \
    --recipe "$recipe" \
    --pv-commit "$pv_commit" \
    --build-run-id "$build_run_id" \
    --minimum-pv-version "$minimum_pv_version" \
    --published-at "$published_at" \
    "$@"
}
