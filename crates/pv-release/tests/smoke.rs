use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use data_encoding::HEXLOWER;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::assert_debug_snapshot;
use pv_release::ReleaseError;
use pv_release::smoke::run_smoke_hook;
use serde_json::Value;
use sha2::{Digest, Sha256};

use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Output;
use tar::{Builder, Header};

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests execute smoke hook fixtures directly"
)]
type StdCommand = std::process::Command;

#[test]
fn smoke_hook_reports_success_and_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let success = tempdir.path().join("success.sh");
    let failure = tempdir.path().join("failure.sh");
    write_executable(&success, "#!/bin/sh\nexit 0\n")?;
    write_executable(&failure, "#!/bin/sh\nexit 42\n")?;

    assert_debug_snapshot!((
        summarize_result(run_smoke_hook(&success, tempdir.path())),
        summarize_result(run_smoke_hook(&failure, tempdir.path())),
    ));

    Ok(())
}

#[test]
fn php_smoke_validates_frankenphp_when_cli_binary_is_also_present() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");
    let frankenphp_log = tempdir.path().join("frankenphp.log");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_file(&frankenphp_log, "")?;
    write_executable(
        &artifact_bin.join("php"),
        r#"#!/bin/sh
set -eu
case "$1" in
  -v) printf '%s\n' 'PHP 8.4.20 (cli)' ;;
  -m) printf '%s\n' 'json' ;;
  *) exit 99 ;;
esac
"#,
    )?;
    write_executable(
        &artifact_bin.join("frankenphp"),
        r#"#!/bin/sh
set -eu
case "${1:-}" in
  php-cli)
    case "${2:-}" in
      -v)
        printf '%s\n' 'php-cli -v' >>"$PV_FRANKENPHP_LOG"
        printf '%s\n' 'PHP 8.4.20 (cli)'
        ;;
      -m)
        printf '%s\n' 'php-cli -m' >>"$PV_FRANKENPHP_LOG"
        printf '%s\n' 'json'
        ;;
      *) exit 99 ;;
    esac
    ;;
  php-server)
    shift
    listen=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --listen)
          shift
          listen=${1:-}
          ;;
      esac
      shift
    done
    [ "$listen" != "127.0.0.1:48123" ] || exit 70
    printf 'php-server %s\n' "$listen" >>"$PV_FRANKENPHP_LOG"
    exec sleep 60
    ;;
  *) exit 99 ;;
esac
"#,
    )?;
    write_executable(
        &command_bin.join("curl"),
        r#"#!/bin/sh
set -eu
printf '%s\n' 'pv-frankenphp-ok'
"#,
    )?;

    let smoke_hook = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/php/smoke.sh");
    let status = StdCommand::new(smoke_hook)
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .env("PV_EXPECTED_EXTENSIONS", "json")
        .env("PV_FRANKENPHP_LOG", &frankenphp_log)
        .env("PV_UPSTREAM_VERSION", "8.4.20-frankenphp1.12.3")
        .status()?;

    assert!(status.success(), "smoke hook exited with {status}");
    let frankenphp_log = read_file(&frankenphp_log)?;
    assert!(frankenphp_log.starts_with("php-cli -v\nphp-cli -m\nphp-server 127.0.0.1:"));
    assert!(
        !frankenphp_log.contains("php-server 127.0.0.1:48123\n"),
        "smoke hook should not use the old fixed loopback port: {frankenphp_log}"
    );

    Ok(())
}

#[test]
fn php_smoke_normalizes_realistic_module_output() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");

    create_dir_all(&artifact_bin)?;
    write_executable(
        &artifact_bin.join("php"),
        r#"#!/bin/sh
set -eu
case "$1" in
  -v) printf '%s\n' 'PHP 8.4.20 (cli)' ;;
  -m)
    printf '%s\n' \
      '[PHP Modules]' \
      'Core' \
      'date' \
      'PDO' \
      'Phar' \
      'SimpleXML' \
      'SPL' \
      'Reflection' \
      'standard' \
      'json' \
      '[Zend Modules]'
    ;;
  *) exit 99 ;;
