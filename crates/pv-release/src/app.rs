use camino::{Utf8Path, Utf8PathBuf};
use data_encoding::HEXLOWER;
use self_update::{
    AppUpdateManifest, AppUpdateManifestError, AppUpdatePlatform, AppUpdatePublishedAt,
    AppUpdateVersion, Sha256Digest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use url::Url;

const SUPPORTED_SCHEMA_VERSION: u64 = 1;
const STABLE_CHANNEL: &str = "stable";
const APP_INSTALLER_TEMPLATE: &str = r##"#!/usr/bin/env bash
set -euo pipefail

PV_VERSION=@@PV_VERSION@@
ARM64_URL=@@ARM64_URL@@
ARM64_SHA256=@@ARM64_SHA256@@
ARM64_SIZE=@@ARM64_SIZE@@
AMD64_URL=@@AMD64_URL@@
AMD64_SHA256=@@AMD64_SHA256@@
AMD64_SIZE=@@AMD64_SIZE@@

YES=0
NON_INTERACTIVE=0
NO_SETUP=0
NO_PATH=0
PV_HOME="${HOME}/.pv"
PV_BIN_DIR="${PV_HOME}/bin"
PV_RELEASE_DIR="${HOME}/.pv/bin/releases/${PV_VERSION}"
PV_RELEASE_BIN="${PV_RELEASE_DIR}/pv"
PV_ACTIVE_BIN="${PV_BIN_DIR}/pv"
TMP_DIR=

usage() {
  cat <<'USAGE'
PV macOS installer

Usage: install.sh [OPTIONS]

Options:
  --yes              Accept PV installer confirmations
  --non-interactive  Disable prompts and fail when interactive input is required
  --no-setup         Install the pv binary without running pv setup
  --no-path          Skip shell profile PATH integration
  --help             Show this help
USAGE
}

info() {
  printf 'pv installer: %s\n' "$*" >&2
}

warn() {
  printf 'pv installer: %s\n' "$*" >&2
}

die() {
  printf 'pv installer: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "${TMP_DIR}" ]; then
    rm -rf "${TMP_DIR}"
  fi
}

trap cleanup EXIT

while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes)
      YES=1
      ;;
    --non-interactive)
      NON_INTERACTIVE=1
      ;;
    --no-setup)
      NO_SETUP=1
      ;;
    --no-path)
      NO_PATH=1
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      die "unknown option: $1"
      ;;
  esac
  shift
done

is_rosetta() {
  [ "$(sysctl -in sysctl.proc_translated 2>/dev/null || true)" = "1" ]
}

select_asset() {
  local os machine platform

  os="$(uname -s)"
  if [ "${os}" != "Darwin" ]; then
    die "macOS is required; found ${os}"
  fi

  machine="$(uname -m)"
  case "${machine}" in
    arm64|aarch64)
      platform="darwin-arm64"
      ;;
    x86_64|amd64)
      if is_rosetta; then
        platform="darwin-arm64"
      else
        platform="darwin-amd64"
      fi
      ;;
    *)
      die "unsupported macOS architecture: ${machine}"
      ;;
  esac

  case "${platform}" in
    darwin-arm64)
      ASSET_URL="${ARM64_URL}"
      EXPECTED_SHA256="${ARM64_SHA256}"
      EXPECTED_SIZE="${ARM64_SIZE}"
      ;;
    darwin-amd64)
      ASSET_URL="${AMD64_URL}"
      EXPECTED_SHA256="${AMD64_SHA256}"
      EXPECTED_SIZE="${AMD64_SIZE}"
      ;;
    *)
      die "unsupported PV installer platform: ${platform}"
      ;;
  esac
}

sha256_file() {
  local path

  path="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${path}" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${path}" | awk '{print $1}'
  else
    die "neither shasum nor sha256sum is available for checksum verification"
  fi
}

