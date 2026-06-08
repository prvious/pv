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