esac
"#,
    )?;

    let smoke_hook = php_smoke_hook();
    let output = StdCommand::new(smoke_hook)
        .arg(&artifact_root)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("PV_EXPECTED_EXTENSIONS", "json,pdo,phar,simplexml")
        .env("PV_UPSTREAM_VERSION", "8.4.20")
        .output()?;

    assert!(
        output.status.success(),
        "smoke hook failed: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn php_smoke_rejects_unexpected_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");

    create_dir_all(&artifact_bin)?;
    write_executable(
        &artifact_bin.join("php"),
        r#"#!/bin/sh
set -eu
case "$1" in
  -v) printf '%s\n' 'PHP 8.4.20 (cli)' ;;
  -m) printf '%s\n' 'json' 'xdebug' ;;
  *) exit 99 ;;
esac
"#,
    )?;

    let smoke_hook = php_smoke_hook();
    let output = StdCommand::new(smoke_hook)
        .arg(&artifact_root)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("PV_EXPECTED_EXTENSIONS", "json")
        .env("PV_UPSTREAM_VERSION", "8.4.20")
        .output()?;

    assert_debug_snapshot!(command_output_summary(&output));

    Ok(())
}

#[test]
fn php_pair_build_smoke_builds_cli_and_frankenphp_from_one_staticphp_buildroot() -> Result<()> {
    let run = run_php_build_recipe_smoke()?;
    let php_source_dir = format!("{}/sources/php-8.4.20-source/php-source", run.out_dir);
    let frankenphp_source_dir = format!(
        "{}/sources/frankenphp-8.4.20-frankenphp1.12.3-pv1-source/frankenphp-source",
        run.out_dir
    );
    let expected_log = format!(
        "pwd={}/work/php-pair-8.4-darwin-arm64/staticphp\n\
argv=[build:php][json][--build-cli][--build-frankenphp][--enable-zts][--dl-with-php=8.4.20][--dl-custom-local][php-src:{php_source_dir}][--dl-custom-local][frankenphp:{frankenphp_source_dir}]\n",
        run.out_dir
    );

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert_eq!(run.spc_log, expected_log);
    let expected_curl_log = format!(
        "argv=[-L][--fail][--show-error][--silent][--retry][3][--retry-delay][2][--retry-all-errors][--connect-timeout][20][--max-time][600][https://sources.example.test/php.tar.gz][-o][{}/sources/php-8.4.20-source.tar.gz]\n\
argv=[-L][--fail][--show-error][--silent][--retry][3][--retry-delay][2][--retry-all-errors][--connect-timeout][20][--max-time][600][https://sources.example.test/frankenphp.tar.gz][-o][{}/sources/frankenphp-8.4.20-frankenphp1.12.3-pv1-source.tar.gz]\n",
        run.out_dir, run.out_dir
    );
    assert_eq!(run.curl_log, expected_curl_log);
    assert!(run.php_record_json.is_some(), "PHP record was not written");
    assert!(run.php_notice.is_some(), "PHP NOTICE was not written");
    assert!(
        run.frankenphp_record_json.is_some(),
        "FrankenPHP record was not written"
    );
    assert!(run.php_archive_exists, "PHP archive was not written");
    assert!(
        run.frankenphp_archive_exists,
        "FrankenPHP archive was not written"
    );
    assert_debug_snapshot!(build_recipe_record_provenance(
        run.php_record_json.as_deref()
    )?);
    assert_debug_snapshot!(build_recipe_record_provenance(
        run.frankenphp_record_json.as_deref()
    )?);
    assert_debug_snapshot!(build_recipe_notice_source_lines(
        run.frankenphp_notice.as_deref()
    )?);

    Ok(())
}

#[test]
fn php_build_smoke_rejects_unexpected_macho_architecture() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        lipo_archs: "x86_64",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_pair_build_smoke_passes_per_resource_metadata_to_archive_validation() -> Result<()> {
    let run = run_php_build_recipe_smoke()?;

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert_eq!(
        run.validate_log,
        "archive=php-8.4.20-pv1-darwin-arm64.tar.gz record=php-8.4.20-pv1-darwin-arm64.json upstream=8.4.20 php=8.4.20 expected=json deployment=13.0\n\
archive=frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64.tar.gz record=frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64.json upstream=8.4.20-frankenphp1.12.3 php=8.4.20 expected=json deployment=13.0\n"
    );

    Ok(())
}