verify_download() {
  local path actual_size actual_sha256

  path="$1"
  actual_size="$(wc -c < "${path}" | tr -d '[:space:]')"
  if [ "${actual_size}" != "${EXPECTED_SIZE}" ]; then
    rm -f "${path}"
    die "download size mismatch: expected ${EXPECTED_SIZE} bytes, got ${actual_size}"
  fi

  actual_sha256="$(sha256_file "${path}")"
  if [ "${actual_sha256}" != "${EXPECTED_SHA256}" ]; then
    rm -f "${path}"
    die "download checksum mismatch: expected ${EXPECTED_SHA256}, got ${actual_sha256}"
  fi
}

download_asset() {
  local download

  TMP_DIR="${PV_HOME}/tmp/installer.$$"
  mkdir -p "${TMP_DIR}"
  download="${TMP_DIR}/pv"

  curl --fail --location --silent --show-error \
    --connect-timeout 15 \
    --max-time 300 \
    --retry 3 \
    --retry-delay 2 \
    --retry-connrefused \
    --output "${download}" "${ASSET_URL}" || {
    rm -f "${download}"
    die "failed to download ${ASSET_URL}"
  }

  verify_download "${download}"
  install_binary "${download}"
}

install_binary() {
  local download release_tmp link_tmp

  download="$1"
  mkdir -p "${PV_RELEASE_DIR}" "${PV_BIN_DIR}"
  chmod 755 "${download}"

  release_tmp="${PV_RELEASE_BIN}.tmp.$$"
  mv "${download}" "${release_tmp}"
  mv -f "${release_tmp}" "${PV_RELEASE_BIN}"

  link_tmp="${PV_ACTIVE_BIN}.tmp.$$"
  rm -f "${link_tmp}"
  ln -s "${PV_RELEASE_BIN}" "${link_tmp}"
  mv -f "${link_tmp}" "${PV_ACTIVE_BIN}"
}

detect_shell_profile() {
  local shell_path shell_name

  shell_path="${SHELL:-}"
  shell_name="${shell_path##*/}"
  case "${shell_name}" in
    zsh)
      PROFILE_SHELL="zsh"
      PROFILE_PATH="${HOME}/.zprofile"
      ;;
    bash)
      PROFILE_SHELL="bash"
      PROFILE_PATH="${HOME}/.bash_profile"
      ;;
    fish)
      PROFILE_SHELL="fish"
      PROFILE_PATH="${HOME}/.config/fish/config.fish"
      ;;
    *)
      return 1
      ;;
  esac
}

profile_block() {
  local shell_name

  shell_name="$1"
  case "${shell_name}" in
    fish)
      cat <<'FISH'
# >>> PV ENV
if test -x "$HOME/.pv/bin/pv"
  "$HOME/.pv/bin/pv" env --shell fish | source
end
# <<< PV ENV
FISH
      ;;
    *)
      cat <<EOF
# >>> PV ENV
if [ -x "\$HOME/.pv/bin/pv" ]; then
  eval "\$("\$HOME/.pv/bin/pv" env --shell ${shell_name})"
fi
# <<< PV ENV
EOF
      ;;
  esac
}

manual_shell_instructions() {
  local shell_name

  shell_name="${1:-zsh}"
  warn "add PV to your shell profile manually if you want pv on PATH in new terminals"
  case "${shell_name}" in
    fish)
      printf '  "%s" env --shell fish | source\n' "${PV_ACTIVE_BIN}" >&2
      ;;
    bash|zsh)
      printf '  eval "%s("%s" env --shell %s)"\n' '$' "${PV_ACTIVE_BIN}" "${shell_name}" >&2
      ;;
    *)
      printf '  eval "%s("%s" env --shell zsh)"\n' '$' "${PV_ACTIVE_BIN}" >&2
      ;;
  esac
}

confirm_profile_edit() {
  local action profile reply

  action="$1"
  profile="$2"
  if [ "${YES}" -eq 1 ]; then
    return 0
  fi

  if [ "${NON_INTERACTIVE}" -eq 1 ]; then
    die "shell profile confirmation required to ${action} ${profile}"
  fi

  if [ ! -r /dev/tty ] || [ ! -w /dev/tty ]; then
    warn "cannot prompt to ${action} ${profile}; skipping shell profile integration"
    return 1
  fi

  printf 'pv installer: %s %s? [y/N] ' "${action}" "${profile}" >/dev/tty
  IFS= read -r reply </dev/tty || return 1
  case "${reply}" in
    y|Y|yes|YES)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

