#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

: "${PV_RECIPE_RESOURCE:?}" "${PV_RECIPE_TRACK:?}" "${PV_RECIPE_PLATFORM:?}"
: "${PV_ARTIFACT_OUT_DIR:?}" "${PV_ARTIFACT_RECORD_DIR:?}"
case "$PV_RECIPE_RESOURCE" in
  php | composer | redis | mysql | postgres | mailpit | rustfs | caddy) ;;
  *) printf 'unsupported resource: %s\n' "$PV_RECIPE_RESOURCE" >&2; exit 1 ;;
esac
case "$PV_RECIPE_PLATFORM:$(uname -m)" in
  linux-amd64:x86_64 | linux-arm64:aarch64) ;;
  *) printf 'Linux recipe requires its native CPU: %s\n' "$PV_RECIPE_PLATFORM" >&2; exit 1 ;;
esac

mkdir -p "$HOME" "$PV_ARTIFACT_OUT_DIR" "$PV_ARTIFACT_RECORD_DIR"
if [ "$PV_RECIPE_RESOURCE" = php ]; then
  tools="$PV_ARTIFACT_OUT_DIR/tools"
  mkdir -p "$tools"
  curl -L --fail --show-error --silent --retry 3 --retry-delay 2 --retry-all-errors \
    --connect-timeout 20 --max-time 600 \
    "https://dl.static-php.dev/v3/spc-bin/nightly/spc-linux-$(uname -m)" -o "$tools/spc"
  chmod +x "$tools/spc"
  PATH="$tools:$PATH"
  export PATH
  spc --version
  spc build:php --help | grep -- '--build-frankenphp' >/dev/null
  spc build:php --help | grep -- '--dl-custom-local' >/dev/null
fi

"release/artifacts/recipes/$PV_RECIPE_RESOURCE/build.sh"

defaults="$PV_ARTIFACT_OUT_DIR/defaults.toml"
resources="$PV_RECIPE_RESOURCE"
if [ "$PV_RECIPE_RESOURCE" = php ]; then resources='php frankenphp'; fi
for resource in $resources; do
  printf '[[resource]]\nname = "%s"\ndefault_track = "%s"\n\n' "$resource" "$PV_RECIPE_TRACK"
done >"$defaults"
cargo run --locked -p pv-release -- generate-manifest \
  --records "$PV_ARTIFACT_RECORD_DIR" --revocations release/artifacts/revocations \
  --defaults "$defaults" --output "$PV_ARTIFACT_OUT_DIR/manifest.json" \
  --base-url https://artifacts.example.test

if [ -d "$PV_ARTIFACT_OUT_DIR/work" ]; then chmod -R u+w "$PV_ARTIFACT_OUT_DIR/work"; fi
rm -rf "$PV_ARTIFACT_OUT_DIR/sources" "$PV_ARTIFACT_OUT_DIR/work" "$PV_ARTIFACT_OUT_DIR/tools"
