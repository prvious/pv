use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use data_encoding::HEXLOWER;
use insta::assert_snapshot;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Output;

#[expect(
    clippy::disallowed_types,
    reason = "installer contract tests execute generated CLI and shell script fixtures"
)]
type StdCommand = std::process::Command;

const APP_VERSION: &str = "0.9.0";
const STAGING_BASE_URL: &str = "https://artifacts-staging.pv.prvious.dev";
const ARM64_OBJECT_KEY: &str = "pv/0.9.0/pv-darwin-arm64";
const AMD64_OBJECT_KEY: &str = "pv/0.9.0/pv-darwin-amd64";

#[test]
fn generated_app_installer_embeds_staging_assets_and_contract() -> Result<()> {
    let fixture = AppInstallerFixture::new()?;
    let installer = fixture.generate_installer()?;

    assert_snapshot!(installer_contract_summary(&installer), @r#"
    version_present=true
    staging_base_present=true
    arm64_url_present=true
    amd64_url_present=true
    arm64_sha256_present=true
    amd64_sha256_present=true
    arm64_size_present=true
    amd64_size_present=true
    app_manifest_not_embedded=true
    release_install_path_present=true
    active_symlink_path_present=true
    setup_invocation_present=true
    no_setup_flag_present=true
    no_path_flag_present=true
    yes_flag_present=true
    non_interactive_flag_present=true
    pv_env_block_present=true
    checksum_verification_present=true
    curl_connect_timeout_present=true
    curl_max_time_present=true
    curl_retry_present=true
    "#);

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_no_setup_installs_binary_and_symlink_only() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer(&["--no-setup"], ChecksumMode::Match)?;

    assert!(
        output.status.success(),
        "installer should succeed with --no-setup: {}",
        command_output_summary(&output)
    );
    assert_eq!(read_file(&fixture.release_binary())?, fake_pv_binary());
    assert!(active_pv_symlink_points_to_release(
        &fixture.active_binary(),
        &fixture.release_binary()
    )?);
    assert_eq!(
        read_link(&fixture.active_binary())?,
        Utf8PathBuf::from(format!("releases/{APP_VERSION}/pv"))
    );
    assert!(!path_exists(&fixture.pv_log()));
    assert!(!path_exists(&fixture.home().join(".zprofile")));
    assert!(!path_exists(&fixture.home().join(".bash_profile")));
    assert!(!path_exists(
        &fixture.home().join(".config/fish/config.fish")
    ));

    assert_snapshot!(fixture.command_logs()?, @r#"
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-arm64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=<arm64-sha256>

    pv:
    <missing>
    "#);

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_default_mode_invokes_pv_setup() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer(&[], ChecksumMode::Match)?;

    assert!(
        output.status.success(),
        "installer should succeed in default mode: {}",
        command_output_summary(&output)
    );
    assert_eq!(read_file(&fixture.release_binary())?, fake_pv_binary());
    assert!(active_pv_symlink_points_to_release(
        &fixture.active_binary(),
        &fixture.release_binary()
    )?);

    assert_snapshot!(fixture.command_logs()?, @"
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-arm64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=<arm64-sha256>

    pv:
    setup
    ");

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_fish_profile_block_matches_setup_block() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer_with_shell(&["--yes"], ChecksumMode::Match, "/bin/fish")?;

    assert!(
        output.status.success(),
        "installer should succeed for fish profile setup: {}",
        command_output_summary(&output)
    );

    assert_snapshot!(read_file(&fixture.home().join(".config/fish/config.fish"))?, @r##"
    # >>> PV ENV
    if test -x "$HOME/.pv/bin/pv"
      eval ("$HOME/.pv/bin/pv" env --shell fish | string collect)
    end
    # <<< PV ENV

    "##);

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_leaves_incomplete_profile_block_unchanged() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let profile = fixture.home().join(".zprofile");
    let original_profile = "export BEFORE=1\n# >>> PV ENV\nexport SHOULD_STAY=1\n";
    write_file(&profile, original_profile)?;

    let output = fixture.run_installer_with_shell(&["--yes"], ChecksumMode::Match, "/bin/zsh")?;

    assert!(
        output.status.success(),
        "installer should continue setup after skipping incomplete shell profile block: {}",
        command_output_summary(&output)
    );
    assert_eq!(read_file(&profile)?, original_profile);
    assert_eq!(read_file(&fixture.release_binary())?, fake_pv_binary());
    assert!(active_pv_symlink_points_to_release(
        &fixture.active_binary(),
        &fixture.release_binary()
    )?);

    assert_snapshot!(incomplete_profile_block_summary(&output, &fixture, &profile, original_profile)?, @r#"
    status_success=true
    stderr_mentions_incomplete_profile=true
    profile_unchanged=true
    logs:
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-arm64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=<arm64-sha256>

    pv:
    setup --yes
    "#);

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_non_interactive_fails_when_shell_profile_confirmation_is_required() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer_with_shell(
        &["--non-interactive"],
        ChecksumMode::Match,
        "/bin/zsh",
    )?;

    assert!(
        !output.status.success(),
        "installer should fail when --non-interactive would need shell profile confirmation"
    );
    assert_eq!(read_file(&fixture.release_binary())?, fake_pv_binary());
    assert!(active_pv_symlink_points_to_release(
        &fixture.active_binary(),
        &fixture.release_binary()
    )?);
    assert!(!path_exists(&fixture.pv_log()));
    assert!(!path_exists(&fixture.home().join(".zprofile")));

    assert_snapshot!(non_interactive_shell_profile_confirmation_summary(&output, &fixture)?, @r#"
    status_success=false
    stderr_mentions_shell_profile_confirmation=true
    logs:
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-arm64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=<arm64-sha256>

    pv:
    <missing>
    "#);

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_checksum_mismatch_deletes_bad_download_and_does_not_install() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer(&["--no-setup"], ChecksumMode::Mismatch)?;

    assert!(
        !output.status.success(),
        "installer should reject checksum mismatches"
    );
    assert!(!path_exists(&fixture.release_binary()));
    assert!(!path_exists(&fixture.active_binary()));
    assert!(!path_exists(&fixture.pv_log()));
    assert_download_outputs_were_removed(fixture.curl_log())?;

    assert_snapshot!(checksum_mismatch_summary(&output, &fixture)?, @r#"
    status_success=false
    stderr_mentions_checksum=true
    logs:
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-arm64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=0000000000000000000000000000000000000000000000000000000000000000

    pv:
    <missing>
    "#);

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_downloads_amd64_asset_on_native_x86_64() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer_with_platform(
        &["--no-setup"],
        ChecksumMode::Match,
        "/bin/pv-unsupported-shell",
        "x86_64",
        false,
    )?;

    assert!(
        output.status.success(),
        "installer should succeed on native x86_64: {}",
        command_output_summary(&output)
    );

    assert_snapshot!(fixture.command_logs()?, @"
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-amd64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=<amd64-sha256>

    pv:
    <missing>
    ");

    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_downloads_arm64_asset_under_rosetta() -> Result<()> {
    let fixture = InstallerExecutionFixture::new()?;
    let output = fixture.run_installer_with_platform(
        &["--no-setup"],
        ChecksumMode::Match,
        "/bin/pv-unsupported-shell",
        "x86_64",
        true,
    )?;

    assert!(
        output.status.success(),
        "installer should succeed under Rosetta: {}",
        command_output_summary(&output)
    );

    assert_snapshot!(fixture.command_logs()?, @r#"
    curl:
    url=https://artifacts-staging.pv.prvious.dev/pv/0.9.0/pv-darwin-arm64
    output=<download>

    checksum:
    tool=<checksum-tool> target=<download> sha=<arm64-sha256>

    pv:
    <missing>
    "#);

    Ok(())
}

struct AppInstallerFixture {
    tempdir: Utf8TempDir,
    records: Utf8PathBuf,
    output: Utf8PathBuf,
    arm64_sha256: String,
    amd64_sha256: String,
}

impl AppInstallerFixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        let records = tempdir.path().join("records");
        let output = tempdir.path().join("install.sh");
        let arm64_binary = fake_pv_binary();
        let amd64_binary = "#!/bin/sh\nprintf '%s\\n' pv-amd64-fixture\n";
        let (arm64_sha256, arm64_size) = digest_and_size(arm64_binary.as_bytes());
        let (amd64_sha256, amd64_size) = digest_and_size(amd64_binary.as_bytes());

        create_dir_all(&records)?;
        write_app_record(
            &records.join("pv-0.9.0-darwin-arm64.json"),
            "darwin-arm64",
            ARM64_OBJECT_KEY,
            &arm64_sha256,
            arm64_size,
        )?;
        write_app_record(
            &records.join("pv-0.9.0-darwin-amd64.json"),
            "darwin-amd64",
            AMD64_OBJECT_KEY,
            &amd64_sha256,
            amd64_size,
        )?;

        Ok(Self {
            tempdir,
            records,
            output,
            arm64_sha256,
            amd64_sha256,
        })
    }

    fn generate_installer(&self) -> Result<String> {
        let output = run_pv_release([
            "generate-app-installer",
            "--records",
            self.records.as_str(),
            "--output",
            self.output.as_str(),
            "--base-url",
            STAGING_BASE_URL,
        ])?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "generate-app-installer failed: {}",
                command_output_summary(&output)
            ));
        }

        read_file(&self.output)
    }
}

#[cfg(unix)]
struct InstallerExecutionFixture {
    app: AppInstallerFixture,
    home: Utf8PathBuf,
    fake_bin: Utf8PathBuf,
    download_source: Utf8PathBuf,
    amd64_download_source: Utf8PathBuf,
    curl_log: Utf8PathBuf,
    checksum_log: Utf8PathBuf,
    pv_log: Utf8PathBuf,
}

#[cfg(unix)]
impl InstallerExecutionFixture {
    fn new() -> Result<Self> {
        let app = AppInstallerFixture::new()?;
        let home = app.tempdir.path().join("home");
        let fake_bin = app.tempdir.path().join("fake-bin");
        let download_source = app.tempdir.path().join("fake-download/pv");
        let amd64_download_source = app.tempdir.path().join("fake-download/pv-amd64");
        let curl_log = app.tempdir.path().join("curl.log");
        let checksum_log = app.tempdir.path().join("checksum.log");
        let pv_log = app.tempdir.path().join("pv.log");

        create_dir_all(&home)?;
        create_dir_all(&fake_bin)?;
        create_dir_all(download_source.parent().context("download source parent")?)?;
        write_executable(&download_source, fake_pv_binary())?;
        write_executable(
            &amd64_download_source,
            "#!/bin/sh\nprintf '%s\\n' pv-amd64-fixture\n",
        )?;
        write_fake_curl(&fake_bin.join("curl"))?;
        write_fake_checksum_tool(&fake_bin.join("shasum"))?;
        write_fake_checksum_tool(&fake_bin.join("sha256sum"))?;
        write_fake_uname(&fake_bin.join("uname"))?;
        write_fake_sysctl(&fake_bin.join("sysctl"))?;

        Ok(Self {
            app,
            home,
            fake_bin,
            download_source,
            amd64_download_source,
            curl_log,
            checksum_log,
            pv_log,
        })
    }

    fn run_installer(&self, args: &[&str], checksum_mode: ChecksumMode) -> Result<Output> {
        self.run_installer_with_shell(args, checksum_mode, "/bin/pv-unsupported-shell")
    }

    fn run_installer_with_shell(
        &self,
        args: &[&str],
        checksum_mode: ChecksumMode,
        shell: &str,
    ) -> Result<Output> {
        self.run_installer_with_platform(args, checksum_mode, shell, "arm64", false)
    }

    fn run_installer_with_platform(
        &self,
        args: &[&str],
        checksum_mode: ChecksumMode,
        shell: &str,
        machine: &str,
        translated: bool,
    ) -> Result<Output> {
        let installer = self.app.generate_installer()?;
        write_executable(&self.app.output, &installer)?;

        let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.fake_bin);
        let mut command = StdCommand::new("/bin/bash");
        command
            .arg(&self.app.output)
            .args(args)
            .env("HOME", &self.home)
            .env("PATH", path)
            .env("SHELL", shell)
            .env("PV_TEST_CURL_LOG", &self.curl_log)
            .env("PV_TEST_CHECKSUM_LOG", &self.checksum_log)
            .env("PV_TEST_PV_LOG", &self.pv_log)
            .env("PV_TEST_DOWNLOAD_SOURCE", &self.download_source)
            .env("PV_TEST_AMD64_DOWNLOAD_SOURCE", &self.amd64_download_source)
            .env("PV_TEST_ARM64_SHA256", &self.app.arm64_sha256)
            .env("PV_TEST_AMD64_SHA256", &self.app.amd64_sha256)
            .env("PV_TEST_CHECKSUM_MODE", checksum_mode.as_env_value())
            .env("PV_TEST_UNAME_MACHINE", machine)
            .env(
                "PV_TEST_SYSCTL_TRANSLATED",
                if translated { "1" } else { "0" },
            );

        Ok(command.output()?)
    }

    fn home(&self) -> &Utf8Path {
        &self.home
    }

    fn curl_log(&self) -> &Utf8Path {
        &self.curl_log
    }

    fn pv_log(&self) -> Utf8PathBuf {
        self.pv_log.clone()
    }

    fn release_binary(&self) -> Utf8PathBuf {
        self.home.join(format!(".pv/bin/releases/{APP_VERSION}/pv"))
    }

    fn active_binary(&self) -> Utf8PathBuf {
        self.home.join(".pv/bin/pv")
    }

    fn command_logs(&self) -> Result<String> {
        let logs = format!(
            "curl:\n{}\n\nchecksum:\n{}\n\npv:\n{}",
            normalize_log_file(&self.curl_log, self)?,
            normalize_log_file(&self.checksum_log, self)?,
            normalize_log_file(&self.pv_log, self)?
        );
        Ok(logs)
    }
}

#[derive(Clone, Copy)]
enum ChecksumMode {
    Match,
    Mismatch,
}

impl ChecksumMode {
    fn as_env_value(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Mismatch => "mismatch",
        }
    }
}

fn installer_contract_summary(installer: &str) -> String {
    let fixture = ContractValues::new();
    format!(
        "\
version_present={}
staging_base_present={}
arm64_url_present={}
amd64_url_present={}
arm64_sha256_present={}
amd64_sha256_present={}
arm64_size_present={}
amd64_size_present={}
app_manifest_not_embedded={}
release_install_path_present={}
active_symlink_path_present={}
setup_invocation_present={}
no_setup_flag_present={}
no_path_flag_present={}
yes_flag_present={}
non_interactive_flag_present={}
pv_env_block_present={}
checksum_verification_present={}
curl_connect_timeout_present={}
curl_max_time_present={}
curl_retry_present={}",
        installer.contains(APP_VERSION),
        installer.contains(STAGING_BASE_URL),
        installer.contains(&fixture.arm64_url),
        installer.contains(&fixture.amd64_url),
        installer.contains(&fixture.arm64_sha256),
        installer.contains(&fixture.amd64_sha256),
        installer.contains(&fixture.arm64_size.to_string()),
        installer.contains(&fixture.amd64_size.to_string()),
        !installer.contains("pv-app-manifest.json"),
        installer.contains(".pv/bin/releases"),
        installer.contains(".pv/bin/pv"),
        installer.contains(" setup"),
        installer.contains("--no-setup"),
        installer.contains("--no-path"),
        installer.contains("--yes"),
        installer.contains("--non-interactive"),
        installer.contains("PV ENV"),
        installer.contains("checksum") || installer.contains("sha256"),
        installer.contains("--connect-timeout"),
        installer.contains("--max-time"),
        installer.contains("--retry"),
    )
}

struct ContractValues {
    arm64_url: String,
    amd64_url: String,
    arm64_sha256: String,
    amd64_sha256: String,
    arm64_size: u64,
    amd64_size: u64,
}

impl ContractValues {
    fn new() -> Self {
        let (arm64_sha256, arm64_size) = digest_and_size(fake_pv_binary().as_bytes());
        let (amd64_sha256, amd64_size) =
            digest_and_size(b"#!/bin/sh\nprintf '%s\\n' pv-amd64-fixture\n");

        Self {
            arm64_url: format!("{STAGING_BASE_URL}/{ARM64_OBJECT_KEY}"),
            amd64_url: format!("{STAGING_BASE_URL}/{AMD64_OBJECT_KEY}"),
            arm64_sha256,
            amd64_sha256,
            arm64_size,
            amd64_size,
        }
    }
}

fn write_app_record(
    path: &Utf8Path,
    platform: &str,
    object_key: &str,
    sha256: &str,
    size: u64,
) -> Result<()> {
    let record = json!({
        "schema_version": 1,
        "channel": "stable",
        "version": APP_VERSION,
        "minimum_pv_version": "0.1.0",
        "published_at": "2026-06-13T12:00:00Z",
        "platform": platform,
        "object_key": object_key,
        "sha256": sha256,
        "size": size,
        "provenance": {
            "source_url": "https://github.com/prvious/pv/archive/refs/tags/v0.9.0.tar.gz",
            "source_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "recipe": ".github/workflows/app-release.yml",
            "pv_commit": "0123456789abcdef0123456789abcdef01234567",
            "build_run_id": "installer-contract-test",
        },
    });
    let json = format!("{}\n", serde_json::to_string_pretty(&record)?);
    write_file(path, &json)
}

fn run_pv_release(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Output> {
    let mut command = StdCommand::new(env!("CARGO_BIN_EXE_pv-release"));
    for arg in args {
        command.arg(arg.as_ref());
    }
    Ok(command.output()?)
}

fn command_output_summary(output: &Output) -> String {
    format!(
        "code={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn fake_pv_binary() -> &'static str {
    r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >>"$PV_TEST_PV_LOG"
case "${1:-}" in
  setup)
    exit 0
    ;;
  env)
    printf '%s\n' 'export PATH="$HOME/.pv/bin:$PATH"'
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
"#
}

#[cfg(unix)]
fn write_fake_curl(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
url=
output=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output)
      shift
      output=${1:-}
      ;;
    -o*)
      output=${1#-o}
      ;;
    --output=*)
      output=${1#--output=}
      ;;
    -*)
      ;;
    *)
      url=$1
      ;;
  esac
  shift
done
{
  printf 'url=%s\n' "$url"
  printf 'output=%s\n' "$output"
} >>"$PV_TEST_CURL_LOG"
if [ -n "$output" ]; then
  mkdir -p "$(dirname "$output")"
  case "$url" in
    *darwin-amd64)
      cp "$PV_TEST_AMD64_DOWNLOAD_SOURCE" "$output"
      ;;
    *)
      cp "$PV_TEST_DOWNLOAD_SOURCE" "$output"
      ;;
  esac