write_profile_block() {
  local profile block profile_dir timestamp backup tmp line inserted skipping

  profile="$1"
  block="$2"
  profile_dir="${profile%/*}"
  if [ "${profile_dir}" != "${profile}" ]; then
    mkdir -p "${profile_dir}" || return 1
  fi

  if [ ! -f "${profile}" ]; then
    printf '%s\n' "${block}" >"${profile}" || return 1
    info "created ${profile} with PV ENV"
    return 0
  fi

  timestamp="$(date +%Y%m%d-%H%M%S)"
  backup="${profile}.${timestamp}.pv.bak"
  cp "${profile}" "${backup}" || return 1

  tmp="${profile}.pv.tmp.$$"
  : >"${tmp}" || return 1
  inserted=0
  skipping=0
  while IFS= read -r line || [ -n "${line}" ]; do
    if [ "${line}" = "# >>> PV ENV" ]; then
      if [ "${inserted}" -eq 0 ]; then
        printf '%s\n' "${block}" >>"${tmp}" || return 1
        inserted=1
      fi
      skipping=1
      continue
    fi

    if [ "${skipping}" -eq 1 ]; then
      if [ "${line}" = "# <<< PV ENV" ]; then
        skipping=0
      fi
      continue
    fi

    printf '%s\n' "${line}" >>"${tmp}" || return 1
  done <"${profile}"

  if [ "${inserted}" -eq 0 ]; then
    if [ -s "${tmp}" ]; then
      printf '\n' >>"${tmp}" || return 1
    fi
    printf '%s\n' "${block}" >>"${tmp}" || return 1
  fi

  mv "${tmp}" "${profile}" || return 1
  info "updated ${profile}; backup saved at ${backup}"
}

install_shell_profile_block() {
  local action block

  if [ "${NO_SETUP}" -eq 1 ] || [ "${NO_PATH}" -eq 1 ]; then
    return 0
  fi

  if ! detect_shell_profile; then
    warn "unsupported or unknown shell '${SHELL:-}'; skipping shell profile integration"
    manual_shell_instructions unknown
    return 0
  fi

  if [ -f "${PROFILE_PATH}" ]; then
    action="update"
  else
    action="create"
  fi

  if ! confirm_profile_edit "${action}" "${PROFILE_PATH}"; then
    manual_shell_instructions "${PROFILE_SHELL}"
    return 0
  fi

  block="$(profile_block "${PROFILE_SHELL}")"
  if ! write_profile_block "${PROFILE_PATH}" "${block}"; then
    if [ "${NON_INTERACTIVE}" -eq 1 ]; then
      die "failed to ${action} ${PROFILE_PATH}"
    fi
    warn "failed to ${action} ${PROFILE_PATH}; continuing without shell profile integration"
    manual_shell_instructions "${PROFILE_SHELL}"
  fi
}

run_setup() {
  if [ "${NO_SETUP}" -eq 1 ]; then
    return 0
  fi

  install_shell_profile_block

  set -- setup
  if [ "${YES}" -eq 1 ]; then
    set -- "$@" --yes
  fi
  if [ "${NON_INTERACTIVE}" -eq 1 ]; then
    set -- "$@" --non-interactive
  fi
  if [ "${NO_PATH}" -eq 1 ]; then
    set -- "$@" --no-path
  fi

  if ! "${PV_ACTIVE_BIN}" "$@"; then
    warn "pv setup failed after installing ${PV_ACTIVE_BIN}"
    warn "rerun \"${PV_ACTIVE_BIN}\" setup after fixing the issue"
    return 1
  fi
}

select_asset
download_asset
run_setup
info "PV ${PV_VERSION} installed at ${PV_ACTIVE_BIN}"
"##;