#[test]
fn php_pair_build_smoke_rejects_frankenphp_archive_validation_without_final_outputs() -> Result<()>
{
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        validate_archive_failure_resource: "frankenphp",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.php_record_json.is_none(),
        "PHP record should not be written before the FrankenPHP archive is valid"
    );
    assert!(
        run.frankenphp_record_json.is_none(),
        "FrankenPHP record should not be written when its archive is invalid"
    );
    assert!(
        !run.php_archive_exists,
        "PHP archive should not be written before the FrankenPHP archive is valid"
    );
    assert!(
        !run.frankenphp_archive_exists,
        "FrankenPHP archive should not be written when its archive is invalid"
    );
    assert_eq!(
        String::from_utf8_lossy(&run.output.stdout),
        "",
        "archive paths should not be printed before both archives are valid"
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_build_smoke_rejects_newer_macho_minimum_os() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        macho_minos: "14.0",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_build_smoke_rejects_homebrew_linked_dylib() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        macho_libraries: "\t/opt/homebrew/opt/icu4c/lib/libicuuc.74.dylib (compatibility version 74.0.0, current version 74.2.0)",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_build_smoke_rejects_usr_local_linked_dylib() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        macho_libraries: "\t/usr/local/lib/libfoo.dylib (compatibility version 1.0.0, current version 1.0.0)",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_pair_build_smoke_rejects_homebrew_rpath_on_frankenphp_binary() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        frankenphp_macho_libraries: "\t@rpath/libphp.dylib (compatibility version 1.0.0, current version 1.0.0)",
        frankenphp_macho_rpaths: "/usr/local/opt/openssl@3/lib",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.php_record_json.is_none(),
        "PHP record should not be written before the FrankenPHP binary is valid"
    );
    assert!(
        !run.php_archive_exists,
        "PHP archive should not be written before the FrankenPHP binary is valid"
    );
    assert_eq!(
        run.validate_log, "",
        "archive validation should not run before both pair binaries are valid"
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_pair_build_smoke_rejects_runner_rpath_on_frankenphp_binary() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        frankenphp_macho_libraries: "\t@rpath/libphp.dylib (compatibility version 1.0.0, current version 1.0.0)",
        frankenphp_macho_rpaths: "/Users/runner/hostedtoolcache/php/lib",
        ..default_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(build_recipe_output_summary(&run));

    Ok(())
}

#[test]
fn php_build_smoke_accepts_system_and_relative_macho_runtime_metadata() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        macho_libraries: "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)\n\t/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 2503.1.0)\n\t@rpath/libphp.dylib (compatibility version 1.0.0, current version 1.0.0)\n\t@loader_path/../lib/libz.dylib (compatibility version 1.0.0, current version 1.3.1)",
        macho_rpaths: "@loader_path/../lib\n@executable_path/../lib",
        ..default_build_recipe_options()
    })?;

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );

    Ok(())
}

fn summarize_result(result: pv_release::Result<()>) -> Result<(), ErrorSummary> {
    result.map_err(ErrorSummary::from)
}

struct BuildRecipeRun {
    out_dir: String,
    output: Output,
    php_record_json: Option<String>,
    frankenphp_record_json: Option<String>,
    php_notice: Option<String>,
    frankenphp_notice: Option<String>,
    spc_log: String,
    curl_log: String,
    validate_log: String,
    php_archive_exists: bool,
    frankenphp_archive_exists: bool,
}

struct BuildRecipeOptions<'a> {
    lipo_archs: &'a str,
    macho_minos: &'a str,
    macho_libraries: &'a str,
    macho_rpaths: &'a str,
    frankenphp_macho_libraries: &'a str,
    frankenphp_macho_rpaths: &'a str,
    validate_archive_failure_resource: &'a str,
}

fn php_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/php/smoke.sh")
}

fn run_php_build_recipe_smoke() -> Result<BuildRecipeRun> {
    run_php_build_recipe_smoke_with_options(default_build_recipe_options())
}

fn default_build_recipe_options() -> BuildRecipeOptions<'static> {
    BuildRecipeOptions {
        lipo_archs: "arm64",
        macho_minos: "13.0",
        macho_libraries: "",
        macho_rpaths: "",
        frankenphp_macho_libraries: "",
        frankenphp_macho_rpaths: "",
        validate_archive_failure_resource: "",
    }
}

