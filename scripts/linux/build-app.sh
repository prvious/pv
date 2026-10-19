#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"
: "${PV_APP_OUT_DIR:?}" "${PV_COMMIT:?}" "${PV_BUILD_RUN_ID:?}" "${CARGO_TARGET_DIR:?}"
: "${PV_DEFAULT_ARTIFACT_MANIFEST_URL:?}" "${PV_APP_SOURCE_ARCHIVE:?}"
case "$(uname -s):$(uname -m)" in
  Linux:x86_64) platform=linux-amd64; rust_target=x86_64-unknown-linux-gnu ;;
  Linux:aarch64) platform=linux-arm64; rust_target=aarch64-unknown-linux-gnu ;;
  *) die "Linux app builds require a native Linux CPU" ;;
esac

cargo build --release --locked --target "$rust_target" \
  -p pv -p pv-privileged-helper -p pv-release --bins
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
helper_version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/privileged-helper/Cargo.toml | head -n 1)
protocol=$(sed -n 's/^pub const HELPER_PROTOCOL_VERSION: u32 = \([0-9][0-9]*\);/\1/p' crates/platform/src/helper.rs)
binary_dir="$CARGO_TARGET_DIR/$rust_target/release"
object_key="pv/$version/pv-$platform"
helper_key="pv/$version/pv-helper-$helper_version-$platform"
mkdir -p "$PV_APP_OUT_DIR/pv/$version" "$PV_APP_OUT_DIR/app/records"
cp "$binary_dir/pv" "$PV_APP_OUT_DIR/$object_key"
cp "$binary_dir/pv-helper" "$PV_APP_OUT_DIR/$helper_key"
cp "$binary_dir/pv-release" "$PV_APP_OUT_DIR/app/pv-release"
cp "$PV_APP_SOURCE_ARCHIVE" "$PV_APP_OUT_DIR/source.tar.gz"
source_sha256=$(sha256_file "$PV_APP_OUT_DIR/source.tar.gz")
"$binary_dir/pv-release" write-app-release-record \
  --record "$PV_APP_OUT_DIR/app/records/pv-$platform.json" \
  --binary "$PV_APP_OUT_DIR/$object_key" --helper-binary "$PV_APP_OUT_DIR/$helper_key" \
  --version "$version" --minimum-pv-version 0.2.7 --published-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --platform "$platform" --object-key "$object_key" --helper-version "$helper_version" \
  --helper-protocol-version "$protocol" --helper-object-key "$helper_key" \
  --source-url https://artifacts.pv-linux.invalid/source.tar.gz --source-sha256 "$source_sha256" \
  --recipe scripts/linux/build-app.sh --pv-commit "$PV_COMMIT" --build-run-id "$PV_BUILD_RUN_ID"
pv_recipe_validate_elf_tree "$PV_APP_OUT_DIR" "$platform"