#[derive(Clone, Debug)]
pub struct WriteAppReleaseRecordRequest {
    pub record: Utf8PathBuf,
    pub binary: Utf8PathBuf,
    pub version: String,
    pub minimum_pv_version: String,
    pub published_at: String,
    pub platform: String,
    pub object_key: String,
    pub source_url: String,
    pub source_sha256: String,
    pub recipe: String,
    pub pv_commit: String,
    pub build_run_id: String,
}

#[derive(Clone, Debug)]
pub struct AppReleaseRecord {
    path: Utf8PathBuf,
    schema_version: u64,
    channel: String,
    version: String,
    minimum_pv_version: String,
    published_at: String,
    platform: AppUpdatePlatform,
    object_key: String,
    sha256: String,
    size: u64,
    provenance: AppReleaseProvenance,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppReleaseProvenance {
    source_url: String,
    source_sha256: String,
    recipe: String,
    pv_commit: String,
    build_run_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAppReleaseRecord {
    schema_version: u64,
    channel: String,
    version: String,
    minimum_pv_version: String,
    published_at: String,
    platform: String,
    object_key: String,
    sha256: String,
    size: u64,
    provenance: AppReleaseProvenance,
}

#[derive(Serialize)]
struct AppReleaseRecordJson<'a> {
    schema_version: u64,
    channel: &'a str,
    version: &'a str,
    minimum_pv_version: &'a str,
    published_at: &'a str,
    platform: &'a str,
    object_key: &'a str,
    sha256: String,
    size: u64,
    provenance: AppReleaseProvenanceJson<'a>,
}

#[derive(Serialize)]
struct AppReleaseProvenanceJson<'a> {
    source_url: &'a str,
    source_sha256: &'a str,
    recipe: &'a str,
    pv_commit: &'a str,
    build_run_id: &'a str,
}

#[derive(Serialize)]
struct AppManifestJson {
    schema_version: u64,
    channel: String,
    version: String,
    minimum_pv_version: String,
    published_at: String,
    assets: Vec<AppManifestAssetJson>,
}

#[derive(Serialize)]
struct AppManifestAssetJson {
    platform: String,
    url: String,
    sha256: String,
    size: u64,
}

struct InstallerAsset {
    url: String,
    sha256: String,
    size: u64,
}

impl AppReleaseRecord {
    pub fn from_json(path: &Utf8Path, json: &str) -> crate::Result<Self> {
        let raw: RawAppReleaseRecord =
            serde_json::from_str(json).map_err(|error| invalid_app(path, error.to_string()))?;

        if raw.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(invalid_app(
                path,
                format!(
                    "unsupported PV app release record schema version {}, expected {SUPPORTED_SCHEMA_VERSION}",
                    raw.schema_version
                ),
            ));
        }
        if raw.channel != STABLE_CHANNEL {
            return Err(invalid_app(
                path,
                AppUpdateManifestError::UnsupportedChannel {
                    channel: raw.channel.clone(),
                },
            ));
        }

        AppUpdateVersion::parse(raw.version.clone())
            .map_err(|error| invalid_app(path, format!("invalid version: {error}")))?;
        AppUpdateVersion::parse(raw.minimum_pv_version.clone())
            .map_err(|error| invalid_app(path, format!("invalid minimum_pv_version: {error}")))?;
        AppUpdatePublishedAt::parse(raw.published_at.clone())
            .map_err(|error| invalid_app(path, error))?;
        let platform =
            AppUpdatePlatform::parse(&raw.platform).map_err(|error| invalid_app(path, error))?;
        Sha256Digest::parse(raw.sha256.clone()).map_err(|error| invalid_app(path, error))?;
        if raw.size == 0 {
            return Err(invalid_app(
                path,
                AppUpdateManifestError::InvalidAssetSize {
                    platform: platform.as_str().to_string(),
                    size: raw.size,
                },
            ));
        }
        validate_relative_path(path, "object_key", &raw.object_key)?;
        let expected_object_key = format!("pv/{}/pv-{}", raw.version, platform.as_str());
        if raw.object_key != expected_object_key {
            return Err(invalid_app(
                path,
                format!("object_key must be `{expected_object_key}`"),
            ));
        }
        raw.provenance.validate(path)?;