else
  cat "$PV_TEST_DOWNLOAD_SOURCE"
fi
"#,
    )
}

#[cfg(unix)]
fn write_fake_checksum_tool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
target=
for arg in "$@"; do
  case "$arg" in
    -*)
      ;;
    *)
      target=$arg
      ;;
  esac
done
case "${PV_TEST_CHECKSUM_MODE:-match}" in
  mismatch)
    sha=0000000000000000000000000000000000000000000000000000000000000000
    ;;
  *)
    if grep -q pv-amd64-fixture "$target"; then
      sha=$PV_TEST_AMD64_SHA256
    else
      sha=$PV_TEST_ARM64_SHA256
    fi
    ;;
esac
printf '%s  %s\n' "$sha" "$target"
printf 'tool=%s target=%s sha=%s\n' "${0##*/}" "$target" "$sha" >>"$PV_TEST_CHECKSUM_LOG"
"#,
    )
}

#[cfg(unix)]
fn write_fake_uname(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
case "${1:-}" in
  -s)
    printf '%s\n' Darwin
    ;;
  -m)
    printf '%s\n' "${PV_TEST_UNAME_MACHINE:-arm64}"
    ;;
  *)
    printf '%s\n' Darwin
    ;;
esac
"#,
    )
}

#[cfg(unix)]
fn write_fake_sysctl(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
case "$*" in
  *sysctl.proc_translated*)
    printf '%s\n' "${PV_TEST_SYSCTL_TRANSLATED:-0}"
    ;;
  *)
    exit 1
    ;;
