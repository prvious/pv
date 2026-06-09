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

pv_recipe_macho_loader_prefix() {
  root_dir=$1
  macho=$2

  case "$macho" in
    "$root_dir"/lib/*)
      relative=${macho#"$root_dir"/lib/}
      case "$relative" in
        */*)
          directory=${relative%/*}
          loader_prefix="@loader_path"
          # Nested lib modules need enough ".." hops to resolve back to the artifact lib root.
          while :; do
            loader_prefix="$loader_prefix/.."
            case "$directory" in
              */*) directory=${directory#*/} ;;
              *) break ;;
            esac
          done
          printf '%s\n' "$loader_prefix"
          ;;
        *) printf '%s\n' "@loader_path" ;;
      esac
      ;;
    *) printf '%s\n' "@loader_path/../lib" ;;
  esac
}

rewrite_macho_install_names() {
  root_dir=$1
  shift

  need install_name_tool
  need find
  need otool
  [ "$#" -gt 0 ] || die "missing install root for Mach-O install-name rewrite"

  if [ -d "$root_dir/lib" ]; then
    find "$root_dir/lib" -type f -name '*.dylib' -exec sh -c '
      set -e
      root_count=$1
      shift
      root_index=0
      install_roots=
      while [ "$root_index" -lt "$root_count" ]; do
        install_roots="$install_roots
$1"
        root_index=$((root_index + 1))
        shift
      done
      for library do
        install_name=$(
          otool -D "$library" 2>/dev/null | {
            IFS= read -r _ || true
            IFS= read -r line || true
            printf "%s\n" "$line"
          }
        )
        printf "%s\n" "$install_roots" | while IFS= read -r install_root; do
          [ -n "$install_root" ] || continue
          case "$install_name" in
            "$install_root"/lib/*)
              install_name_tool -id "@loader_path/${install_name##*/}" "$library" || exit 1
              ;;
          esac
        done
      done
    ' sh "$#" "$@" {} +
  fi

  for macho_dir in "$root_dir/bin" "$root_dir/lib"; do
    [ -d "$macho_dir" ] || continue
    find "$macho_dir" -type f | while IFS= read -r macho; do
      otool -L "$macho" >/dev/null 2>&1 || continue
      loader_prefix=$(pv_recipe_macho_loader_prefix "$root_dir" "$macho")
      otool -L "$macho" | while read -r linked _; do
        for install_root in "$@"; do
          case "$linked" in
            "$install_root"/lib/*)
              install_name_tool -change "$linked" "$loader_prefix/${linked##*/}" "$macho" || exit 1
              ;;
          esac
        done
      done
    done
  done
}

pv_recipe_ad_hoc_sign_macho_tree() {
  root_dir=$1

  need codesign
  need find
  need otool

  for macho_dir in "$root_dir/bin" "$root_dir/lib"; do
    [ -d "$macho_dir" ] || continue
    find "$macho_dir" -type f -exec sh -c '
      set -e
      for macho do
        otool -L "$macho" >/dev/null 2>&1 || continue
        codesign --force --sign - "$macho" >/dev/null || exit 1
      done
    ' sh {} +
  done
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

pv_recipe_validate_macho_binary() {
  validate_macho_binary "$@"
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
