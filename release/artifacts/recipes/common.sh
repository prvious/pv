#!/bin/sh
set -eu

die() {
  printf '%s\n' "error: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

sha256_file() (
  checksum_output=$(
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "$1"
    else
      shasum -a 256 "$1"
    fi
  ) || die "failed to calculate SHA-256 for $1"
  checksum=${checksum_output%% *}
  [ -n "$checksum" ] || die "SHA-256 output was empty for $1"
  printf '%s\n' "$checksum"
)

cpu_count() {
  if command -v nproc >/dev/null 2>&1; then
    nproc
  else
    sysctl -n hw.ncpu
  fi
}

require_sha256() {
  file=$1
  expected=$2
  actual=$(sha256_file "$file")
  [ "$actual" = "$expected" ] || die "$file checksum mismatch: expected $expected, got $actual"
}

pv_recipe_openssl_configure_target_for_platform() {
  case "$1" in
    darwin-arm64) printf '%s\n' darwin64-arm64-cc ;;
    darwin-amd64) printf '%s\n' darwin64-x86_64-cc ;;
    linux-amd64) printf '%s\n' linux-x86_64 ;;
    linux-arm64) printf '%s\n' linux-aarch64 ;;
    *) die "unsupported OpenSSL artifact platform: $1" ;;
  esac
}

pv_recipe_openssl_library_names_for_platform() {
  case "$1" in
    darwin-arm64 | darwin-amd64) printf '%s\n' 'libssl.3.dylib libcrypto.3.dylib' ;;
    linux-amd64 | linux-arm64) printf '%s\n' 'libssl.so.3 libcrypto.so.3' ;;
    *) die "unsupported OpenSSL artifact platform: $1" ;;
  esac
}