fn run_php_build_recipe_smoke_with_options(
    options: BuildRecipeOptions<'_>,
) -> Result<BuildRecipeRun> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let out_dir = tempdir.path().join("out");
    let record_dir = tempdir.path().join("records");
    let source_archive = tempdir.path().join("source.tar.gz");
    let php_source_archive = tempdir.path().join("php-source.tar.gz");
    let curl_log = tempdir.path().join("curl.log");
    let spc_log = tempdir.path().join("spc.log");
    let validate_log = tempdir.path().join("validate.log");

    create_dir_all(&fake_bin)?;
    write_source_archive(&source_archive, "frankenphp-source")?;
    write_source_archive(&php_source_archive, "php-source")?;
    write_fake_cargo(&fake_bin.join("cargo"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_fake_lipo(&fake_bin.join("lipo"))?;
    write_fake_otool(&fake_bin.join("otool"))?;
    write_fake_spc(&fake_bin.join("spc"))?;
    write_file(&curl_log, "")?;
    write_file(&spc_log, "")?;
    write_file(&validate_log, "")?;

    let build_script = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/php/build.sh");
    let mut command = StdCommand::new(build_script);
    command
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_ARTIFACT_OUT_DIR", &out_dir)
        .env("PV_ARTIFACT_RECORD_DIR", &record_dir)
        .env("PV_BUILD_RUN_ID", "local-test")
        .env("PV_COMMIT", "0123456789abcdef0123456789abcdef01234567")
        .env("PV_RECIPE_PLATFORM", "darwin-arm64")
        .env("PV_RECIPE_TRACK", "8.4")
        .env(
            "PV_TEST_FRANKENPHP_MACHO_LIBRARIES",
            options.frankenphp_macho_libraries,
        )
        .env(
            "PV_TEST_FRANKENPHP_MACHO_RPATHS",
            options.frankenphp_macho_rpaths,
        )
        .env("PV_TEST_LIPO_ARCHS", options.lipo_archs)
        .env("PV_TEST_MACHO_LIBRARIES", options.macho_libraries)
        .env("PV_TEST_MACHO_MINOS", options.macho_minos)
        .env("PV_TEST_MACHO_RPATHS", options.macho_rpaths)
        .env("PV_TEST_CURL_LOG", &curl_log)
        .env("PV_TEST_PHP_SOURCE_ARCHIVE", &php_source_archive)
        .env(
            "PV_TEST_PHP_SOURCE_SHA256",
            file_sha256(&php_source_archive)?,
        )
        .env("PV_TEST_SOURCE_ARCHIVE", &source_archive)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_archive)?)
        .env(
            "PV_TEST_VALIDATE_ARCHIVE_FAILURE_RESOURCE",
            options.validate_archive_failure_resource,
        )
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .env("PV_TEST_SPC_LOG", &spc_log);
    let output = command.output()?;

    let php_artifact_version = "8.4.20-pv1";
    let php_artifact_basename = "php-8.4.20-pv1-darwin-arm64";
    let php_archive = out_dir.join(format!("{php_artifact_basename}.tar.gz"));
    let php_record = record_dir
        .join("php")
        .join("8.4")
        .join(php_artifact_version)
        .join("darwin-arm64")
        .join(format!("{php_artifact_basename}.json"));
    let php_notice = out_dir
        .join("work")
        .join("php-pair-8.4-darwin-arm64")
        .join(php_artifact_basename)
        .join("NOTICE");

    let frankenphp_artifact_version = "8.4.20-frankenphp1.12.3-pv1";
    let frankenphp_artifact_basename = "frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64";
    let frankenphp_archive = out_dir.join(format!("{frankenphp_artifact_basename}.tar.gz"));
    let frankenphp_record = record_dir
        .join("frankenphp")
        .join("8.4")
        .join(frankenphp_artifact_version)
        .join("darwin-arm64")
        .join(format!("{frankenphp_artifact_basename}.json"));
    let frankenphp_notice = out_dir
        .join("work")
        .join("php-pair-8.4-darwin-arm64")
        .join(frankenphp_artifact_basename)
        .join("NOTICE");

    let php_record_json = read_optional_file(&php_record)?;
    let frankenphp_record_json = read_optional_file(&frankenphp_record)?;
    let php_notice = read_optional_file(&php_notice)?;
    let frankenphp_notice = read_optional_file(&frankenphp_notice)?;

    Ok(BuildRecipeRun {
        out_dir: out_dir.to_string(),
        output,
        php_record_json,
        frankenphp_record_json,
        php_notice,
        frankenphp_notice,
        spc_log: read_file(&spc_log)?,
        curl_log: read_file(&curl_log)?,
        validate_log: read_file(&validate_log)?,
        php_archive_exists: path_exists(&php_archive),
        frankenphp_archive_exists: path_exists(&frankenphp_archive),
    })
}

