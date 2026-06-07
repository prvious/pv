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

file_size() {
  file=$1
  if size=$(stat -c '%s' "$file" 2>/dev/null); then
    printf '%s\n' "$size"
    return 0
  fi
  if size=$(stat -f '%z' "$file" 2>/dev/null); then
    printf '%s\n' "$size"
    return 0
  fi
  wc -c <"$file" | awk '{ print $1 }'
}

require_sha256() {
  file=$1
  expected=$2
  actual=$(sha256_file "$file")
  [ "$actual" = "$expected" ] || die "$file checksum mismatch: expected $expected, got $actual"
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
  source_inputs_json=${15:-}

  artifact_version="${upstream_version}-${pv_build_revision}"
  sha256=$(sha256_file "$archive")
  size=$(file_size "$archive")
  published_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  source_inputs_record_json=
  if [ -n "$source_inputs_json" ]; then
    source_inputs_record_json=$(cat <<JSON
    "source_inputs": $source_inputs_json,
JSON
)
  fi
  mkdir -p "$(dirname "$record_path")"
  cat >"$record_path" <<JSON
{
  "resource": "$resource",
  "track": "$track",
  "upstream_version": "$upstream_version",
  "pv_build_revision": "$pv_build_revision",
  "artifact_version": "$artifact_version",
  "platform": "$platform",
  "object_key": "$object_key",
  "sha256": "$sha256",
  "size": $size,
  "published_at": "$published_at",
  "minimum_pv_version": "$minimum_pv_version",
  "license_files": ["LICENSE"],
  "notice_files": ["NOTICE"],
  "provenance": {
    "source_url": "$source_url",
    "source_sha256": "$source_sha256",
$source_inputs_record_json    "recipe": "$recipe",
    "pv_commit": "$pv_commit",
    "build_run_id": "$build_run_id"
  }
}
JSON
}