pv_recipe_validate_openssl_prefix() (
  openssl_prefix=$1
  platform=$2
  deployment_target=$3

  [ -f "$openssl_prefix/include/openssl/ssl.h" ] || die "OpenSSL headers not found under $openssl_prefix"
  libraries=$(pv_recipe_openssl_library_names_for_platform "$platform")
  for library_name in $libraries; do
    [ -f "$openssl_prefix/lib/$library_name" ] || die "OpenSSL shared library $library_name not found under $openssl_prefix"
  done
  [ -f "$openssl_prefix/LICENSE.txt" ] || die "OpenSSL license not found under $openssl_prefix"

  case "$platform" in
    linux-amd64 | linux-arm64)
      [ ! -L "$openssl_prefix/lib" ] || die "OpenSSL library directory is a symlink: $openssl_prefix/lib"
      need file
      for library_name in $libraries; do
        library="$openssl_prefix/lib/$library_name"
        [ ! -L "$library" ] || die "OpenSSL SONAME library is a symlink: $library"
        description=$(file -E -b -- "$library") || die "failed to inspect OpenSSL shared library $library"
        case "$description" in
          ELF\ *) ;;
          *) die "OpenSSL shared library is not ELF: $library" ;;
        esac
      done
      pv_recipe_validate_elf_tree "$openssl_prefix/lib" "$platform"
      return
      ;;
  esac

  for library_name in $libraries; do
    library="$openssl_prefix/lib/$library_name"
    pv_recipe_validate_macho_architecture_and_target "$library" "$platform" "$deployment_target"
    linked_libraries=$(otool -L "$library") || die "failed to inspect OpenSSL linked libraries for $library"
    linked_library_paths=$(awk 'NR > 1 && NF > 0 { print $1 }' <<EOF
$linked_libraries
EOF
    ) || die "failed to parse OpenSSL linked libraries for $library"
    if [ -n "$linked_library_paths" ]; then
      while IFS= read -r linked_library; do
        case "$linked_library" in
          /usr/lib/* | /System/Library/* | @rpath/* | @loader_path/* | @executable_path/* | "$openssl_prefix"/lib/*)
            ;;
          *) die "$library references dependency outside its reusable OpenSSL prefix: $linked_library" ;;
        esac
      done <<EOF
$linked_library_paths
EOF
    fi

    openssl_rpaths=$(macho_rpaths "$library") || die "failed to inspect OpenSSL rpaths for $library"
    if [ -n "$openssl_rpaths" ]; then
      while IFS= read -r openssl_rpath; do
        case "$openssl_rpath" in
          /usr/lib/* | /System/Library/* | @rpath/* | @loader_path/* | @executable_path/* | @loader_path | @executable_path | "$openssl_prefix"/lib | "$openssl_prefix"/lib/*)
            ;;
          *) die "$library Mach-O rpath references unmanaged runtime path $openssl_rpath" ;;
        esac
      done <<EOF
$openssl_rpaths
EOF
    fi
  done
)

pv_recipe_build_openssl_dependency() (
  openssl_prefix=$1
  openssl_source_archive=$2
  openssl_source_extract_dir=$3
  openssl_source_url=$4
  openssl_source_sha256=$5
  platform=$6
  deployment_target=$7
  build_jobs=$8
  openssl_dir=$9

  rm -rf "$openssl_prefix"
  download_source "$openssl_source_archive" "$openssl_source_url" "$openssl_source_sha256"
  openssl_source_dir=$(extract_source OpenSSL "$openssl_source_archive" "$openssl_source_extract_dir")
  openssl_configure_target=$(pv_recipe_openssl_configure_target_for_platform "$platform")

  (
    cd "$openssl_source_dir"
    set --
    case "$platform" in
      darwin-arm64 | darwin-amd64) export MACOSX_DEPLOYMENT_TARGET="$deployment_target" ;;
      linux-amd64 | linux-arm64)
        need patchelf
        set -- --libdir=lib
        ;;
    esac
    perl ./Configure "$openssl_configure_target" no-tests "$@" \
      --prefix="$openssl_prefix" \
      --openssldir="$openssl_dir"
    make -j "$build_jobs"
    make install_sw
  )

  cp "$openssl_source_dir/LICENSE.txt" "$openssl_prefix/LICENSE.txt"
  case "$platform" in
    linux-amd64 | linux-arm64)
      libraries=$(pv_recipe_openssl_library_names_for_platform "$platform")
      for library_name in $libraries; do
        patchelf --set-rpath "\$ORIGIN" "$openssl_prefix/lib/$library_name"
      done
      ;;
  esac
  pv_recipe_validate_openssl_prefix "$openssl_prefix" "$platform" "$deployment_target"
)

pv_recipe_library_loader_prefix() {
  root_dir=$1
  binary=$2
  loader_origin=$3

  case "$binary" in
    "$root_dir"/lib/*)
      relative=${binary#"$root_dir"/lib/}
      case "$relative" in
        */*)
          directory=${relative%/*}
          loader_prefix=$loader_origin
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
        *) printf '%s\n' "$loader_origin" ;;
      esac
      ;;
    *) printf '%s\n' "$loader_origin/../lib" ;;
  esac
}

pv_recipe_is_macho() (
  file_description=$(file -b "$1") || die "failed to inspect file type for $1"
  case "$file_description" in
    *Mach-O*) return 0 ;;
    *) return 1 ;;
  esac
)

rewrite_macho_install_names() {
  root_dir=$1
  shift

  need install_name_tool
  need file
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
      pv_recipe_is_macho "$macho" || continue
      loader_prefix=$(pv_recipe_library_loader_prefix "$root_dir" "$macho" @loader_path)
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
  need file
  need find
  need otool

  for macho_dir in "$root_dir/bin" "$root_dir/lib"; do
    [ -d "$macho_dir" ] || continue
    find "$macho_dir" -type f -exec sh -c '
      set -e
      for macho do
        file_description=$(file -b "$macho") || exit 1
        case "$file_description" in
          *Mach-O*) ;;
          *) continue ;;
        esac
        codesign --force --sign - "$macho" >/dev/null || exit 1
      done
    ' sh {} +
  done
}

expected_arch_for_platform() {
  case "$1" in
    darwin-arm64) printf '%s\n' arm64 ;;
    darwin-amd64 | linux-amd64) printf '%s\n' x86_64 ;;
    linux-arm64) printf '%s\n' aarch64 ;;
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
    @loader_path | @executable_path)
      [ "$metadata_kind" = "rpath" ] || die "$binary Mach-O $metadata_kind references unmanaged runtime path $runtime_path"
      ;;
    *) die "$binary Mach-O $metadata_kind references unmanaged runtime path $runtime_path" ;;
  esac
}

macho_rpaths() (
  load_commands=$(otool -l "$1") || die "failed to inspect Mach-O load commands for $1"
  printf '%s\n' "$load_commands" | awk '
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
)

pv_recipe_cleanup_macho_rpaths() {
  binary=$1

  macho_rpath_list=$(macho_rpaths "$binary") || die "failed to inspect Mach-O rpaths for $binary"
  [ -n "$macho_rpath_list" ] || return 0
  printf '%s\n' "$macho_rpath_list" | while IFS= read -r macho_rpath; do
    case "$macho_rpath" in
      /usr/lib/* | /System/Library/* | @rpath/* | @loader_path/* | @executable_path/* | @loader_path | @executable_path)
        ;;
      *) install_name_tool -delete_rpath "$macho_rpath" "$binary" || exit 1 ;;
    esac
  done
}

pv_recipe_cleanup_macho_rpaths_tree() {
  root_dir=$1

  need find
  need file
  need install_name_tool
  need otool

  for macho_dir in "$root_dir/bin" "$root_dir/lib"; do
    [ -d "$macho_dir" ] || continue
    find "$macho_dir" -type f | while IFS= read -r macho; do
      pv_recipe_is_macho "$macho" || continue
      pv_recipe_cleanup_macho_rpaths "$macho"
    done
  done
}

validate_macho_runtime_paths() {
  binary=$1
  linked_libraries=$(otool -L "$binary")
  printf '%s\n' "$linked_libraries" | awk 'NR > 1 && NF > 0 { print $1 }' | while IFS= read -r linked_library; do
    reject_unmanaged_macho_runtime_path "$binary" "linked library" "$linked_library"
  done

  macho_rpath_list=$(macho_rpaths "$binary") || die "failed to inspect Mach-O rpaths for $binary"
  [ -z "$macho_rpath_list" ] || printf '%s\n' "$macho_rpath_list" | while IFS= read -r macho_rpath; do
    reject_unmanaged_macho_runtime_path "$binary" "rpath" "$macho_rpath"
  done
}

validate_macho_binary() {
  binary=$1
  platform=$2
  deployment_target=$3

  pv_recipe_validate_macho_architecture_and_target "$binary" "$platform" "$deployment_target"
  validate_macho_runtime_paths "$binary"
}

pv_recipe_validate_macho_architecture_and_target() {
  binary=$1
  platform=$2
  deployment_target=$3
  expected_arch=$(expected_arch_for_platform "$platform")
  binary_archs=$(lipo -archs "$binary")
  [ "$binary_archs" = "$expected_arch" ] || die "$binary Mach-O architecture $binary_archs does not match expected $expected_arch for $platform"

  binary_minos=$(macho_minimum_os "$binary")
  [ -n "$binary_minos" ] || die "$binary Mach-O minimum macOS version not found"
  version_lte "$binary_minos" "$deployment_target" || die "$binary Mach-O minimum macOS $binary_minos is newer than deployment target $deployment_target"
}

sign_macho_binary() {
  binary=$1
  codesign --force --sign - "$binary"
  codesign --verify "$binary"
}

pv_recipe_validate_macho_binary() {
  validate_macho_binary "$@"
}

pv_recipe_validate_elf_tree() (
  root_dir=$1
  platform=$2
  export LC_ALL=C

  case "$platform" in
    linux-amd64 | linux-arm64) ;;
    *) die "unsupported ELF artifact platform: $platform" ;;
  esac
  expected_arch=$(expected_arch_for_platform "$platform")
  need find
  need file
  need readelf
  need awk
  [ -d "$root_dir" ] || die "ELF artifact directory not found: $root_dir"
  artifact_files=$(find "$root_dir" -type f) || die "failed to list ELF artifact files under $root_dir"
  [ -n "$artifact_files" ] || return 0

  while IFS= read -r binary; do
    description=$(file -E -b -- "$binary") || die "failed to inspect file type for $binary"
    case "$description" in
      ELF\ *) ;;
      *) continue ;;
    esac
    headers=$(readelf --wide --file-header -- "$binary") || die "failed to inspect ELF header for $binary"
    machine=$(awk '$1 == "Machine:" { sub(/^.*Machine:[[:space:]]*/, ""); print }' <<EOF