esac
"#,
    )
}

fn digest_and_size(bytes: &[u8]) -> (String, u64) {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    (HEXLOWER.encode(&hasher.finalize()), bytes.len() as u64)
}

fn normalize_log_file(path: &Utf8Path, fixture: &InstallerExecutionFixture) -> Result<String> {
    if !path_exists(path) {
        return Ok("<missing>".to_string());
    }

    let log = read_file(path)?;
    Ok(log
        .lines()
        .map(|line| normalize_log_line(line, fixture))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn normalize_output(output: &str, fixture: &InstallerExecutionFixture) -> String {
    output
        .replace(&fixture.app.arm64_sha256, "<arm64-sha256>")
        .replace(&fixture.app.amd64_sha256, "<amd64-sha256>")
        .replace(fixture.app.tempdir.path().as_str(), "<tmp>")
}

#[cfg(unix)]
fn normalize_log_line(line: &str, fixture: &InstallerExecutionFixture) -> String {
    let line = normalize_output(line, fixture);
    if line.starts_with("output=") {
        return "output=<download>".to_string();
    }

    if line.starts_with("tool=") && line.contains(" target=") {
        let checksum_tool = line
            .split_once(' ')
            .map(|(_, rest)| rest)
            .unwrap_or(line.as_str());
        let Some((_, sha)) = checksum_tool.split_once(" sha=") else {
            return line;
        };
        return format!("tool=<checksum-tool> target=<download> sha={sha}");
    }

    line
}

#[cfg(unix)]
fn checksum_mismatch_summary(
    output: &Output,
    fixture: &InstallerExecutionFixture,
) -> Result<String> {
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    Ok(format!(
        "status_success={}\nstderr_mentions_checksum={}\nlogs:\n{}",
        output.status.success(),
        stderr.contains("checksum") || stderr.contains("sha256"),
        fixture.command_logs()?
    ))
}

#[cfg(unix)]
fn non_interactive_shell_profile_confirmation_summary(
    output: &Output,
    fixture: &InstallerExecutionFixture,
) -> Result<String> {
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    Ok(format!(
        "status_success={}\nstderr_mentions_shell_profile_confirmation={}\nlogs:\n{}",
        output.status.success(),
        stderr.contains("shell profile") && stderr.contains("confirmation"),
        fixture.command_logs()?
    ))
}

#[cfg(unix)]
fn incomplete_profile_block_summary(
    output: &Output,
    fixture: &InstallerExecutionFixture,
    profile: &Utf8Path,
    original_profile: &str,
) -> Result<String> {
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    Ok(format!(
        "status_success={}\nstderr_mentions_incomplete_profile={}\nprofile_unchanged={}\nlogs:\n{}",
        output.status.success(),
        stderr.contains("incomplete pv env block")
            && stderr.contains("leaving shell profile unchanged"),
        read_file(profile)? == original_profile,
        fixture.command_logs()?
    ))
}

#[cfg(unix)]
fn active_pv_symlink_points_to_release(active: &Utf8Path, release: &Utf8Path) -> Result<bool> {
    if !is_symlink(active)? {
        return Ok(false);
    }

    let Some(active_parent) = active.parent() else {
        return Ok(false);
    };
    let target = read_link(active)?;
    let resolved = if target.is_absolute() {
        target
    } else {
        active_parent.join(target)
    };

    Ok(resolved == release)
}

#[cfg(unix)]
fn assert_download_outputs_were_removed(curl_log: &Utf8Path) -> Result<()> {
    let log = read_file(curl_log)?;
    for line in log.lines() {
        let Some(output) = line.strip_prefix("output=") else {
            continue;
        };
        if output.is_empty() {
            return Err(anyhow::anyhow!(
                "fake curl was not given an output path, so cleanup cannot be verified"
            ));
        }
        if path_exists(Utf8Path::new(output)) {
            return Err(anyhow::anyhow!(
                "bad download was left behind at `{output}`"
            ));
        }
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "installer contract tests create local fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "installer contract tests write local fixture files"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "installer contract tests create executable shell fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "installer contract tests read generated files and command logs"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(fs::read_to_string(path)?)
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "installer contract tests inspect the generated active symlink"
)]
fn is_symlink(path: &Utf8Path) -> Result<bool> {
    Ok(fs::symlink_metadata(path)?.file_type().is_symlink())
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "installer contract tests inspect the generated active symlink target"
)]
fn read_link(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let target = fs::read_link(path)?;
    Utf8PathBuf::from_path_buf(target)
        .map_err(|path| anyhow::anyhow!("symlink target is not UTF-8: {path:?}"))
}