        Ok(Self {
            path: path.to_path_buf(),
            schema_version: raw.schema_version,
            channel: raw.channel,
            version: raw.version,
            minimum_pv_version: raw.minimum_pv_version,
            published_at: raw.published_at,
            platform,
            object_key: raw.object_key,
            sha256: raw.sha256.to_ascii_lowercase(),
            size: raw.size,
            provenance: raw.provenance,
        })
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn schema_version(&self) -> u64 {
        self.schema_version
    }

    pub fn channel(&self) -> &str {
        &self.channel
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn minimum_pv_version(&self) -> &str {
        &self.minimum_pv_version
    }

    pub fn published_at(&self) -> &str {
        &self.published_at
    }

    pub fn platform(&self) -> AppUpdatePlatform {
        self.platform
    }

    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn provenance(&self) -> &AppReleaseProvenance {
        &self.provenance
    }
}

impl AppReleaseProvenance {
    fn validate(&self, path: &Utf8Path) -> crate::Result<()> {
        validate_https_url(path, "source_url", &self.source_url)?;
        Sha256Digest::parse(self.source_sha256.clone()).map_err(|error| {
            invalid_app(path, format!("invalid provenance source_sha256: {error}"))
        })?;
        validate_relative_path(path, "recipe", &self.recipe)?;
        validate_commit(path, &self.pv_commit)?;
        require_non_empty(path, "build_run_id", &self.build_run_id)?;

        Ok(())
    }

    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn recipe(&self) -> &str {
        &self.recipe
    }

    pub fn pv_commit(&self) -> &str {
        &self.pv_commit
    }