$headers
EOF
    ) || die "failed to parse ELF architecture for $binary"
    case "$machine" in
      "Advanced Micro Devices X86-64") actual_arch=x86_64 ;;
      AArch64) actual_arch=aarch64 ;;
      *) actual_arch=$machine ;;
    esac
    [ "$actual_arch" = "$expected_arch" ] || die "$binary ELF architecture $actual_arch does not match expected $expected_arch for $platform"

    versions=$(readelf --wide --version-info -- "$binary") || die "failed to inspect ELF symbol versions for $binary"
    glibc_versions=$(awk '{ for (field_index = 1; field_index <= NF; field_index++) if ($field_index ~ /^GLIBC_[0-9]+(\.[0-9]+)*$/ || $field_index == "GLIBC_ABI_DT_RELR") print $field_index }' <<EOF
$versions
EOF
    ) || die "failed to parse ELF symbol versions for $binary"
    if [ -n "$glibc_versions" ]; then
      while IFS= read -r glibc_version; do
        [ "$glibc_version" != GLIBC_ABI_DT_RELR ] || die "$binary ELF symbol version $glibc_version requires glibc 2.36"
        version_lte "${glibc_version#GLIBC_}" 2.35 || die "$binary ELF symbol version $glibc_version is newer than GLIBC_2.35"
      done <<EOF