fn command_output_summary(output: &Output) -> (bool, Option<i32>, String, String) {
    (
        output.status.success(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn build_recipe_output_summary(run: &BuildRecipeRun) -> (bool, Option<i32>, String, String) {
    let (success, code, stdout, stderr) = command_output_summary(&run.output);
    (
        success,
        code,
        stdout.replace(&run.out_dir, "<out>"),
        stderr.replace(&run.out_dir, "<out>"),
    )
}

fn build_recipe_record_provenance(record_json: Option<&str>) -> Result<Value> {
    let record_json =
        record_json.ok_or_else(|| anyhow::anyhow!("build recipe did not produce a record"))?;
    let record: Value = serde_json::from_str(record_json)?;
    record
        .get("provenance")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("build recipe record did not contain provenance"))
}

fn build_recipe_notice_source_lines(notice: Option<&str>) -> Result<Vec<&str>> {
    let notice = notice.ok_or_else(|| anyhow::anyhow!("build recipe did not produce NOTICE"))?;
    Ok(notice
        .lines()
        .filter(|line| line.contains("source"))
        .collect())
}

fn command_output_debug(output: &Output) -> String {
    format!("{:#?}", command_output_summary(output))
}

fn write_fake_cargo(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

if [ "$#" -ge 5 ] && [ "$1" = "run" ] && [ "$2" = "-p" ] && [ "$3" = "pv-release" ] && [ "$4" = "--" ]; then
  case "$5" in
    print-recipe-env)
      resource=
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --resource)
            shift
            resource=${1:-}
            ;;
        esac
        shift
      done
      case "$resource" in
        php)
          upstream_version=8.4.20
          artifact_version=8.4.20-pv1
          source_url=https://sources.example.test/php.tar.gz
          source_sha256=$PV_TEST_PHP_SOURCE_SHA256
          php_source_env=
          ;;
        frankenphp)
          upstream_version=8.4.20-frankenphp1.12.3
          artifact_version=8.4.20-frankenphp1.12.3-pv1
          source_url=https://sources.example.test/frankenphp.tar.gz
          source_sha256=$PV_TEST_SOURCE_SHA256
          php_source_env="PV_PHP_SOURCE_URL=https://sources.example.test/php.tar.gz
PV_PHP_SOURCE_SHA256=$PV_TEST_PHP_SOURCE_SHA256"
          ;;
        *) exit 77 ;;
      esac
      cat <<EOF
PV_RESOURCE=$resource
PV_TRACK=$PV_RECIPE_TRACK
PV_PLATFORM=$PV_RECIPE_PLATFORM
PV_PHP_VERSION=8.4.20
PV_UPSTREAM_VERSION=$upstream_version
PV_ARTIFACT_VERSION=$artifact_version
PV_SOURCE_URL=$source_url
PV_SOURCE_SHA256=$source_sha256
$php_source_env
PV_EXPECTED_EXTENSIONS=json
PV_BUILD_EXTENSIONS=json
PV_DEPLOYMENT_TARGET=13.0
PV_PV_BUILD_REVISION=pv1
PV_MINIMUM_PV_VERSION=0.1.0
EOF
      ;;
    validate-archive)
      archive=
      record=
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --archive)
            shift
            archive=${1:-}
            ;;
          --record)
            shift
            record=${1:-}
            ;;
        esac
        shift
      done
      archive_basename=${archive##*/}
      record_basename=${record##*/}
      printf 'archive=%s record=%s upstream=%s php=%s expected=%s deployment=%s\n' \
        "$archive_basename" \
        "$record_basename" \
        "$PV_UPSTREAM_VERSION" \
        "$PV_PHP_VERSION" \
        "$PV_EXPECTED_EXTENSIONS" \
        "$PV_DEPLOYMENT_TARGET" >>"$PV_TEST_VALIDATE_LOG"
      case "$archive_basename" in
        "$PV_TEST_VALIDATE_ARCHIVE_FAILURE_RESOURCE"-*)
          printf 'validate-archive failed for %s\n' "$archive_basename" >&2
          exit 79
          ;;
      esac
      exit 0
      ;;
    *) exit 77 ;;
  esac
else
  exit 77
fi
"#,
    )
}

fn write_fake_curl(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

output=
url=
printf 'argv=' >>"$PV_TEST_CURL_LOG"
for arg in "$@"; do
  printf '[%s]' "$arg" >>"$PV_TEST_CURL_LOG"
done
printf '\n' >>"$PV_TEST_CURL_LOG"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      shift
      output=$1
      ;;
    -*)
      ;;
    *)
      url=$1
      ;;
  esac
  shift