    pub fn build_run_id(&self) -> &str {
        &self.build_run_id
    }
}

pub fn write_app_release_record(request: &WriteAppReleaseRecordRequest) -> crate::Result<()> {
    let (sha256, size) = digest_and_size(&request.binary)?;
    let record = AppReleaseRecordJson {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        channel: STABLE_CHANNEL,
        version: &request.version,
        minimum_pv_version: &request.minimum_pv_version,
        published_at: &request.published_at,
        platform: &request.platform,
        object_key: &request.object_key,
        sha256,
        size,
        provenance: AppReleaseProvenanceJson {
            source_url: &request.source_url,
            source_sha256: &request.source_sha256,
            recipe: &request.recipe,
            pv_commit: &request.pv_commit,
            build_run_id: &request.build_run_id,
        },
    };

    let mut json = serde_json::to_string_pretty(&record)
        .map_err(|error| invalid_app(&request.record, error))?;
    json.push('\n');
    AppReleaseRecord::from_json(&request.record, &json)?;

    if let Some(parent) = request
        .record
        .parent()
        .filter(|parent| !parent.as_str().is_empty())
    {
        create_dir_all(parent)?;
    }
    write_bytes(&request.record, json.as_bytes())
}

pub fn generate_app_manifest_file(
    records: &Utf8Path,
    output: &Utf8Path,
    base_url: &str,
) -> crate::Result<()> {
    let records = load_app_release_records(records)?;
    let manifest = generate_app_manifest_json(&records, base_url)?;

    if let Some(parent) = output.parent().filter(|parent| !parent.as_str().is_empty()) {
        create_dir_all(parent)?;
    }
    write_string(output, &manifest)
}

pub fn generate_app_installer_file(
    records: &Utf8Path,
    output: &Utf8Path,
    base_url: &str,
) -> crate::Result<()> {
    let records = load_app_release_records(records)?;
    let installer = generate_app_installer_script(&records, base_url)?;

    if let Some(parent) = output.parent().filter(|parent| !parent.as_str().is_empty()) {
        create_dir_all(parent)?;
    }
    write_string(output, &installer)
}

pub fn generate_app_manifest_json(
    records: &[AppReleaseRecord],
    base_url: &str,
) -> crate::Result<String> {
    let Some((first_record, remaining_records)) = records.split_first() else {
        return Err(crate::ReleaseError::GeneratedAppManifestInvalid {
            reason: "app release records must not be empty".to_string(),
        });
    };

    let mut seen_platforms = BTreeSet::new();
    validate_app_record_group(first_record, remaining_records, &mut seen_platforms)?;

    let manifest = AppManifestJson {
        schema_version: first_record.schema_version(),
        channel: first_record.channel().to_string(),
        version: first_record.version().to_string(),
        minimum_pv_version: first_record.minimum_pv_version().to_string(),
        published_at: first_record.published_at().to_string(),
        assets: records
            .iter()
            .map(|record| AppManifestAssetJson {
                platform: record.platform().as_str().to_string(),
                url: artifact_url(base_url, record.object_key()),
                sha256: record.sha256().to_string(),
                size: record.size(),
            })
            .collect(),
    };

    let json = serde_json::to_string_pretty(&manifest).map_err(|error| {
        crate::ReleaseError::GeneratedAppManifestInvalid {
            reason: error.to_string(),
        }
    })?;
    AppUpdateManifest::parse(&json).map_err(|error| {
        crate::ReleaseError::GeneratedAppManifestInvalid {
            reason: error.to_string(),
        }
    })?;

    Ok(json)
}

pub fn generate_app_installer_script(
    records: &[AppReleaseRecord],
    base_url: &str,
) -> crate::Result<String> {
    let Some((first_record, remaining_records)) = records.split_first() else {
        return Err(crate::ReleaseError::GeneratedAppInstallerInvalid {
            reason: "app release records must not be empty".to_string(),
        });
    };

    let mut seen_platforms = BTreeSet::new();
    validate_app_record_group(first_record, remaining_records, &mut seen_platforms)?;
    generate_app_manifest_json(records, base_url)?;

    let arm64 = require_installer_asset(records, AppUpdatePlatform::DarwinArm64, base_url)?;
    let amd64 = require_installer_asset(records, AppUpdatePlatform::DarwinAmd64, base_url)?;

    Ok(APP_INSTALLER_TEMPLATE
        .replace("@@PV_VERSION@@", &shell_quote(first_record.version()))
        .replace("@@ARM64_URL@@", &shell_quote(&arm64.url))
        .replace("@@ARM64_SHA256@@", &shell_quote(&arm64.sha256))
        .replace("@@ARM64_SIZE@@", &shell_quote(&arm64.size.to_string()))
        .replace("@@AMD64_URL@@", &shell_quote(&amd64.url))
        .replace("@@AMD64_SHA256@@", &shell_quote(&amd64.sha256))
        .replace("@@AMD64_SIZE@@", &shell_quote(&amd64.size.to_string())))
}

pub fn load_app_release_records(root: &Utf8Path) -> crate::Result<Vec<AppReleaseRecord>> {
    json_files(root)?
        .into_iter()
        .map(|path| {
            let json = read_to_string(&path)?;
            AppReleaseRecord::from_json(&path, &json)
        })
        .collect()
}

fn validate_app_record_group(
    first_record: &AppReleaseRecord,
    remaining_records: &[AppReleaseRecord],
    seen_platforms: &mut BTreeSet<AppUpdatePlatform>,
) -> crate::Result<()> {
    seen_platforms.insert(first_record.platform());
    for record in remaining_records {
        require_same_metadata(
            "channel",
            first_record.channel(),
            record.channel(),
            record.path(),
        )?;
        require_same_metadata(
            "version",
            first_record.version(),
            record.version(),
            record.path(),
        )?;
        require_same_metadata(
            "minimum_pv_version",
            first_record.minimum_pv_version(),
            record.minimum_pv_version(),
            record.path(),
        )?;
        require_same_metadata(
            "published_at",
            first_record.published_at(),
            record.published_at(),
            record.path(),
        )?;
        if !seen_platforms.insert(record.platform()) {
            return Err(crate::ReleaseError::DuplicateAppReleasePlatform {
                platform: record.platform().as_str().to_string(),
            });
        }
    }

    Ok(())
}

fn require_installer_asset(
    records: &[AppReleaseRecord],
    platform: AppUpdatePlatform,
    base_url: &str,
) -> crate::Result<InstallerAsset> {
    let Some(record) = records.iter().find(|record| record.platform() == platform) else {
        return Err(crate::ReleaseError::GeneratedAppInstallerInvalid {
            reason: format!("app release records must include {platform}"),
        });
    };

    Ok(InstallerAsset {
        url: artifact_url(base_url, record.object_key()),
        sha256: record.sha256().to_string(),
        size: record.size(),
    })
}

fn require_same_metadata(
    field: &'static str,
    expected: &str,
    actual: &str,
    path: &Utf8Path,
) -> crate::Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(crate::ReleaseError::AppReleaseMetadataMismatch {
            field,
            expected: expected.to_string(),
            actual: actual.to_string(),
            path: path.to_string(),
        })
    }
}