$glibc_versions
EOF
    fi

    dynamic=$(readelf --wide --dynamic -- "$binary") || die "failed to inspect ELF dynamic section for $binary"
    dynamic_entries=$(awk '
      $2 == "(NEEDED)" || $2 == "(SONAME)" || $2 == "(RPATH)" || $2 == "(RUNPATH)" {
        value = substr($0, index($0, "[") + 1)
        sub(/\]$/, "", value)
        print $2 ":" value
      }
    ' <<EOF
$dynamic
EOF
    ) || die "failed to parse ELF dynamic section for $binary"
    [ -n "$dynamic_entries" ] || continue
    while IFS= read -r dynamic_entry; do
      value=${dynamic_entry#*:}
      case "$dynamic_entry" in
        '(NEEDED):'*)
          case "$value" in
            libc.so.6 | libm.so.6 | libpthread.so.0 | libdl.so.2 | librt.so.1 | libresolv.so.2 | ld-linux-x86-64.so.2 | ld-linux-aarch64.so.1 | libgcc_s.so.1 | libstdc++.so.6) continue ;;
          esac
          bundled=false
          while IFS= read -r artifact_file; do
            if [ "${artifact_file##*/}" = "$value" ]; then
              bundled=true
              break
            fi
          done <<EOF
$artifact_files
EOF
          [ "$bundled" = true ] || die "$binary ELF dependency is not a system library or bundled file: $value"
          ;;
        '(SONAME):'*)
          [ "${binary##*/}" = "$value" ] || die "$binary ELF filename does not match SONAME $value"
          ;;
        '(RPATH):'* | '(RUNPATH):'*)
          while :; do
            search_path=${value%%:*}
            case "$search_path" in
              "\$ORIGIN" | "\$ORIGIN/"* | "\${ORIGIN}" | "\${ORIGIN}/"*) ;;
              *) die "$binary ELF library search path is not ORIGIN-relative: $search_path" ;;
            esac
            case "$value" in
              *:*) value=${value#*:} ;;
              *) break ;;
            esac
          done
          ;;
      esac
    done <<EOF
$dynamic_entries
EOF
  done <<EOF
$artifact_files
EOF
)

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