done

[ -n "$output" ] || exit 78
case "$url" in
  https://sources.example.test/php.tar.gz)
    cp "$PV_TEST_PHP_SOURCE_ARCHIVE" "$output"
    ;;
  *)
    cp "$PV_TEST_SOURCE_ARCHIVE" "$output"
    ;;
esac
"#,
    )
}

fn write_fake_lipo(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

[ "${1:-}" = "-archs" ] || exit 78
printf '%s\n' "$PV_TEST_LIPO_ARCHS"
"#,
    )
}

fn write_fake_otool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

binary=${2:-}
macho_libraries=${PV_TEST_MACHO_LIBRARIES:-}
macho_rpaths=${PV_TEST_MACHO_RPATHS:-}
case "$binary" in
  */bin/frankenphp)
    macho_libraries=${PV_TEST_FRANKENPHP_MACHO_LIBRARIES:-$macho_libraries}
    macho_rpaths=${PV_TEST_FRANKENPHP_MACHO_RPATHS:-$macho_rpaths}
    ;;
esac

case "${1:-}" in
  -L)
    printf '%s:\n' "$binary"
    if [ -n "$macho_libraries" ]; then
      printf '%s\n' "$macho_libraries"
    fi
    ;;
  -l)
    cat <<EOF
Load command 1
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform MACOS
    minos $PV_TEST_MACHO_MINOS
      sdk 15.0
EOF
    if [ -n "$macho_rpaths" ]; then
      load_command=2
      printf '%s\n' "$macho_rpaths" | while IFS= read -r macho_rpath; do
        cat <<EOF
Load command $load_command
          cmd LC_RPATH
      cmdsize 32
         path $macho_rpath (offset 12)
EOF
        load_command=$((load_command + 1))
      done
    fi
    ;;
  *)
    exit 78
    ;;
esac
"#,
    )
}

fn write_fake_spc(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

printf 'pwd=%s\n' "$(pwd)" >>"$PV_TEST_SPC_LOG"
printf 'argv=' >>"$PV_TEST_SPC_LOG"
for arg in "$@"; do
  printf '[%s]' "$arg" >>"$PV_TEST_SPC_LOG"
done
printf '\n' >>"$PV_TEST_SPC_LOG"

[ "${1:-}" = "build:php" ] || {
  printf 'unexpected spc command: %s\n' "${1:-}" >&2
  exit 78
}

mkdir -p buildroot/bin
built_target=
case " $* " in
  *" --build-cli "*)
    printf '%s\n' '#!/bin/sh' >buildroot/bin/php
    built_target=1
    ;;
esac
case " $* " in
  *" --build-frankenphp "*)
    printf '%s\n' '#!/bin/sh' >buildroot/bin/frankenphp
    built_target=1
    ;;
esac
[ -n "$built_target" ] || {
  printf '%s\n' 'missing StaticPHP build target flag' >&2
  exit 78
}
"#,
    )
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create source tarball fixtures directly"
)]
fn write_source_archive(path: &Utf8Path, top_level_dir: &str) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    let content = b"source fixture";
    let mut header = Header::new_gnu();
    header.set_path(format!("{top_level_dir}/README.md"))?;
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, content as &[u8])?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read local source archive fixtures to seed matching checksums"
)]
fn file_sha256(path: &Utf8Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);

    Ok(HEXLOWER.encode(&hasher.finalize()))
}

#[derive(Debug, PartialEq, Eq)]
struct ErrorSummary {
    kind: &'static str,
    path: String,
    reason: String,
}

impl From<ReleaseError> for ErrorSummary {
    fn from(error: ReleaseError) -> Self {
        match error {
            ReleaseError::SmokeHookFailed { hook, status } => Self {
                kind: "SmokeHookFailed",
                path: file_name(&hook),
                reason: status,
            },
            error => Self {
                kind: "Other",
                path: String::new(),
                reason: error.to_string(),
            },
        }
    }
}

fn file_name(path: &str) -> String {
    match Utf8Path::new(path).file_name() {
        Some(file_name) => file_name.to_string(),
        None => path.to_string(),
    }
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create executable smoke hook fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create executable smoke hook fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create smoke hook fixture directories"
)]
fn create_dir_all(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create and inspect smoke hook fixtures"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create and inspect smoke hook fixtures"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests inspect optional smoke hook outputs"
)]
fn read_optional_file(path: &Utf8Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}