fn json_files(root: &Utf8Path) -> crate::Result<Vec<Utf8PathBuf>> {
    let mut paths = Vec::new();
    collect_json_files(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_json_files(root: &Utf8Path, paths: &mut Vec<Utf8PathBuf>) -> crate::Result<()> {
    for entry in root
        .read_dir_utf8()
        .map_err(|error| filesystem_error(root, error))?
    {
        let entry = entry.map_err(|error| filesystem_error(root, error))?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(path, paths)?;
        } else if path.extension() == Some("json") {
            paths.push(path.to_path_buf());
        }
    }

    Ok(())
}

fn digest_and_size(path: &Utf8Path) -> crate::Result<(String, u64)> {
    let mut file = open_file(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    let mut size = 0;

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| filesystem_error(path, error))?;
        if bytes_read == 0 {
            break;
        }
        size += bytes_read as u64;
        hasher.update(&buffer[..bytes_read]);
    }

    Ok((HEXLOWER.encode(&hasher.finalize()), size))
}

fn artifact_url(base_url: &str, object_key: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        object_key.trim_start_matches('/')
    )
}

fn shell_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

fn validate_relative_path(path: &Utf8Path, field: &str, value: &str) -> crate::Result<()> {
    if relative_path_is_valid(value) {
        Ok(())
    } else {
        Err(invalid_app(
            path,
            format!("{field} contains invalid relative path `{value}`"),
        ))
    }
}

fn relative_path_is_valid(value: &str) -> bool {
    let candidate = Utf8Path::new(value);
    !candidate.is_absolute()
        && !value.is_empty()
        && !value.contains('\\')
        && !value.split('/').any(str::is_empty)
        && !candidate
            .components()
            .any(|component| matches!(component.as_str(), "." | ".."))
}

fn validate_https_url(path: &Utf8Path, field: &str, value: &str) -> crate::Result<()> {
    let value = require_non_empty(path, field, value)?;
    if value.contains('\\') {
        return Err(invalid_app(
            path,
            format!("{field} must be an https URL with a host"),
        ));
    }

    let parsed = Url::parse(value)
        .map_err(|_error| invalid_app(path, format!("{field} must be an https URL with a host")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid_app(
            path,
            format!("{field} must be an https URL with a host"),
        ));
    }

    Ok(())
}

fn validate_commit(path: &Utf8Path, value: &str) -> crate::Result<()> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid_app(
            path,
            "pv_commit must be a 40-character hex commit",
        ))
    }
}

fn require_non_empty<'a>(path: &Utf8Path, field: &str, value: &'a str) -> crate::Result<&'a str> {
    if value.trim().is_empty() {
        Err(invalid_app(path, format!("{field} must not be empty")))
    } else {
        Ok(value)
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns direct app binary reads for release records"
)]
fn open_file(path: &Utf8Path) -> crate::Result<std::fs::File> {
    std::fs::File::open(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads repository-local app release records"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling creates generated app release directories"
)]
fn create_dir_all(path: &Utf8Path) -> crate::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated app release records"
)]
fn write_bytes(path: &Utf8Path, content: &[u8]) -> crate::Result<()> {
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling writes generated app manifests"
)]
fn write_string(path: &Utf8Path, content: &str) -> crate::Result<()> {
    std::fs::write(path, content).map_err(|error| filesystem_error(path, error))
}

fn invalid_app(path: &Utf8Path, reason: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::InvalidAppReleaseRecord {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

fn filesystem_error(path: &Utf8Path, error: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    }
}
