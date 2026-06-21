use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use data_encoding::HEXLOWER;
use flate2::Compression;
use flate2::write::GzEncoder;
use insta::assert_debug_snapshot;
use pv_release::ReleaseError;
use pv_release::smoke::{run_smoke_hook, run_smoke_hook_with_timeout};
use serde_json::Value;
use sha2::{Digest, Sha256};

use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Output;
use std::time::{Duration, Instant};
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
fn smoke_hook_times_out_and_kills_child() -> Result<()> {
    let tempdir = tempdir()?;
    let hook = tempdir.path().join("timeout.sh");
    write_executable(
        &hook,
        r#"#!/bin/sh
while :; do
  :
done
"#,
    )?;

    let started = Instant::now();
    let result = run_smoke_hook_with_timeout(&hook, tempdir.path(), Duration::from_millis(100));

    assert_debug_snapshot!(summarize_result(result));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "timeout smoke hook should not wait for the script forever"
    );
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
    [ "${2:-}" = "-r" ] || exit 99
    code=${3:-}
    if [ "$code" = 'printf("PHP %s\n", PHP_VERSION);' ]; then
      printf '%s\n' 'php-cli -r version' >>"$PV_FRANKENPHP_LOG"
      printf '%s\n' 'PHP 8.4.20'
    elif [ "$code" = 'foreach (get_loaded_extensions() as $extension) { echo $extension, PHP_EOL; }' ]; then
      printf '%s\n' 'php-cli -r extensions' >>"$PV_FRANKENPHP_LOG"
      printf '%s\n' 'json'
    else
      exit 99
    fi
    ;;
  php-server)
    shift
    listen=
    root=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --listen)
          shift
          listen=${1:-}
          ;;
        --root)
          shift
          root=${1:-}
          ;;
      esac
      shift
    done
    [ "$listen" != "127.0.0.1:48123" ] || exit 70
    phpinfo_state=missing-phpinfo
    if [ -n "$root" ] && grep -F 'phpinfo(INFO_CONFIGURATION);' "$root/index.php" >/dev/null; then
      phpinfo_state=phpinfo
    fi
    printf 'php-server %s %s\n' "$listen" "$phpinfo_state" >>"$PV_FRANKENPHP_LOG"
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
i=0
while [ "$i" -lt 5 ]; do
  if grep -F 'php-server 127.0.0.1:' "$PV_FRANKENPHP_LOG" >/dev/null; then
    printf '%s\n' 'pv-frankenphp-ok'
    printf '%s\n' 'Configuration File (php.ini) Path => /var/empty/com.prvious.pv/php'
    exit 0
  fi
  i=$((i + 1))
  sleep 1
done
exit 28
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
    assert!(
        frankenphp_log
            .starts_with("php-cli -r version\nphp-cli -r extensions\nphp-server 127.0.0.1:")
    );
    assert!(
        frankenphp_log.contains(" phpinfo\n"),
        "smoke hook should serve phpinfo(INFO_CONFIGURATION): {frankenphp_log}"
    );
    assert!(
        !frankenphp_log.contains("php-server 127.0.0.1:48123 "),
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
  --ini) printf '%s\n' 'Configuration File (php.ini) Path: /var/empty/com.prvious.pv/php' ;;
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
fn php_smoke_allows_extra_extensions() -> Result<()> {
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
  --ini) printf '%s\n' 'Configuration File (php.ini) Path: /var/empty/com.prvious.pv/php' ;;
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

    assert!(
        output.status.success(),
        "smoke hook failed: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn php_smoke_rejects_usr_local_ini_path_from_php_ini_output() -> Result<()> {
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
  -m) printf '%s\n' 'json' ;;
  --ini) printf '%s\n' 'Configuration File (php.ini) Path: /usr/local/etc/php' ;;
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

    assert!(
        !output.status.success(),
        "smoke hook should reject unsafe PHP ini fallback: {}",
        command_output_debug(&output)
    );
    assert_eq!(output.status.code(), Some(46));

    Ok(())
}

#[test]
fn php_smoke_rejects_usr_local_ini_path_from_frankenphp_response() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_executable(
        &artifact_bin.join("frankenphp"),
        r#"#!/bin/sh
set -eu
case "${1:-}" in
  php-cli)
    [ "${2:-}" = "-r" ] || exit 99
    code=${3:-}
    if [ "$code" = 'printf("PHP %s\n", PHP_VERSION);' ]; then
      printf '%s\n' 'PHP 8.4.20'
    elif [ "$code" = 'foreach (get_loaded_extensions() as $extension) { echo $extension, PHP_EOL; }' ]; then
      printf '%s\n' 'json'
    else
      exit 99
    fi
    ;;
  php-server)
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
printf '%s\n' 'Loaded Configuration File => /usr/local/etc/php/php.ini'
"#,
    )?;

    let smoke_hook = php_smoke_hook();
    let output = StdCommand::new(smoke_hook)
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .env("PV_EXPECTED_EXTENSIONS", "json")
        .env("PV_UPSTREAM_VERSION", "8.4.20-frankenphp1.12.3")
        .output()?;

    assert!(
        !output.status.success(),
        "smoke hook should reject unsafe FrankenPHP ini fallback: {}",
        command_output_debug(&output)
    );
    assert_eq!(output.status.code(), Some(46));

    Ok(())
}

#[test]
fn composer_smoke_requires_php_binary() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");

    create_dir_all(&artifact_root)?;
    write_file(&artifact_root.join("composer.phar"), "composer fixture\n")?;

    let output = StdCommand::new(composer_smoke_hook())
        .arg(&artifact_root)
        .env_remove("PV_COMPOSER_SMOKE_PHP")
        .env("PV_UPSTREAM_VERSION", "2.10.1")
        .output()?;

    assert!(
        !output.status.success(),
        "Composer smoke should fail without a PHP binary: {}",
        command_output_debug(&output)
    );
    assert_eq!(output.status.code(), Some(42));

    Ok(())
}

#[test]
fn composer_smoke_rejects_prefix_version_match() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let command_bin = tempdir.path().join("commands");
    let php = command_bin.join("php");

    create_dir_all(&artifact_root)?;
    create_dir_all(&command_bin)?;
    write_file(&artifact_root.join("composer.phar"), "composer fixture\n")?;
    write_executable(
        &php,
        r#"#!/bin/sh
set -eu
printf '%s\n' 'Composer version 2.10.10 2026-01-01 00:00:00'
"#,
    )?;

    let output = StdCommand::new(composer_smoke_hook())
        .arg(&artifact_root)
        .env("PV_COMPOSER_SMOKE_PHP", &php)
        .env("PV_UPSTREAM_VERSION", "2.10.1")
        .output()?;

    assert!(
        !output.status.success(),
        "Composer smoke should reject prefix version matches: {}",
        command_output_debug(&output)
    );
    assert_eq!(output.status.code(), Some(43));

    Ok(())
}

#[test]
fn php_build_recipe_smoke() -> Result<()> {
    let run = run_php_build_recipe_smoke()?;
    let php_source_dir = format!("{}/sources/php-8.4.20-source/php-source", run.out_dir);
    let frankenphp_source_dir = format!(
        "{}/sources/frankenphp-8.4.20-frankenphp1.12.3-pv1-source/frankenphp-source",
        run.out_dir
    );
    let expected_log = format!(
        "pwd={}/work/php-pair-8.4-darwin-arm64/staticphp\n\
argv=[build:php][json][--build-cli][--build-frankenphp][--enable-zts][--with-config-file-path=/var/empty/com.prvious.pv/php][--with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d][--dl-with-php=8.4.20][--dl-retry=3][--dl-custom-local][php-src:{php_source_dir}][--dl-custom-local][frankenphp:{frankenphp_source_dir}]\n",
        run.out_dir
    );

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert_eq!(run.spc_log, expected_log);
    assert!(
        !run.spc_log.contains("/usr/local/etc/php"),
        "PHP recipe must not pass /usr/local/etc/php fallback paths: {}",
        run.spc_log
    );
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
    assert_debug_snapshot!(
        "php_pair_build_smoke_builds_cli_and_frankenphp_from_one_staticphp_buildroot",
        build_recipe_record_provenance(run.php_record_json.as_deref())?
    );
    assert_debug_snapshot!(
        "php_pair_build_smoke_builds_cli_and_frankenphp_from_one_staticphp_buildroot-2",
        build_recipe_record_provenance(run.frankenphp_record_json.as_deref())?
    );
    assert_debug_snapshot!(
        "php_pair_build_smoke_builds_cli_and_frankenphp_from_one_staticphp_buildroot-3",
        build_recipe_notice_source_lines(run.frankenphp_notice.as_deref())?
    );

    Ok(())
}

#[test]
fn php_pair_build_smoke_prepatches_frankenphp_for_staticphp_php83_avx512_probe() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        recipe_track: "8.3",
        php_version: "8.3.31",
        frankenphp_version: "1.12.4",
        require_staticphp_php83_frankenphp_patch_context: true,
        ..default_build_recipe_options()
    })?;

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.frankenphp_archive_exists,
        "FrankenPHP archive was not written"
    );

    Ok(())
}

#[test]
fn composer_build_smoke_uses_platform_suffixed_archive_name() -> Result<()> {
    let run = run_composer_build_recipe_smoke()?;

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.platform_archive_exists,
        "Composer archive should include the platform in its basename"
    );
    assert!(
        !run.legacy_archive_exists,
        "Composer archive should not use the old platformless basename"
    );
    assert_eq!(
        run.validate_log,
        "archive=composer-2.10.1-pv1-any.tar.gz record=composer-2.10.1-pv1-any.json smoke=smoke.sh\n"
    );
    let expected_curl_log = format!(
        "argv=[-L][--fail][--show-error][--silent][--retry][3][--retry-delay][2][--retry-all-errors][--connect-timeout][20][--max-time][300][https://sources.example.test/composer.phar][-o][{}/work/composer-2.10.1-pv1-any/composer-2.10.1-pv1-any/composer.phar]\n",
        run.out_dir
    );
    assert_eq!(run.curl_log, expected_curl_log);

    let record_json = run
        .record_json
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Composer build recipe did not write a record"))?;
    let record: Value = serde_json::from_str(record_json)?;
    assert_eq!(
        record["object_key"],
        "resources/composer/2/2.10.1-pv1/any/composer-2.10.1-pv1-any.tar.gz"
    );

    Ok(())
}

#[test]
fn redis_build_recipe_signs_binaries_and_requires_third_party_notices() -> Result<()> {
    let run = run_redis_build_recipe_smoke(default_redis_build_recipe_options())?;

    assert!(
        run.output.status.success(),
        "Redis build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.archive_exists,
        "Redis archive was not written: {}",
        command_output_debug(&run.output)
    );
    assert!(run.record_json.is_some(), "Redis record was not written");
    assert_eq!(
        run.validate_log,
        "archive=redis-8.2.7-pv1-darwin-arm64.tar.gz record=redis-8.2.7-pv1-darwin-arm64.json smoke=smoke.sh\n"
    );
    assert_eq!(
        codesigned_file_names(&run.codesign_log),
        ["redis-cli".to_string(), "redis-server".to_string()]
    );

    assert!(
        run.archive_entries
            .contains(&"redis-8.2.7-pv1-darwin-arm64/THIRD-PARTY-NOTICES".to_string()),
        "Redis archive should contain third-party legal notices: {:?}",
        run.archive_entries
    );

    let record_json = run
        .record_json
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Redis build recipe did not write a record"))?;
    let record: Value = serde_json::from_str(record_json)?;
    assert_eq!(record["license_files"], serde_json::json!(["LICENSE"]));
    assert_eq!(
        record["notice_files"],
        serde_json::json!(["NOTICE", "THIRD-PARTY-NOTICES"])
    );

    Ok(())
}

#[test]
fn redis_build_recipe_does_not_publish_outputs_before_archive_validation() -> Result<()> {
    let run = run_redis_build_recipe_smoke(RedisBuildRecipeOptions {
        validate_archive_failure: true,
        ..default_redis_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "Redis build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.record_json.is_none(),
        "Redis record should not be written before archive validation succeeds"
    );
    assert!(
        !run.archive_exists,
        "Redis archive should not be written before archive validation succeeds"
    );
    assert_eq!(
        String::from_utf8_lossy(&run.output.stdout),
        "",
        "archive path should not be printed before validation succeeds"
    );

    Ok(())
}

#[test]
fn redis_build_recipe_rejects_missing_third_party_legal_header() -> Result<()> {
    let run = run_redis_build_recipe_smoke(RedisBuildRecipeOptions {
        include_fast_float_legal_header: false,
        ..default_redis_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "Redis build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    let stderr = String::from_utf8_lossy(&run.output.stderr);
    assert!(
        stderr.contains("missing Redis legal header"),
        "Redis build recipe should report the missing legal header: {stderr}"
    );
    assert!(
        run.record_json.is_none(),
        "Redis record should not be written when legal notice collection fails"
    );
    assert!(
        !run.archive_exists,
        "Redis archive should not be written when legal notice collection fails"
    );

    Ok(())
}

#[test]
fn redis_smoke_prints_server_log_on_startup_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_executable(
        &artifact_bin.join("redis-server"),
        r#"#!/bin/sh
set -eu
printf '%s\n' 'redis startup exploded' >&2
exit 71
"#,
    )?;
    write_executable(
        &artifact_bin.join("redis-cli"),
        r#"#!/bin/sh
set -eu
exit 72
"#,
    )?;
    write_executable(&command_bin.join("sleep"), "#!/bin/sh\nexit 0\n")?;

    let output = StdCommand::new(redis_smoke_hook())
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .output()?;

    assert!(
        !output.status.success(),
        "Redis smoke should fail: {}",
        command_output_debug(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("redis startup exploded"),
        "Redis smoke should print redis-server output on failure: {stderr}"
    );

    Ok(())
}

#[test]
fn redis_smoke_kills_server_when_shutdown_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_executable(
        &artifact_bin.join("redis-server"),
        r#"#!/bin/sh
set -eu
exec /bin/sleep 10
"#,
    )?;
    write_executable(
        &artifact_bin.join("redis-cli"),
        r#"#!/bin/sh
set -eu
exit 72
"#,
    )?;
    write_executable(&command_bin.join("sleep"), "#!/bin/sh\nexit 0\n")?;
    let started = Instant::now();
    let output = StdCommand::new(redis_smoke_hook())
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .output()?;

    assert!(
        !output.status.success(),
        "Redis smoke should fail when redis-cli never returns PONG: {}",
        command_output_debug(&output)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "Redis smoke should kill the server instead of waiting for it to exit"
    );

    Ok(())
}

#[test]
fn mailpit_smoke_rejects_prefix_version_match() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_fake_mailpit(&artifact_bin.join("mailpit"))?;
    write_fake_success_curl(&command_bin.join("curl"))?;

    let output = StdCommand::new(mailpit_smoke_hook())
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .env("PV_TEST_MAILPIT_VERSION_OUTPUT", "Mailpit v1.30.10")
        .env("PV_UPSTREAM_VERSION", "1.30.1")
        .output()?;

    assert!(
        !output.status.success(),
        "Mailpit smoke should reject prefix version matches: {}",
        command_output_debug(&output)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Mailpit version mismatch"),
        "Mailpit smoke should report the exact version mismatch: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn mailpit_smoke_fails_when_server_does_not_stop() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_fake_mailpit(&artifact_bin.join("mailpit"))?;
    write_fake_success_curl(&command_bin.join("curl"))?;

    let started = Instant::now();
    let output = StdCommand::new(mailpit_smoke_hook())
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .env("PV_TEST_MAILPIT_IGNORE_TERM", "1")
        .env("PV_TEST_MAILPIT_SLEEP_SECONDS", "10")
        .env("PV_TEST_MAILPIT_VERSION_OUTPUT", "Mailpit v1.30.1")
        .env("PV_UPSTREAM_VERSION", "1.30.1")
        .output()?;

    assert!(
        !output.status.success(),
        "Mailpit smoke should fail when the server ignores shutdown: {}",
        command_output_debug(&output)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "Mailpit smoke should use a bounded shutdown wait"
    );

    Ok(())
}

#[test]
fn rustfs_smoke_rejects_prefix_version_match() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");

    create_dir_all(&artifact_bin)?;
    write_fake_rustfs(&artifact_bin.join("rustfs"))?;

    let output = StdCommand::new(rustfs_smoke_hook())
        .arg(&artifact_root)
        .env("PV_TEST_RUSTFS_VERSION_OUTPUT", "rustfs 1.0.0-beta.70")
        .env("PV_UPSTREAM_VERSION", "1.0.0-beta.7")
        .output()?;

    assert!(
        !output.status.success(),
        "RustFS smoke should reject prefix version matches: {}",
        command_output_debug(&output)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("RustFS version mismatch"),
        "RustFS smoke should report the exact version mismatch: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn rustfs_smoke_uses_explicit_loopback_console_port() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let rustfs_log = tempdir.path().join("rustfs.log");

    create_dir_all(&artifact_bin)?;
    write_fake_rustfs(&artifact_bin.join("rustfs"))?;
    write_file(&rustfs_log, "")?;

    let output = StdCommand::new(rustfs_smoke_hook())
        .arg(&artifact_root)
        .env("PV_TEST_RUSTFS_LOG", &rustfs_log)
        .env("PV_TEST_RUSTFS_REQUIRE_CONSOLE_ADDRESS", "1")
        .env("PV_TEST_RUSTFS_VERSION_OUTPUT", "rustfs 1.0.0-beta.7")
        .env("PV_UPSTREAM_VERSION", "1.0.0-beta.7")
        .output()?;

    assert!(
        output.status.success(),
        "RustFS smoke should pass an explicit loopback console port: {}",
        command_output_debug(&output)
    );
    let rustfs_log = read_file(&rustfs_log)?;
    assert!(
        rustfs_log.contains("console=127.0.0.1:"),
        "RustFS smoke should not leave the console on the default wildcard listener: {rustfs_log}"
    );

    Ok(())
}

#[test]
fn rustfs_smoke_fails_when_server_does_not_stop() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");

    create_dir_all(&artifact_bin)?;
    write_fake_rustfs(&artifact_bin.join("rustfs"))?;

    let started = Instant::now();
    let output = StdCommand::new(rustfs_smoke_hook())
        .arg(&artifact_root)
        .env("PV_TEST_RUSTFS_IGNORE_TERM", "1")
        .env("PV_TEST_RUSTFS_SLEEP_SECONDS", "10")
        .env("PV_TEST_RUSTFS_VERSION_OUTPUT", "rustfs 1.0.0-beta.7")
        .env("PV_UPSTREAM_VERSION", "1.0.0-beta.7")
        .output()?;

    assert!(
        !output.status.success(),
        "RustFS smoke should fail when the server ignores shutdown: {}",
        command_output_debug(&output)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "RustFS smoke should use a bounded shutdown wait"
    );

    Ok(())
}

#[test]
fn mailpit_build_recipe_rejects_unexpected_macho_architecture() -> Result<()> {
    let run = run_mailpit_build_recipe_smoke(BackingBuildRecipeOptions {
        lipo_archs: "x86_64",
        ..default_backing_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "Mailpit build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert!(
        !run.archive_exists,
        "Mailpit archive should not be written before Mach-O validation succeeds"
    );
    assert!(
        run.record_json.is_none(),
        "Mailpit record should not be written before Mach-O validation succeeds"
    );
    assert_eq!(
        run.validate_log, "",
        "archive validation should not run before Mach-O validation succeeds"
    );

    Ok(())
}

#[test]
fn rustfs_build_recipe_rejects_newer_macho_minimum_os() -> Result<()> {
    let run = run_rustfs_build_recipe_smoke(BackingBuildRecipeOptions {
        macho_minos: "14.0",
        ..default_backing_build_recipe_options()
    })?;

    assert!(
        !run.output.status.success(),
        "RustFS build recipe unexpectedly succeeded: {}",
        command_output_debug(&run.output)
    );
    assert!(
        !run.archive_exists,
        "RustFS archive should not be written before Mach-O validation succeeds"
    );
    assert!(
        run.record_json.is_none(),
        "RustFS record should not be written before Mach-O validation succeeds"
    );
    assert_eq!(
        run.validate_log, "",
        "archive validation should not run before Mach-O validation succeeds"
    );

    Ok(())
}

#[test]
fn common_recipe_helper_ad_hoc_signs_macho_files() -> Result<()> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let artifact_lib = artifact_root.join("lib");
    let sign_log = tempdir.path().join("codesign.log");
    let harness = tempdir.path().join("sign-harness.sh");
    let common =
        Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/common.sh");

    create_dir_all(&fake_bin)?;
    create_dir_all(&artifact_bin)?;
    create_dir_all(&artifact_lib)?;
    write_file(&artifact_bin.join("mysql"), "mach-o fixture\n")?;
    write_file(&artifact_bin.join("mysqladmin"), "mach-o fixture\n")?;
    write_file(&artifact_bin.join("README"), "plain text\n")?;
    write_file(
        &artifact_lib.join("libmysqlclient.dylib"),
        "mach-o fixture\n",
    )?;
    write_file(&sign_log, "")?;
    write_fake_signing_otool(&fake_bin.join("otool"))?;
    write_fake_codesign(&fake_bin.join("codesign"))?;
    write_executable(
        &harness,
        r#"#!/bin/sh
set -eu

# shellcheck source=/dev/null
. "$PV_TEST_COMMON_SH"
pv_recipe_ad_hoc_sign_macho_tree "$PV_TEST_ARTIFACT_ROOT"
"#,
    )?;

    let output = StdCommand::new(&harness)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_TEST_ARTIFACT_ROOT", &artifact_root)
        .env("PV_TEST_CODESIGN_LOG", &sign_log)
        .env("PV_TEST_COMMON_SH", &common)
        .output()?;

    assert!(
        output.status.success(),
        "signing helper failed: {}",
        command_output_debug(&output)
    );
    assert_debug_snapshot!(read_file(&sign_log)?.replace(tempdir.path().as_str(), "<tmp>"));

    Ok(())
}

#[test]
fn common_recipe_helper_rewrites_nested_macho_install_names() -> Result<()> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let artifact_lib = artifact_root.join("lib");
    let artifact_plugin = artifact_lib.join("plugin");
    let artifact_postgresql = artifact_lib.join("postgresql");
    let openssl_prefix = tempdir.path().join("openssl@3");
    let install_name_log = tempdir.path().join("install-name.log");
    let harness = tempdir.path().join("rewrite-harness.sh");
    let common =
        Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/common.sh");

    create_dir_all(&fake_bin)?;
    create_dir_all(&artifact_bin)?;
    create_dir_all(&artifact_lib)?;
    create_dir_all(&artifact_plugin)?;
    create_dir_all(&artifact_postgresql)?;
    write_file(&artifact_bin.join("mysql"), "mach-o fixture\n")?;
    write_file(
        &artifact_lib.join("libmysqlclient.dylib"),
        "mach-o fixture\n",
    )?;
    write_file(&artifact_lib.join("libssl.3.dylib"), "mach-o fixture\n")?;
    write_file(&artifact_plugin.join("auth.so"), "mach-o fixture\n")?;
    write_file(
        &artifact_postgresql.join("extension.so"),
        "mach-o fixture\n",
    )?;
    write_file(&install_name_log, "")?;
    write_fake_install_name_otool(&fake_bin.join("otool"))?;
    write_fake_install_name_tool(&fake_bin.join("install_name_tool"))?;
    write_executable(
        &harness,
        r#"#!/bin/sh
set -eu

# shellcheck source=/dev/null
. "$PV_TEST_COMMON_SH"
rewrite_macho_install_names "$PV_TEST_ARTIFACT_ROOT" "$PV_TEST_INSTALL_DIR" "$PV_TEST_OPENSSL_PREFIX"
"#,
    )?;

    let install_dir = "/opt/pv-mysql";
    let output = StdCommand::new(&harness)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_TEST_ARTIFACT_ROOT", &artifact_root)
        .env("PV_TEST_COMMON_SH", &common)
        .env("PV_TEST_INSTALL_DIR", install_dir)
        .env("PV_TEST_INSTALL_NAME_LOG", &install_name_log)
        .env("PV_TEST_OPENSSL_PREFIX", &openssl_prefix)
        .output()?;

    assert!(
        output.status.success(),
        "rewrite helper failed: {}",
        command_output_debug(&output)
    );

    let mut rewrites = read_file(&install_name_log)?
        .lines()
        .map(|line| {
            line.replace(tempdir.path().as_str(), "<tmp>")
                .replace(install_dir, "<install>")
        })
        .collect::<Vec<_>>();
    rewrites.sort();
    assert_debug_snapshot!(rewrites);

    Ok(())
}

#[test]
fn common_recipe_helper_fails_when_signing_one_macho_file_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let sign_count = tempdir.path().join("codesign.count");
    let harness = tempdir.path().join("sign-fail-harness.sh");
    let common =
        Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/common.sh");

    create_dir_all(&fake_bin)?;
    create_dir_all(&artifact_bin)?;
    write_file(&artifact_bin.join("mysql"), "mach-o fixture\n")?;
    write_file(&artifact_bin.join("mysqladmin"), "mach-o fixture\n")?;
    write_fake_signing_otool(&fake_bin.join("otool"))?;
    write_fake_first_call_failing_codesign(&fake_bin.join("codesign"))?;
    write_file(&sign_count, "0\n")?;
    write_executable(
        &harness,
        r#"#!/bin/sh
set -eu

# shellcheck source=/dev/null
. "$PV_TEST_COMMON_SH"
pv_recipe_ad_hoc_sign_macho_tree "$PV_TEST_ARTIFACT_ROOT"
"#,
    )?;

    let output = StdCommand::new(&harness)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_TEST_ARTIFACT_ROOT", &artifact_root)
        .env("PV_TEST_CODESIGN_COUNT", &sign_count)
        .env("PV_TEST_COMMON_SH", &common)
        .output()?;

    assert!(
        !output.status.success(),
        "signing helper should fail on the first failed file: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn common_recipe_helper_fails_when_install_name_rewrite_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let artifact_root = tempdir.path().join("artifact");
    let artifact_lib = artifact_root.join("lib");
    let install_name_count = tempdir.path().join("install-name.count");
    let harness = tempdir.path().join("rewrite-fail-harness.sh");
    let common =
        Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/common.sh");

    create_dir_all(&fake_bin)?;
    create_dir_all(&artifact_lib)?;
    write_file(&artifact_lib.join("liba.dylib"), "mach-o fixture\n")?;
    write_file(&artifact_lib.join("libz.dylib"), "mach-o fixture\n")?;
    write_fake_rewrite_failure_otool(&fake_bin.join("otool"))?;
    write_fake_first_call_failing_install_name_tool(&fake_bin.join("install_name_tool"))?;
    write_file(&install_name_count, "0\n")?;
    write_executable(
        &harness,
        r#"#!/bin/sh
set -eu

# shellcheck source=/dev/null
. "$PV_TEST_COMMON_SH"
rewrite_macho_install_names "$PV_TEST_ARTIFACT_ROOT" "$PV_TEST_INSTALL_DIR"
"#,
    )?;

    let output = StdCommand::new(&harness)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_TEST_ARTIFACT_ROOT", &artifact_root)
        .env("PV_TEST_COMMON_SH", &common)
        .env("PV_TEST_INSTALL_DIR", "/opt/pv-mysql")
        .env("PV_TEST_INSTALL_NAME_COUNT", &install_name_count)
        .output()?;

    assert!(
        !output.status.success(),
        "rewrite helper should fail on the first failed file: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn mysql_smoke_uses_tcp_readiness_and_select() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact_root = tempdir.path().join("artifact");
    let artifact_bin = artifact_root.join("bin");
    let command_bin = tempdir.path().join("commands");

    create_dir_all(&artifact_bin)?;
    create_dir_all(&command_bin)?;
    write_fake_mysql_server(&artifact_bin.join("mysqld"))?;
    write_fake_mysqladmin_requires_tcp(&artifact_bin.join("mysqladmin"))?;
    write_fake_mysql_requires_tcp(&artifact_bin.join("mysql"))?;
    write_executable(&command_bin.join("sleep"), "#!/bin/sh\nexit 0\n")?;

    let output = StdCommand::new(mysql_smoke_hook())
        .arg(&artifact_root)
        .env(
            "PATH",
            format!("{command_bin}:/usr/bin:/bin:/usr/sbin:/sbin"),
        )
        .output()?;

    assert!(
        output.status.success(),
        "MySQL smoke should validate TCP readiness and SELECT 1: {}",
        command_output_debug(&output)
    );

    Ok(())
}

#[test]
fn mysql_build_recipe_builds_openssl_prefix_for_cmake() -> Result<()> {
    let run = run_mysql_build_recipe_smoke()?;

    assert!(
        run.output.status.success(),
        "MySQL build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.archive_exists,
        "MySQL archive should be written after CMake build and archive validation"
    );
    assert!(run.record_json.is_some(), "MySQL record was not written");
    assert_eq!(
        run.validate_log,
        "archive=mysql-8.4.9-pv1-darwin-arm64.tar.gz record=mysql-8.4.9-pv1-darwin-arm64.json smoke=smoke.sh\n"
    );
    let cmake_log = run.cmake_log.replace(&run.openssl_prefix, "<openssl>");
    let cmake_log = cmake_log.replace(&run.bison_executable, "<bison>");
    let openssl_build_log = run
        .openssl_build_log
        .replace(&run.openssl_prefix, "<openssl>");
    assert!(
        openssl_build_log.contains("configure-target=darwin64-arm64-cc deployment=13.0"),
        "MySQL recipe should configure its OpenSSL dependency for arm64 macOS 13: {openssl_build_log}"
    );
    assert!(
        openssl_build_log.contains("make=[-j][1]"),
        "MySQL recipe should build OpenSSL before configuring MySQL: {openssl_build_log}"
    );
    assert!(
        openssl_build_log.contains("make=[install_sw]"),
        "MySQL recipe should install the recipe-built OpenSSL prefix before configuring MySQL: {openssl_build_log}"
    );
    assert!(
        cmake_log.contains("[-DWITH_SSL=<openssl>]"),
        "MySQL CMake invocation should use the recipe-built OpenSSL prefix: {cmake_log}"
    );
    assert!(
        !cmake_log.contains("OPENSSL_USE_STATIC_LIBS"),
        "MySQL CMake invocation should let WITH_SSL select the recipe-built OpenSSL libraries: {cmake_log}"
    );
    assert!(
        cmake_log.contains("[-DBISON_EXECUTABLE=<bison>]"),
        "MySQL CMake invocation should use a Homebrew Bison executable: {cmake_log}"
    );
    assert!(
        !cmake_log.contains("[-DWITH_SSL=bundled]"),
        "MySQL 8.4 rejects WITH_SSL=bundled: {cmake_log}"
    );
    assert_debug_snapshot!(mysql_record_source_inputs(&run)?, @r#"
    Array [
        Object {
            "name": String("openssl"),
            "source_sha256": String("8505c910292123009b4f1327adb5ae9935c04bb05780d1436998953efe501ed4"),
            "source_url": String("https://sources.example.test/openssl.tar.gz"),
        },
    ]
    "#);

    Ok(())
}

#[test]
fn mysql_build_recipe_prunes_broken_optional_plugin_symlinks() -> Result<()> {
    let run = run_mysql_build_recipe_smoke_with_options(MysqlBuildRecipeOptions {
        install_broken_plugin_symlink: true,
        ..MysqlBuildRecipeOptions::default()
    })?;

    assert!(
        run.output.status.success(),
        "MySQL build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert!(
        run.archive_exists,
        "MySQL archive should still be written when an optional plugin symlink is broken"
    );

    Ok(())
}

#[test]
fn mysql_80_build_recipe_records_boost_source_input() -> Result<()> {
    let run = run_mysql_build_recipe_smoke_with_options(MysqlBuildRecipeOptions {
        track: "8.0",
        upstream_version: "8.0.46",
        ..MysqlBuildRecipeOptions::default()
    })?;

    assert!(
        run.output.status.success(),
        "MySQL 8.0 build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert_debug_snapshot!(mysql_record_source_inputs(&run)?);

    Ok(())
}

#[test]
fn mysql_build_recipe_rejects_unknown_broken_symlinks() -> Result<()> {
    let run = run_mysql_build_recipe_smoke_with_options(MysqlBuildRecipeOptions {
        install_broken_required_symlink: true,
        ..MysqlBuildRecipeOptions::default()
    })?;

    assert!(
        !run.output.status.success(),
        "MySQL build recipe should reject unknown broken symlinks: {}",
        command_output_debug(&run.output)
    );
    let stderr = String::from_utf8_lossy(&run.output.stderr);
    assert!(
        stderr.contains("broken MySQL install symlink"),
        "MySQL build recipe should report the broken symlink path: {stderr}"
    );
    assert!(
        !run.archive_exists,
        "MySQL archive should not be written after an unknown broken symlink"
    );

    Ok(())
}

#[test]
fn sql_build_recipes_pin_macos_deployment_target() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mysql_build = read_file(&workspace_root.join("release/artifacts/recipes/mysql/build.sh"))?;
    let postgres_build =
        read_file(&workspace_root.join("release/artifacts/recipes/postgres/build.sh"))?;

    assert!(
        mysql_build.contains("\nDEPLOYMENT_TARGET=13.0\n"),
        "MySQL build recipe should pin the deployment target to macOS 13.0"
    );
    assert!(
        !mysql_build.contains("PV_MACOSX_DEPLOYMENT_TARGET"),
        "MySQL build recipe should not allow caller-controlled deployment targets"
    );
    assert!(
        postgres_build.contains("\nDEPLOYMENT_TARGET=13.0\n"),
        "Postgres build recipe should pin the deployment target to macOS 13.0"
    );
    assert!(
        !postgres_build.contains("PV_MACOSX_DEPLOYMENT_TARGET"),
        "Postgres build recipe should not allow caller-controlled deployment targets"
    );

    Ok(())
}

#[test]
fn backing_build_recipes_ad_hoc_sign_macho_payloads() -> Result<()> {
    let mut summaries = Vec::new();
    for recipe in BackingBuildRecipe::all() {
        let run = run_backing_build_recipe_signing_smoke(recipe)?;
        assert!(
            run.output.status.success(),
            "{} build recipe failed: {}",
            recipe.resource,
            command_output_debug(&run.output)
        );
        assert!(
            run.archive_exists,
            "{} build recipe did not write an archive",
            recipe.resource
        );
        assert!(
            run.record_json.is_some(),
            "{} build recipe did not write a record",
            recipe.resource
        );
        let expected_signed_files = recipe
            .signed_files
            .iter()
            .map(|file_name| (*file_name).to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            run.signed_files, expected_signed_files,
            "{} build recipe did not ad-hoc sign the expected payloads",
            recipe.resource
        );
        summaries.push((
            recipe.resource,
            run.signed_files,
            run.validate_log.replace(&run.out_dir, "<out>"),
        ));
    }

    assert_debug_snapshot!(summaries);

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
fn php_pair_build_smoke_removes_unmanaged_frankenphp_rpath_before_validation() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        frankenphp_macho_rpaths: "@loader_path/../lib\n/usr/local/lib",
        ..default_build_recipe_options()
    })?;

    assert!(
        run.output.status.success(),
        "build recipe failed: {}",
        command_output_debug(&run.output)
    );
    assert_eq!(run.deleted_rpath_log, "/usr/local/lib\n");
    assert!(
        run.frankenphp_archive_exists,
        "FrankenPHP archive should be written after the unmanaged rpath is removed"
    );

    Ok(())
}

#[test]
fn php_build_smoke_accepts_system_and_relative_macho_runtime_metadata() -> Result<()> {
    let run = run_php_build_recipe_smoke_with_options(BuildRecipeOptions {
        macho_libraries: "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)\n\t/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 2503.1.0)\n\t@rpath/libphp.dylib (compatibility version 1.0.0, current version 1.0.0)\n\t@loader_path/../lib/libz.dylib (compatibility version 1.0.0, current version 1.3.1)",
        macho_rpaths: "@loader_path\n@loader_path/../lib\n@executable_path\n@executable_path/../lib",
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
    deleted_rpath_log: String,
    php_archive_exists: bool,
    frankenphp_archive_exists: bool,
}

struct ComposerBuildRecipeRun {
    out_dir: String,
    output: Output,
    record_json: Option<String>,
    curl_log: String,
    validate_log: String,
    platform_archive_exists: bool,
    legacy_archive_exists: bool,
}

struct RedisBuildRecipeRun {
    output: Output,
    record_json: Option<String>,
    validate_log: String,
    codesign_log: String,
    archive_entries: Vec<String>,
    archive_exists: bool,
}

struct MysqlBuildRecipeRun {
    output: Output,
    record_json: Option<String>,
    bison_executable: String,
    cmake_log: String,
    openssl_build_log: String,
    openssl_prefix: String,
    validate_log: String,
    archive_exists: bool,
}

struct MysqlBuildRecipeOptions {
    track: &'static str,
    upstream_version: &'static str,
    install_broken_plugin_symlink: bool,
    install_broken_required_symlink: bool,
}

impl Default for MysqlBuildRecipeOptions {
    fn default() -> Self {
        Self {
            track: "8.4",
            upstream_version: "8.4.9",
            install_broken_plugin_symlink: false,
            install_broken_required_symlink: false,
        }
    }
}

#[derive(Clone, Copy)]
struct BackingBuildRecipe {
    resource: &'static str,
    track: &'static str,
    upstream_version: &'static str,
    artifact_version: &'static str,
    platform: &'static str,
    source_kind: BackingSourceKind,
    signed_files: &'static [&'static str],
}

#[derive(Clone, Copy)]
enum BackingSourceKind {
    Redis,
    TarGzBinary,
    ZipBinary,
}

struct BackingBuildRecipeRun {
    out_dir: String,
    output: Output,
    record_json: Option<String>,
    signed_files: Vec<String>,
    validate_log: String,
    archive_exists: bool,
}

struct BuildRecipeOptions<'a> {
    recipe_track: &'a str,
    php_version: &'a str,
    frankenphp_version: &'a str,
    lipo_archs: &'a str,
    macho_minos: &'a str,
    macho_libraries: &'a str,
    macho_rpaths: &'a str,
    frankenphp_macho_libraries: &'a str,
    frankenphp_macho_rpaths: &'a str,
    validate_archive_failure_resource: &'a str,
    require_staticphp_php83_frankenphp_patch_context: bool,
}

struct RedisBuildRecipeOptions {
    validate_archive_failure: bool,
    include_fast_float_legal_header: bool,
}

struct BackingBuildRecipeOptions<'a> {
    lipo_archs: &'a str,
    macho_minos: &'a str,
    macho_libraries: &'a str,
    macho_rpaths: &'a str,
}

const PHP_83_AVX512_ORIGINAL_M4: &str = r#"dnl PHP_CHECK_AVX512_SUPPORTS
dnl
AC_DEFUN([PHP_CHECK_AVX512_SUPPORTS], [
  AC_MSG_CHECKING([for avx512 supports in compiler])
  save_CFLAGS="$CFLAGS"
  CFLAGS="-mavx512f -mavx512cd -mavx512vl -mavx512dq -mavx512bw $CFLAGS"

  AC_LINK_IFELSE([AC_LANG_SOURCE([[
    #include <immintrin.h>
      int main(void) {
        __m512i mask = _mm512_set1_epi32(0x1);
        char out[32];
        _mm512_storeu_si512(out, _mm512_shuffle_epi8(mask, mask));
        return 0;
    }]])], [
    have_avx512_supports=1
    AC_MSG_RESULT([yes])
  ], [
    have_avx512_supports=0
    AC_MSG_RESULT([no])
  ])

  CFLAGS="$save_CFLAGS"

  AC_DEFINE_UNQUOTED([PHP_HAVE_AVX512_SUPPORTS],
   [$have_avx512_supports], [Whether the compiler supports AVX512])
])

dnl PHP_CHECK_AVX512_VBMI_SUPPORTS
dnl
AC_DEFUN([PHP_CHECK_AVX512_VBMI_SUPPORTS], [
  AC_MSG_CHECKING([for avx512 vbmi supports in compiler])
  save_CFLAGS="$CFLAGS"
  CFLAGS="-mavx512f -mavx512cd -mavx512vl -mavx512dq -mavx512bw -mavx512vbmi $CFLAGS"
  AC_LINK_IFELSE([AC_LANG_SOURCE([[
    #include <immintrin.h>
      int main(void) {
        __m512i mask = _mm512_set1_epi32(0x1);
        char out[32];
        _mm512_storeu_si512(out, _mm512_permutexvar_epi8(mask, mask));
        return 0;
    }]])], [
    have_avx512_vbmi_supports=1
    AC_MSG_RESULT([yes])
  ], [
    have_avx512_vbmi_supports=0
    AC_MSG_RESULT([no])
  ])
  CFLAGS="$save_CFLAGS"
  AC_DEFINE_UNQUOTED([PHP_HAVE_AVX512_VBMI_SUPPORTS],
   [$have_avx512_vbmi_supports], [Whether the compiler supports AVX512 VBMI])
])
"#;

impl BackingBuildRecipe {
    fn all() -> [Self; 3] {
        [
            Self {
                resource: "redis",
                track: "8.2",
                upstream_version: "8.2.1",
                artifact_version: "8.2.1-pv1",
                platform: "darwin-arm64",
                source_kind: BackingSourceKind::Redis,
                signed_files: &["redis-cli", "redis-server"],
            },
            Self {
                resource: "mailpit",
                track: "1",
                upstream_version: "1.30.1",
                artifact_version: "1.30.1-pv1",
                platform: "darwin-arm64",
                source_kind: BackingSourceKind::TarGzBinary,
                signed_files: &["mailpit"],
            },
            Self {
                resource: "rustfs",
                track: "1",
                upstream_version: "1.0.0-beta.7",
                artifact_version: "1.0.0-beta.7-pv1",
                platform: "darwin-arm64",
                source_kind: BackingSourceKind::ZipBinary,
                signed_files: &["rustfs"],
            },
        ]
    }

    fn artifact_basename(self) -> String {
        format!(
            "{}-{}-{}",
            self.resource, self.artifact_version, self.platform
        )
    }
}

fn php_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/php/smoke.sh")
}

fn php_staticphp_avx512_patch() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/php/patches/staticphp/spc_fix_avx512_cache_before_80400.patch")
}

fn composer_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/composer/smoke.sh")
}

fn redis_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/redis/smoke.sh")
}

fn mysql_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts/recipes/mysql/smoke.sh")
}

fn mailpit_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/mailpit/smoke.sh")
}

fn rustfs_smoke_hook() -> camino::Utf8PathBuf {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/rustfs/smoke.sh")
}

fn run_php_build_recipe_smoke() -> Result<BuildRecipeRun> {
    run_php_build_recipe_smoke_with_options(default_build_recipe_options())
}

fn default_redis_build_recipe_options() -> RedisBuildRecipeOptions {
    RedisBuildRecipeOptions {
        validate_archive_failure: false,
        include_fast_float_legal_header: true,
    }
}

fn run_redis_build_recipe_smoke(options: RedisBuildRecipeOptions) -> Result<RedisBuildRecipeRun> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let out_dir = tempdir.path().join("out");
    let record_dir = tempdir.path().join("records");
    let source_archive = tempdir.path().join("redis-source.tar.gz");
    let curl_log = tempdir.path().join("curl.log");
    let validate_log = tempdir.path().join("validate.log");
    let codesign_log = tempdir.path().join("codesign.log");

    create_dir_all(&fake_bin)?;
    write_redis_source_archive_with_options(
        &source_archive,
        options.include_fast_float_legal_header,
    )?;
    write_fake_redis_cargo(&fake_bin.join("cargo"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_fake_lipo(&fake_bin.join("lipo"))?;
    write_fake_otool(&fake_bin.join("otool"))?;
    write_fake_make(&fake_bin.join("make"))?;
    write_fake_codesign(&fake_bin.join("codesign"))?;
    write_fake_sysctl(&fake_bin.join("sysctl"))?;
    write_fake_uname(&fake_bin.join("uname"))?;
    write_file(&curl_log, "")?;
    write_file(&validate_log, "")?;
    write_file(&codesign_log, "")?;

    let build_script = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/redis/build.sh");
    let output = StdCommand::new(build_script)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_ARTIFACT_OUT_DIR", &out_dir)
        .env("PV_ARTIFACT_RECORD_DIR", &record_dir)
        .env("PV_BUILD_RUN_ID", "local-test")
        .env("PV_COMMIT", "0123456789abcdef0123456789abcdef01234567")
        .env("PV_RECIPE_PLATFORM", "darwin-arm64")
        .env("PV_RECIPE_TRACK", "8.2")
        .env("PV_TEST_CURL_LOG", &curl_log)
        .env("PV_TEST_SOURCE_ARCHIVE", &source_archive)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_archive)?)
        .env(
            "PV_TEST_VALIDATE_ARCHIVE_FAILURE",
            if options.validate_archive_failure {
                "1"
            } else {
                ""
            },
        )
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .env("PV_TEST_CODESIGN_LOG", &codesign_log)
        .env("PV_TEST_LIPO_ARCHS", "arm64")
        .env("PV_TEST_MACHO_LIBRARIES", "")
        .env("PV_TEST_MACHO_MINOS", "13.0")
        .env("PV_TEST_MACHO_RPATHS", "")
        .output()?;

    let artifact_version = "8.2.7-pv1";
    let artifact_basename = "redis-8.2.7-pv1-darwin-arm64";
    let archive = out_dir.join(format!("{artifact_basename}.tar.gz"));
    let archive_exists = path_exists(&archive);
    let archive_entries = if archive_exists {
        archive_entries(&archive)?
    } else {
        Vec::new()
    };
    let record = record_dir
        .join("redis")
        .join("8.2")
        .join(artifact_version)
        .join("darwin-arm64")
        .join(format!("{artifact_basename}.json"));

    Ok(RedisBuildRecipeRun {
        output,
        record_json: read_optional_file(&record)?,
        validate_log: read_file(&validate_log)?,
        codesign_log: read_file(&codesign_log)?,
        archive_entries,
        archive_exists,
    })
}

fn run_mysql_build_recipe_smoke() -> Result<MysqlBuildRecipeRun> {
    run_mysql_build_recipe_smoke_with_options(MysqlBuildRecipeOptions::default())
}

fn run_mysql_build_recipe_smoke_with_options(
    options: MysqlBuildRecipeOptions,
) -> Result<MysqlBuildRecipeRun> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let out_dir = tempdir.path().join("out");
    let record_dir = tempdir.path().join("records");
    let source_archive = tempdir.path().join("mysql-source.tar.gz");
    let openssl_source_archive = tempdir.path().join("openssl-source.tar.gz");
    let boost_source_archive = tempdir.path().join("boost-source.tar.gz");
    let artifact_basename = format!("mysql-{}-pv1-darwin-arm64", options.upstream_version);
    let openssl_prefix = out_dir
        .join("work")
        .join(&artifact_basename)
        .join("openssl-3.5.7");
    let bison_prefix = tempdir.path().join("bison");
    let bison_executable = bison_prefix.join("bin/bison");
    let curl_log = tempdir.path().join("curl.log");
    let cmake_log = tempdir.path().join("cmake.log");
    let openssl_build_log = tempdir.path().join("openssl-build.log");
    let validate_log = tempdir.path().join("validate.log");
    let codesign_log = tempdir.path().join("codesign.log");

    create_dir_all(&fake_bin)?;
    write_source_archive(&source_archive, "mysql-source")?;
    write_openssl_source_archive(&openssl_source_archive)?;
    write_source_archive(&boost_source_archive, "boost-source")?;
    write_fake_backing_cargo(&fake_bin.join("cargo"))?;
    write_fake_brew(&fake_bin.join("brew"))?;
    write_fake_mysql_cmake(&fake_bin.join("cmake"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_fake_install_name_tool(&fake_bin.join("install_name_tool"))?;
    write_fake_lipo(&fake_bin.join("lipo"))?;
    write_fake_otool(&fake_bin.join("otool"))?;
    write_fake_codesign(&fake_bin.join("codesign"))?;
    write_fake_mysql_make(&fake_bin.join("make"))?;
    write_fake_openssl_perl(&fake_bin.join("perl"))?;
    write_fake_sysctl(&fake_bin.join("sysctl"))?;
    write_file(&curl_log, "")?;
    write_file(&cmake_log, "")?;
    write_file(&openssl_build_log, "")?;
    write_file(&validate_log, "")?;
    write_file(&codesign_log, "")?;

    let build_script = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/mysql/build.sh");
    let output = StdCommand::new(build_script)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_ARTIFACT_OUT_DIR", &out_dir)
        .env("PV_ARTIFACT_RECORD_DIR", &record_dir)
        .env("PV_BUILD_RUN_ID", "local-test")
        .env("PV_COMMIT", "0123456789abcdef0123456789abcdef01234567")
        .env("PV_RECIPE_PLATFORM", "darwin-arm64")
        .env("PV_RECIPE_TRACK", options.track)
        .env("PV_TEST_CMAKE_LOG", &cmake_log)
        .env("PV_TEST_CODESIGN_LOG", &codesign_log)
        .env("PV_TEST_CURL_LOG", &curl_log)
        .env("PV_TEST_OPENSSL_BUILD_LOG", &openssl_build_log)
        .env("PV_TEST_BISON_EXECUTABLE", &bison_executable)
        .env("PV_TEST_BISON_PREFIX", &bison_prefix)
        .env("PV_TEST_LIPO_ARCHS", "arm64")
        .env("PV_TEST_MACHO_LIBRARIES", "")
        .env("PV_TEST_MACHO_MINOS", "13.0")
        .env("PV_TEST_MACHO_RPATHS", "")
        .env(
            "PV_TEST_MYSQL_INSTALL_BROKEN_PLUGIN_SYMLINK",
            if options.install_broken_plugin_symlink {
                "1"
            } else {
                ""
            },
        )
        .env(
            "PV_TEST_MYSQL_INSTALL_BROKEN_REQUIRED_SYMLINK",
            if options.install_broken_required_symlink {
                "1"
            } else {
                ""
            },
        )
        .env(
            "PV_MYSQL_OPENSSL_SOURCE_URL",
            "https://sources.example.test/openssl.tar.gz",
        )
        .env(
            "PV_MYSQL_OPENSSL_SOURCE_SHA256",
            file_sha256(&openssl_source_archive)?,
        )
        .env(
            "PV_MYSQL_BOOST_SOURCE_URL",
            "https://sources.example.test/boost.tar.gz",
        )
        .env(
            "PV_MYSQL_BOOST_SOURCE_SHA256",
            file_sha256(&boost_source_archive)?,
        )
        .env("PV_TEST_BOOST_SOURCE_ARCHIVE", &boost_source_archive)
        .env("PV_TEST_OPENSSL_PREFIX", &openssl_prefix)
        .env("PV_TEST_OPENSSL_SOURCE_ARCHIVE", &openssl_source_archive)
        .env("PV_TEST_RESOURCE", "mysql")
        .env("PV_TEST_SOURCE_ARCHIVE", &source_archive)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_archive)?)
        .env("PV_TEST_UPSTREAM_VERSION", options.upstream_version)
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .output()?;

    let artifact_version = format!("{}-pv1", options.upstream_version);
    let archive = out_dir.join(format!("{artifact_basename}.tar.gz"));
    let record = record_dir
        .join("mysql")
        .join(options.track)
        .join(&artifact_version)
        .join("darwin-arm64")
        .join(format!("{artifact_basename}.json"));

    Ok(MysqlBuildRecipeRun {
        output,
        record_json: read_optional_file(&record)?,
        bison_executable: bison_executable.to_string(),
        cmake_log: read_file(&cmake_log)?,
        openssl_build_log: read_file(&openssl_build_log)?,
        openssl_prefix: openssl_prefix.to_string(),
        validate_log: read_file(&validate_log)?,
        archive_exists: path_exists(&archive),
    })
}

fn default_backing_build_recipe_options() -> BackingBuildRecipeOptions<'static> {
    BackingBuildRecipeOptions {
        lipo_archs: "arm64",
        macho_minos: "13.0",
        macho_libraries: "",
        macho_rpaths: "",
    }
}

fn run_mailpit_build_recipe_smoke(
    options: BackingBuildRecipeOptions<'_>,
) -> Result<BackingBuildRecipeRun> {
    run_backing_build_recipe_smoke("mailpit", "1.30.1", "mailpit", "tar.gz", options)
}

fn run_rustfs_build_recipe_smoke(
    options: BackingBuildRecipeOptions<'_>,
) -> Result<BackingBuildRecipeRun> {
    run_backing_build_recipe_smoke("rustfs", "1.0.0-beta.7", "rustfs", "zip", options)
}

fn run_backing_build_recipe_smoke(
    resource: &str,
    upstream_version: &str,
    binary_name: &str,
    source_extension: &str,
    options: BackingBuildRecipeOptions<'_>,
) -> Result<BackingBuildRecipeRun> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let out_dir = tempdir.path().join("out");
    let record_dir = tempdir.path().join("records");
    let source_archive = tempdir
        .path()
        .join(format!("{resource}-source.{source_extension}"));
    let curl_log = tempdir.path().join("curl.log");
    let validate_log = tempdir.path().join("validate.log");
    let codesign_log = tempdir.path().join("codesign.log");

    create_dir_all(&fake_bin)?;
    match source_extension {
        "tar.gz" => write_single_binary_source_archive(&source_archive, binary_name)?,
        "zip" => write_file(&source_archive, "zip fixture\n")?,
        _ => anyhow::bail!("unsupported source extension: {source_extension}"),
    }
    write_fake_backing_cargo(&fake_bin.join("cargo"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_fake_lipo(&fake_bin.join("lipo"))?;
    write_fake_otool(&fake_bin.join("otool"))?;
    write_fake_codesign(&fake_bin.join("codesign"))?;
    write_fake_unzip(&fake_bin.join("unzip"))?;
    write_file(&curl_log, "")?;
    write_file(&validate_log, "")?;
    write_file(&codesign_log, "")?;

    let build_script = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../release/artifacts/recipes/{resource}/build.sh"
    ));
    let output = StdCommand::new(build_script)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_ARTIFACT_OUT_DIR", &out_dir)
        .env("PV_ARTIFACT_RECORD_DIR", &record_dir)
        .env("PV_BUILD_RUN_ID", "local-test")
        .env("PV_COMMIT", "0123456789abcdef0123456789abcdef01234567")
        .env("PV_RECIPE_PLATFORM", "darwin-arm64")
        .env("PV_RECIPE_TRACK", "1")
        .env("PV_TEST_BINARY_NAME", binary_name)
        .env("PV_TEST_CURL_LOG", &curl_log)
        .env("PV_TEST_CODESIGN_LOG", &codesign_log)
        .env("PV_TEST_RESOURCE", resource)
        .env("PV_TEST_SOURCE_ARCHIVE", &source_archive)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_archive)?)
        .env("PV_TEST_UPSTREAM_VERSION", upstream_version)
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .env("PV_TEST_LIPO_ARCHS", options.lipo_archs)
        .env("PV_TEST_MACHO_LIBRARIES", options.macho_libraries)
        .env("PV_TEST_MACHO_MINOS", options.macho_minos)
        .env("PV_TEST_MACHO_RPATHS", options.macho_rpaths)
        .output()?;

    let artifact_version = format!("{upstream_version}-pv1");
    let artifact_basename = format!("{resource}-{artifact_version}-darwin-arm64");
    let archive = out_dir.join(format!("{artifact_basename}.tar.gz"));
    let record = record_dir
        .join(resource)
        .join("1")
        .join(&artifact_version)
        .join("darwin-arm64")
        .join(format!("{artifact_basename}.json"));

    Ok(BackingBuildRecipeRun {
        out_dir: out_dir.to_string(),
        output,
        record_json: read_optional_file(&record)?,
        signed_files: codesigned_file_names(&read_file(&codesign_log)?),
        validate_log: read_file(&validate_log)?,
        archive_exists: path_exists(&archive),
    })
}

fn run_composer_build_recipe_smoke() -> Result<ComposerBuildRecipeRun> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let out_dir = tempdir.path().join("out");
    let record_dir = tempdir.path().join("records");
    let source_file = tempdir.path().join("composer.phar");
    let curl_log = tempdir.path().join("curl.log");
    let validate_log = tempdir.path().join("validate.log");

    create_dir_all(&fake_bin)?;
    write_file(&source_file, "composer fixture\n")?;
    write_fake_composer_cargo(&fake_bin.join("cargo"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_file(&curl_log, "")?;
    write_file(&validate_log, "")?;

    let build_script = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes/composer/build.sh");
    let output = StdCommand::new(build_script)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_ARTIFACT_OUT_DIR", &out_dir)
        .env("PV_ARTIFACT_RECORD_DIR", &record_dir)
        .env("PV_BUILD_RUN_ID", "local-test")
        .env("PV_COMMIT", "0123456789abcdef0123456789abcdef01234567")
        .env("PV_RECIPE_PLATFORM", "any")
        .env("PV_RECIPE_TRACK", "2")
        .env("PV_TEST_CURL_LOG", &curl_log)
        .env("PV_TEST_SOURCE_ARCHIVE", &source_file)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_file)?)
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .output()?;

    let artifact_version = "2.10.1-pv1";
    let platform_artifact_basename = "composer-2.10.1-pv1-any";
    let record = record_dir
        .join("composer")
        .join("2")
        .join(artifact_version)
        .join("any")
        .join(format!("{platform_artifact_basename}.json"));

    Ok(ComposerBuildRecipeRun {
        out_dir: out_dir.to_string(),
        output,
        record_json: read_optional_file(&record)?,
        curl_log: read_file(&curl_log)?,
        validate_log: read_file(&validate_log)?,
        platform_archive_exists: path_exists(
            &out_dir.join(format!("{platform_artifact_basename}.tar.gz")),
        ),
        legacy_archive_exists: path_exists(&out_dir.join("composer-2.10.1-pv1.tar.gz")),
    })
}

fn run_backing_build_recipe_signing_smoke(
    recipe: BackingBuildRecipe,
) -> Result<BackingBuildRecipeRun> {
    let tempdir = tempdir()?;
    let fake_bin = tempdir.path().join("bin");
    let out_dir = tempdir.path().join("out");
    let record_dir = tempdir.path().join("records");
    let source_archive = tempdir.path().join(format!("{}-source", recipe.resource));
    let curl_log = tempdir.path().join("curl.log");
    let sign_log = tempdir.path().join("codesign.log");
    let validate_log = tempdir.path().join("validate.log");

    create_dir_all(&fake_bin)?;
    write_backing_source_archive(&source_archive, recipe)?;
    write_fake_backing_cargo(&fake_bin.join("cargo"))?;
    write_fake_codesign(&fake_bin.join("codesign"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_fake_lipo(&fake_bin.join("lipo"))?;
    write_fake_make(&fake_bin.join("make"))?;
    write_fake_otool(&fake_bin.join("otool"))?;
    write_fake_sysctl(&fake_bin.join("sysctl"))?;
    write_fake_uname(&fake_bin.join("uname"))?;
    write_fake_unzip(&fake_bin.join("unzip"))?;
    write_file(&curl_log, "")?;
    write_file(&sign_log, "")?;
    write_file(&validate_log, "")?;

    let build_script = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../release/artifacts/recipes")
        .join(recipe.resource)
        .join("build.sh");
    let output = StdCommand::new(build_script)
        .env("PATH", format!("{fake_bin}:/usr/bin:/bin:/usr/sbin:/sbin"))
        .env("PV_ARTIFACT_OUT_DIR", &out_dir)
        .env("PV_ARTIFACT_RECORD_DIR", &record_dir)
        .env("PV_BUILD_RUN_ID", "local-test")
        .env("PV_COMMIT", "0123456789abcdef0123456789abcdef01234567")
        .env("PV_RECIPE_PLATFORM", recipe.platform)
        .env("PV_RECIPE_TRACK", recipe.track)
        .env("PV_TEST_ARTIFACT_VERSION", recipe.artifact_version)
        .env("PV_TEST_CODESIGN_LOG", &sign_log)
        .env("PV_TEST_CURL_LOG", &curl_log)
        .env("PV_TEST_LIPO_ARCHS", "arm64")
        .env("PV_TEST_MACHO_LIBRARIES", "")
        .env("PV_TEST_MACHO_MINOS", "13.0")
        .env("PV_TEST_MACHO_RPATHS", "")
        .env("PV_TEST_RESOURCE", recipe.resource)
        .env("PV_TEST_SOURCE_ARCHIVE", &source_archive)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_archive)?)
        .env("PV_TEST_UPSTREAM_VERSION", recipe.upstream_version)
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .output()?;

    let artifact_basename = recipe.artifact_basename();
    let archive = out_dir.join(format!("{artifact_basename}.tar.gz"));
    let record = record_dir
        .join(recipe.resource)
        .join(recipe.track)
        .join(recipe.artifact_version)
        .join(recipe.platform)
        .join(format!("{artifact_basename}.json"));

    Ok(BackingBuildRecipeRun {
        out_dir: out_dir.to_string(),
        output,
        record_json: read_optional_file(&record)?,
        signed_files: codesigned_file_names(&read_file(&sign_log)?),
        validate_log: read_file(&validate_log)?,
        archive_exists: path_exists(&archive),
    })
}

fn default_build_recipe_options() -> BuildRecipeOptions<'static> {
    BuildRecipeOptions {
        recipe_track: "8.4",
        php_version: "8.4.20",
        frankenphp_version: "1.12.3",
        lipo_archs: "arm64",
        macho_minos: "13.0",
        macho_libraries: "",
        macho_rpaths: "",
        frankenphp_macho_libraries: "",
        frankenphp_macho_rpaths: "",
        validate_archive_failure_resource: "",
        require_staticphp_php83_frankenphp_patch_context: false,
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
    let deleted_rpath_log = tempdir.path().join("deleted-rpaths.log");
    let install_name_log = tempdir.path().join("install-name.log");
    let removed_rpaths_log = tempdir.path().join("removed-rpaths.log");

    create_dir_all(&fake_bin)?;
    write_source_archive(&source_archive, "frankenphp-source")?;
    if options.require_staticphp_php83_frankenphp_patch_context {
        write_php_source_archive(&php_source_archive)?;
    } else {
        write_source_archive(&php_source_archive, "php-source")?;
    }
    write_fake_cargo(&fake_bin.join("cargo"))?;
    write_fake_curl(&fake_bin.join("curl"))?;
    write_fake_install_name_tool(&fake_bin.join("install_name_tool"))?;
    write_fake_lipo(&fake_bin.join("lipo"))?;
    write_fake_otool(&fake_bin.join("otool"))?;
    write_fake_spc(&fake_bin.join("spc"))?;
    write_file(&curl_log, "")?;
    write_file(&deleted_rpath_log, "")?;
    write_file(&install_name_log, "")?;
    write_file(&removed_rpaths_log, "")?;
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
        .env("PV_RECIPE_TRACK", options.recipe_track)
        .env("PV_TEST_FRANKENPHP_VERSION", options.frankenphp_version)
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
        .env("PV_TEST_DELETED_RPATH_LOG", &deleted_rpath_log)
        .env("PV_TEST_INSTALL_NAME_LOG", &install_name_log)
        .env("PV_TEST_PHP_SOURCE_ARCHIVE", &php_source_archive)
        .env(
            "PV_TEST_PHP_SOURCE_SHA256",
            file_sha256(&php_source_archive)?,
        )
        .env("PV_TEST_PHP_VERSION", options.php_version)
        .env("PV_TEST_REMOVED_RPATHS_LOG", &removed_rpaths_log)
        .env(
            "PV_TEST_REQUIRE_STATICPHP_PHP83_FRANKENPHP_PATCH_CONTEXT",
            if options.require_staticphp_php83_frankenphp_patch_context {
                "1"
            } else {
                ""
            },
        )
        .env("PV_TEST_SOURCE_ARCHIVE", &source_archive)
        .env("PV_TEST_SOURCE_SHA256", file_sha256(&source_archive)?)
        .env(
            "PV_TEST_STATICPHP_PHP83_AVX512_PATCH",
            php_staticphp_avx512_patch(),
        )
        .env(
            "PV_TEST_VALIDATE_ARCHIVE_FAILURE_RESOURCE",
            options.validate_archive_failure_resource,
        )
        .env("PV_TEST_VALIDATE_LOG", &validate_log)
        .env("PV_TEST_SPC_LOG", &spc_log);
    let output = command.output()?;

    let php_artifact_version = format!("{}-pv1", options.php_version);
    let php_artifact_basename = format!("php-{php_artifact_version}-darwin-arm64");
    let php_archive = out_dir.join(format!("{php_artifact_basename}.tar.gz"));
    let php_record = record_dir
        .join("php")
        .join(options.recipe_track)
        .join(&php_artifact_version)
        .join("darwin-arm64")
        .join(format!("{php_artifact_basename}.json"));
    let php_notice = out_dir
        .join("work")
        .join(format!("php-pair-{}-darwin-arm64", options.recipe_track))
        .join(php_artifact_basename)
        .join("NOTICE");

    let frankenphp_artifact_version = format!(
        "{}-frankenphp{}-pv1",
        options.php_version, options.frankenphp_version
    );
    let frankenphp_artifact_basename =
        format!("frankenphp-{frankenphp_artifact_version}-darwin-arm64");
    let frankenphp_archive = out_dir.join(format!("{frankenphp_artifact_basename}.tar.gz"));
    let frankenphp_record = record_dir
        .join("frankenphp")
        .join(options.recipe_track)
        .join(&frankenphp_artifact_version)
        .join("darwin-arm64")
        .join(format!("{frankenphp_artifact_basename}.json"));
    let frankenphp_notice = out_dir
        .join("work")
        .join(format!("php-pair-{}-darwin-arm64", options.recipe_track))
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
        deleted_rpath_log: read_file(&deleted_rpath_log)?,
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

fn mysql_record_source_inputs(run: &MysqlBuildRecipeRun) -> Result<Value> {
    Ok(build_recipe_record_provenance(run.record_json.as_deref())?
        .get("source_inputs")
        .cloned()
        .unwrap_or(Value::Array(Vec::new())))
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

php_version=${PV_TEST_PHP_VERSION:-8.4.20}
frankenphp_version=${PV_TEST_FRANKENPHP_VERSION:-1.12.3}

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
          upstream_version=$php_version
          artifact_version=$php_version-pv1
          source_url=https://sources.example.test/php.tar.gz
          source_sha256=$PV_TEST_PHP_SOURCE_SHA256
          php_source_env=
          ;;
        frankenphp)
          upstream_version=$php_version-frankenphp$frankenphp_version
          artifact_version=$php_version-frankenphp$frankenphp_version-pv1
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
PV_PHP_VERSION=$php_version
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
    write-release-record)
      record=
      object_key=
      source_url=
      source_sha256=
      recipe=
      pv_commit=
      build_run_id=
      source_inputs_json=
      source_input_count=0
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --record)
            shift
            record=${1:-}
            ;;
          --object-key)
            shift
            object_key=${1:-}
            ;;
          --source-url)
            shift
            source_url=${1:-}
            ;;
          --source-sha256)
            shift
            source_sha256=${1:-}
            ;;
          --recipe)
            shift
            recipe=${1:-}
            ;;
          --pv-commit)
            shift
            pv_commit=${1:-}
            ;;
          --build-run-id)
            shift
            build_run_id=${1:-}
            ;;
          --source-input)
            shift
            input_name=${1:-}
            shift
            input_url=${1:-}
            shift
            input_sha256=${1:-}
            input_json="      {\"name\": \"$input_name\", \"source_url\": \"$input_url\", \"source_sha256\": \"$input_sha256\"}"
            if [ "$source_input_count" -eq 0 ]; then
              source_inputs_json=$input_json
            else
              source_inputs_json="$source_inputs_json,
$input_json"
            fi
            source_input_count=$((source_input_count + 1))
            ;;
        esac
        shift
      done
      mkdir -p "$(dirname "$record")"
      {
        printf '{\n  "object_key": "%s",\n  "provenance": {\n' "$object_key"
        printf '    "source_url": "%s",\n' "$source_url"
        printf '    "source_sha256": "%s",\n' "$source_sha256"
        if [ "$source_input_count" -gt 0 ]; then
          printf '    "source_inputs": [\n%s\n    ],\n' "$source_inputs_json"
        fi
        printf '    "recipe": "%s",\n' "$recipe"
        printf '    "pv_commit": "%s",\n' "$pv_commit"
        printf '    "build_run_id": "%s"\n' "$build_run_id"
        printf '  }\n}\n'
      } >"$record"
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

fn write_fake_redis_cargo(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

json_array() {
  first=true
  printf '['
  for value in "$@"; do
    if [ "$first" = true ]; then
      first=false
    else
      printf ', '
    fi
    printf '"%s"' "$value"
  done
  printf ']'
}

if [ "$#" -ge 5 ] && [ "$1" = "run" ] && [ "$2" = "-p" ] && [ "$3" = "pv-release" ] && [ "$4" = "--" ]; then
  case "$5" in
    print-recipe-env)
      cat <<EOF
PV_RESOURCE=redis
PV_TRACK=$PV_RECIPE_TRACK
PV_PLATFORM=$PV_RECIPE_PLATFORM
PV_UPSTREAM_VERSION=8.2.7
PV_ARTIFACT_VERSION=8.2.7-pv1
PV_SOURCE_URL=https://sources.example.test/redis.tar.gz
PV_SOURCE_SHA256=$PV_TEST_SOURCE_SHA256
PV_PV_BUILD_REVISION=pv1
PV_MINIMUM_PV_VERSION=0.1.0
EOF
      ;;
    write-release-record)
      record=
      object_key=
      source_url=
      source_sha256=
      recipe=
      pv_commit=
      build_run_id=
      license_files=
      notice_files=
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --record)
            shift
            record=${1:-}
            ;;
          --object-key)
            shift
            object_key=${1:-}
            ;;
          --source-url)
            shift
            source_url=${1:-}
            ;;
          --source-sha256)
            shift
            source_sha256=${1:-}
            ;;
          --recipe)
            shift
            recipe=${1:-}
            ;;
          --pv-commit)
            shift
            pv_commit=${1:-}
            ;;
          --build-run-id)
            shift
            build_run_id=${1:-}
            ;;
          --license-file)
            shift
            license_files="${license_files}${license_files:+
}${1:-}"
            ;;
          --notice-file)
            shift
            notice_files="${notice_files}${notice_files:+
}${1:-}"
            ;;
        esac
        shift
      done
      [ -n "$license_files" ] || license_files=LICENSE
      [ -n "$notice_files" ] || notice_files=NOTICE
      set -- $license_files
      license_json=$(json_array "$@")
      set -- $notice_files
      notice_json=$(json_array "$@")
      mkdir -p "$(dirname "$record")"
      {
        printf '{\n'
        printf '  "object_key": "%s",\n' "$object_key"
        printf '  "license_files": %s,\n' "$license_json"
        printf '  "notice_files": %s,\n' "$notice_json"
        printf '  "provenance": {\n'
        printf '    "source_url": "%s",\n' "$source_url"
        printf '    "source_sha256": "%s",\n' "$source_sha256"
        printf '    "recipe": "%s",\n' "$recipe"
        printf '    "pv_commit": "%s",\n' "$pv_commit"
        printf '    "build_run_id": "%s"\n' "$build_run_id"
        printf '  }\n}\n'
      } >"$record"
      ;;
    validate-archive)
      archive=
      record=
      smoke_hook=
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
          --smoke-hook)
            shift
            smoke_hook=${1:-}
            ;;
        esac
        shift
      done
      printf 'archive=%s record=%s smoke=%s\n' \
        "${archive##*/}" \
        "${record##*/}" \
        "${smoke_hook##*/}" >>"$PV_TEST_VALIDATE_LOG"
      if [ -n "$PV_TEST_VALIDATE_ARCHIVE_FAILURE" ]; then
        printf 'validate-archive failed for %s\n' "${archive##*/}" >&2
        exit 79
      fi
      ;;
    *) exit 77 ;;
  esac
else
  exit 77
fi
"#,
    )
}

fn write_fake_backing_cargo(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

if [ "$#" -ge 5 ] && [ "$1" = "run" ] && [ "$2" = "-p" ] && [ "$3" = "pv-release" ] && [ "$4" = "--" ]; then
  case "$5" in
    print-recipe-env)
      cat <<EOF
PV_RESOURCE=$PV_TEST_RESOURCE
PV_TRACK=$PV_RECIPE_TRACK
PV_PLATFORM=$PV_RECIPE_PLATFORM
PV_UPSTREAM_VERSION=$PV_TEST_UPSTREAM_VERSION
PV_ARTIFACT_VERSION=$PV_TEST_UPSTREAM_VERSION-pv1
PV_SOURCE_URL=https://sources.example.test/$PV_TEST_RESOURCE
PV_SOURCE_SHA256=$PV_TEST_SOURCE_SHA256
PV_PV_BUILD_REVISION=pv1
PV_MINIMUM_PV_VERSION=0.1.0
EOF
      ;;
    write-release-record)
      record=
      object_key=
      source_url=
      source_sha256=
      recipe=
      pv_commit=
      build_run_id=
      source_inputs_json=
      source_input_count=0
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --record)
            shift
            record=${1:-}
            ;;
          --object-key)
            shift
            object_key=${1:-}
            ;;
          --source-url)
            shift
            source_url=${1:-}
            ;;
          --source-sha256)
            shift
            source_sha256=${1:-}
            ;;
          --recipe)
            shift
            recipe=${1:-}
            ;;
          --pv-commit)
            shift
            pv_commit=${1:-}
            ;;
          --build-run-id)
            shift
            build_run_id=${1:-}
            ;;
          --source-input)
            shift
            input_name=${1:-}
            shift
            input_url=${1:-}
            shift
            input_sha256=${1:-}
            input_json="      {\"name\": \"$input_name\", \"source_url\": \"$input_url\", \"source_sha256\": \"$input_sha256\"}"
            if [ "$source_input_count" -eq 0 ]; then
              source_inputs_json=$input_json
            else
              source_inputs_json="$source_inputs_json,
$input_json"
            fi
            source_input_count=$((source_input_count + 1))
            ;;
        esac
        shift
      done
      mkdir -p "$(dirname "$record")"
      {
        printf '{\n  "object_key": "%s",\n  "provenance": {\n' "$object_key"
        printf '    "source_url": "%s",\n' "$source_url"
        printf '    "source_sha256": "%s",\n' "$source_sha256"
        if [ "$source_input_count" -gt 0 ]; then
          printf '    "source_inputs": [\n%s\n    ],\n' "$source_inputs_json"
        fi
        printf '    "recipe": "%s",\n' "$recipe"
        printf '    "pv_commit": "%s",\n' "$pv_commit"
        printf '    "build_run_id": "%s"\n' "$build_run_id"
        printf '  }\n}\n'
      } >"$record"
      ;;
    validate-archive)
      archive=
      record=
      smoke_hook=
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
          --smoke-hook)
            shift
            smoke_hook=${1:-}
            ;;
        esac
        shift
      done
      printf 'archive=%s record=%s smoke=%s\n' \
        "${archive##*/}" \
        "${record##*/}" \
        "${smoke_hook##*/}" >>"$PV_TEST_VALIDATE_LOG"
      ;;
    *) exit 77 ;;
  esac
else
  exit 77
fi
"#,
    )
}

fn write_fake_composer_cargo(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

if [ "$#" -ge 5 ] && [ "$1" = "run" ] && [ "$2" = "-p" ] && [ "$3" = "pv-release" ] && [ "$4" = "--" ]; then
  case "$5" in
    print-recipe-env)
      cat <<EOF
PV_RESOURCE=composer
PV_TRACK=2
PV_PLATFORM=any
PV_UPSTREAM_VERSION=2.10.1
PV_ARTIFACT_VERSION=2.10.1-pv1
PV_SOURCE_URL=https://sources.example.test/composer.phar
PV_SOURCE_SHA256=$PV_TEST_SOURCE_SHA256
PV_MINIMUM_PV_VERSION=0.1.0
PV_PV_BUILD_REVISION=pv1
EOF
      ;;
    write-release-record)
      record=
      object_key=
      source_url=
      source_sha256=
      recipe=
      pv_commit=
      build_run_id=
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --record)
            shift
            record=${1:-}
            ;;
          --object-key)
            shift
            object_key=${1:-}
            ;;
          --source-url)
            shift
            source_url=${1:-}
            ;;
          --source-sha256)
            shift
            source_sha256=${1:-}
            ;;
          --recipe)
            shift
            recipe=${1:-}
            ;;
          --pv-commit)
            shift
            pv_commit=${1:-}
            ;;
          --build-run-id)
            shift
            build_run_id=${1:-}
            ;;
        esac
        shift
      done
      mkdir -p "$(dirname "$record")"
      cat >"$record" <<EOF
{
  "object_key": "$object_key",
  "provenance": {
    "source_url": "$source_url",
    "source_sha256": "$source_sha256",
    "recipe": "$recipe",
    "pv_commit": "$pv_commit",
    "build_run_id": "$build_run_id"
  }
}
EOF
      ;;
    validate-archive)
      archive=
      record=
      smoke_hook=
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
          --smoke-hook)
            shift
            smoke_hook=${1:-}
            ;;
        esac
        shift
      done
      printf 'archive=%s record=%s smoke=%s\n' \
        "${archive##*/}" \
        "${record##*/}" \
        "${smoke_hook##*/}" >>"$PV_TEST_VALIDATE_LOG"
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
  https://sources.example.test/openssl.tar.gz)
    cp "$PV_TEST_OPENSSL_SOURCE_ARCHIVE" "$output"
    ;;
  https://sources.example.test/boost.tar.gz)
    cp "$PV_TEST_BOOST_SOURCE_ARCHIVE" "$output"
    ;;
  *)
    cp "$PV_TEST_SOURCE_ARCHIVE" "$output"
    ;;
esac
"#,
    )
}

fn write_fake_brew(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

case "${1:-}" in
  --prefix)
    case "${2:-}" in
      bison)
        printf '%s\n' "$PV_TEST_BISON_PREFIX"
        ;;
      openssl@3)
        printf '%s\n' "$PV_TEST_OPENSSL_PREFIX"
        ;;
      *)
        exit 78
        ;;
    esac
    ;;
  install)
    case "${2:-}" in
      bison)
        mkdir -p "$PV_TEST_BISON_PREFIX/bin"
        printf '#!/bin/sh\n' >"$PV_TEST_BISON_EXECUTABLE"
        chmod 755 "$PV_TEST_BISON_EXECUTABLE"
        ;;
      openssl@3)
        mkdir -p "$PV_TEST_OPENSSL_PREFIX/include/openssl"
        printf '%s\n' 'openssl fixture' >"$PV_TEST_OPENSSL_PREFIX/include/openssl/ssl.h"
        ;;
      *)
        exit 78
        ;;
    esac
    ;;
  *)
    exit 78
    ;;
esac
"#,
    )
}

fn write_fake_success_curl(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
exit 0
"#,
    )
}

fn write_fake_mailpit(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

case "${1:-}" in
  version)
    printf '%s\n' "${PV_TEST_MAILPIT_VERSION_OUTPUT:-Mailpit v1.30.1}"
    exit 0
    ;;
esac

smtp=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --smtp)
      shift
      smtp=${1:-}
      ;;
  esac
  shift
done

[ -n "$smtp" ] || exit 78
smtp_port=${smtp##*:}
export PV_TEST_MAILPIT_SMTP_PORT=$smtp_port
exec python3 - <<'PY'
import os
import signal
import socket
import time

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
sock.bind(("127.0.0.1", int(os.environ["PV_TEST_MAILPIT_SMTP_PORT"])))
sock.listen(1)

if os.environ.get("PV_TEST_MAILPIT_IGNORE_TERM"):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)

time.sleep(float(os.environ.get("PV_TEST_MAILPIT_SLEEP_SECONDS", "60")))
PY
"#,
    )
}

fn write_fake_rustfs(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

case "${1:-}" in
  --version)
    printf '%s\n' "${PV_TEST_RUSTFS_VERSION_OUTPUT:-rustfs 1.0.0-beta.7}"
    exit 0
    ;;
  server)
    shift
    ;;
  *)
    exit 78
    ;;
esac

address=
console_address=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --address)
      shift
      address=${1:-}
      ;;
    --console-address)
      shift
      console_address=${1:-}
      ;;
    --access-key | --secret-key)
      shift
      ;;
  esac
  shift
done

[ -n "$address" ] || exit 78
if [ -n "${PV_TEST_RUSTFS_REQUIRE_CONSOLE_ADDRESS:-}" ]; then
  case "$console_address" in
    127.0.0.1:*) ;;
    *) exit 73 ;;
  esac
fi
if [ -n "${PV_TEST_RUSTFS_LOG:-}" ]; then
  printf 'address=%s console=%s\n' "$address" "$console_address" >>"$PV_TEST_RUSTFS_LOG"
fi

export PV_TEST_RUSTFS_ADDRESS=$address
export PV_TEST_RUSTFS_CONSOLE_ADDRESS=$console_address
exec python3 - <<'PY'
import http.server
import os
import signal
import socket
import threading
import time

bucket_names = set()

def split_address(address):
    host, port = address.rsplit(":", 1)
    return host, int(port)

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = "\n".join(sorted(bucket_names)).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_PUT(self):
        bucket_names.add(self.path.strip("/"))
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, _format, *_args):
        pass

api_host, api_port = split_address(os.environ["PV_TEST_RUSTFS_ADDRESS"])
server = http.server.HTTPServer((api_host, api_port), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()

console_address = os.environ.get("PV_TEST_RUSTFS_CONSOLE_ADDRESS")
console_socket = None
if console_address:
    console_host, console_port = split_address(console_address)
    console_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    console_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    console_socket.bind((console_host, console_port))
    console_socket.listen(1)

if os.environ.get("PV_TEST_RUSTFS_IGNORE_TERM"):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)

time.sleep(float(os.environ.get("PV_TEST_RUSTFS_SLEEP_SECONDS", "60")))
PY
"#,
    )
}

fn write_fake_mysql_server(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

for arg in "$@"; do
  case "$arg" in
    --initialize-insecure)
      exit 0
      ;;
  esac
done

exit 0
"#,
    )
}

fn write_fake_mysqladmin_requires_tcp(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

protocol=
command=
for arg in "$@"; do
  case "$arg" in
    --protocol=*)
      protocol=${arg#--protocol=}
      ;;
    ping | shutdown)
      command=$arg
      ;;
  esac
done

[ "$protocol" = "tcp" ] || exit 70
case "$command" in
  ping | shutdown) exit 0 ;;
  *) exit 78 ;;
esac
"#,
    )
}

fn write_fake_mysql_requires_tcp(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

protocol=
host=
port=
for arg in "$@"; do
  case "$arg" in
    --protocol=*)
      protocol=${arg#--protocol=}
      ;;
    --host=*)
      host=${arg#--host=}
      ;;
    --port=*)
      port=${arg#--port=}
      ;;
  esac
done

[ "$protocol" = "tcp" ] || exit 70
[ "$host" = "127.0.0.1" ] || exit 71
[ -n "$port" ] || exit 72
printf '%s\n' 1
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
        if [ -n "${PV_TEST_REMOVED_RPATHS_LOG:-}" ] \
          && grep -Fqx "$binary|$macho_rpath" "$PV_TEST_REMOVED_RPATHS_LOG"; then
          continue
        fi
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

fn write_fake_make(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

build_dir=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -C)
      shift
      build_dir=${1:-}
      ;;
  esac
  shift
done

[ -n "$build_dir" ] || exit 78
mkdir -p "$build_dir/src"
cat >"$build_dir/src/redis-server" <<'EOF'
redis-server fixture
EOF
cat >"$build_dir/src/redis-cli" <<'EOF'
redis-cli fixture
EOF
chmod 755 "$build_dir/src/redis-server" "$build_dir/src/redis-cli"
"#,
    )
}

fn write_fake_mysql_make(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

printf 'make=' >>"$PV_TEST_OPENSSL_BUILD_LOG"
for arg in "$@"; do
  printf '[%s]' "$arg" >>"$PV_TEST_OPENSSL_BUILD_LOG"
done
printf '\n' >>"$PV_TEST_OPENSSL_BUILD_LOG"

[ -f .pv-openssl-prefix ] || exit 78
case "${1:-}" in
  install_sw)
    openssl_prefix=$(cat .pv-openssl-prefix)
    mkdir -p "$openssl_prefix/include/openssl" "$openssl_prefix/lib"
    printf '%s\n' 'openssl fixture' >"$openssl_prefix/include/openssl/ssl.h"
    printf '%s\n' 'libssl fixture' >"$openssl_prefix/lib/libssl.3.dylib"
    printf '%s\n' 'libcrypto fixture' >"$openssl_prefix/lib/libcrypto.3.dylib"
    ;;
  -j)
    [ -n "${2:-}" ] || exit 78
    ;;
  *)
    exit 78
    ;;
esac
"#,
    )
}

fn write_fake_openssl_perl(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

configure_script=${1:-}
shift || true
[ "${configure_script##*/}" = "Configure" ] || exit 78

configure_target=${1:-}
shift || true
prefix=
for arg in "$@"; do
  case "$arg" in
    --prefix=*)
      prefix=${arg#--prefix=}
      ;;
  esac
done

[ -n "$configure_target" ] || exit 78
[ -n "$prefix" ] || exit 78
[ "$MACOSX_DEPLOYMENT_TARGET" = "13.0" ] || exit 79

printf 'configure-target=%s deployment=%s prefix=%s\n' \
  "$configure_target" \
  "$MACOSX_DEPLOYMENT_TARGET" \
  "$prefix" >>"$PV_TEST_OPENSSL_BUILD_LOG"
printf '%s\n' "$prefix" >.pv-openssl-prefix
"#,
    )
}

fn write_fake_mysql_cmake(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

printf 'argv=' >>"$PV_TEST_CMAKE_LOG"
for arg in "$@"; do
  printf '[%s]' "$arg" >>"$PV_TEST_CMAKE_LOG"
done
printf '\n' >>"$PV_TEST_CMAKE_LOG"

case "${1:-}" in
  -S)
    build_dir=
    install_dir=
    bison_executable=
    with_ssl=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        -B)
          shift
          build_dir=${1:-}
          ;;
        -DCMAKE_INSTALL_PREFIX=*)
          install_dir=${1#-DCMAKE_INSTALL_PREFIX=}
          ;;
        -DBISON_EXECUTABLE=*)
          bison_executable=${1#-DBISON_EXECUTABLE=}
          ;;
        -DWITH_SSL=*)
          with_ssl=${1#-DWITH_SSL=}
          ;;
      esac
      shift
    done
    [ -n "$build_dir" ] || exit 78
    [ -n "$install_dir" ] || exit 78
    [ "$bison_executable" = "$PV_TEST_BISON_EXECUTABLE" ] || exit 81
    [ "$with_ssl" = "$PV_TEST_OPENSSL_PREFIX" ] || exit 82
    mkdir -p "$build_dir"
    printf '%s\n' "$install_dir" >"$build_dir/install-prefix"
    ;;
  --build)
    [ -n "${2:-}" ] || exit 78
    ;;
  --install)
    build_dir=${2:-}
    [ -n "$build_dir" ] || exit 78
    install_dir=$(cat "$build_dir/install-prefix")
    mkdir -p "$install_dir/bin" "$install_dir/lib"
    for binary in mysqld mysql mysqladmin; do
      printf '%s fixture\n' "$binary" >"$install_dir/bin/$binary"
      chmod 755 "$install_dir/bin/$binary"
    done
    if [ -n "${PV_TEST_MYSQL_INSTALL_BROKEN_PLUGIN_SYMLINK:-}" ]; then
      mkdir -p "$install_dir/lib/plugin"
      ln -s ../../lib/libfido2.1.dylib "$install_dir/lib/plugin/authentication_fido_client.so"
    fi
    if [ -n "${PV_TEST_MYSQL_INSTALL_BROKEN_REQUIRED_SYMLINK:-}" ]; then
      ln -s missing-libmysqlclient.dylib "$install_dir/lib/libmysqlclient-required.dylib"
    fi
    ;;
  *)
    exit 78
    ;;
esac
"#,
    )
}

fn write_fake_sysctl(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

[ "${1:-}" = "-n" ] || exit 78
[ "${2:-}" = "hw.ncpu" ] || exit 78
printf '%s\n' 1
"#,
    )
}

fn write_fake_uname(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

case "${1:-}" in
  -s) printf '%s\n' Darwin ;;
  -m) printf '%s\n' arm64 ;;
  *) exit 78 ;;
esac
"#,
    )
}

fn write_fake_unzip(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

extract_dir=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -d)
      shift
      extract_dir=${1:-}
      ;;
  esac
  shift
done

[ -n "$extract_dir" ] || exit 78
mkdir -p "$extract_dir"
cat >"$extract_dir/rustfs" <<'EOF'
rustfs fixture
EOF
chmod 755 "$extract_dir/rustfs"
"#,
    )
}

fn write_fake_signing_otool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

[ "${1:-}" = "-L" ] || exit 78
case "${2##*/}" in
  mysql | mysqladmin | libmysqlclient.dylib)
    printf '%s:\n' "$2"
    ;;
  *)
    exit 1
    ;;
esac
"#,
    )
}

fn write_fake_install_name_otool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

mode=${1:-}
binary=${2:-}

case "$mode" in
  -D)
    printf '%s:\n' "$binary"
    case "$binary" in
      */lib/libmysqlclient.dylib)
        printf '%s/lib/libmysqlclient.dylib\n' "$PV_TEST_INSTALL_DIR"
        ;;
      */lib/libssl.3.dylib)
        printf '%s/lib/libssl.3.dylib\n' "$PV_TEST_OPENSSL_PREFIX"
        ;;
    esac
    ;;
  -L)
    case "$binary" in
      */bin/mysql | */lib/libmysqlclient.dylib | */lib/libssl.3.dylib | */lib/plugin/auth.so | */lib/postgresql/extension.so)
        printf '%s:\n' "$binary"
        printf '\t%s/lib/libmysqlclient.dylib (compatibility version 1.0.0, current version 1.0.0)\n' "$PV_TEST_INSTALL_DIR"
        printf '\t%s/lib/libssl.3.dylib (compatibility version 3.0.0, current version 3.6.2)\n' "$PV_TEST_OPENSSL_PREFIX"
        ;;
      *)
        exit 1
        ;;
    esac
    ;;
  *)
    exit 78
    ;;
esac
"#,
    )
}

fn write_fake_install_name_tool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

printf 'argv=' >>"$PV_TEST_INSTALL_NAME_LOG"
for arg in "$@"; do
  printf '[%s]' "$arg" >>"$PV_TEST_INSTALL_NAME_LOG"
done
printf '\n' >>"$PV_TEST_INSTALL_NAME_LOG"

if [ "${1:-}" = "-delete_rpath" ]; then
  printf '%s\n' "${2:-}" >>"$PV_TEST_DELETED_RPATH_LOG"
  printf '%s|%s\n' "${3:-}" "${2:-}" >>"$PV_TEST_REMOVED_RPATHS_LOG"
fi
"#,
    )
}

fn write_fake_rewrite_failure_otool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

mode=${1:-}
binary=${2:-}

case "$mode" in
  -D)
    printf '%s:\n' "$binary"
    printf '%s/lib/%s\n' "$PV_TEST_INSTALL_DIR" "${binary##*/}"
    ;;
  -L)
    printf '%s:\n' "$binary"
    ;;
  *)
    exit 78
    ;;
esac
"#,
    )
}

fn write_fake_first_call_failing_install_name_tool(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

count=$(cat "$PV_TEST_INSTALL_NAME_COUNT")
count=$((count + 1))
printf '%s\n' "$count" >"$PV_TEST_INSTALL_NAME_COUNT"
[ "$count" -ne 1 ] || exit 71
exit 0
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

if [ -n "${PV_TEST_REQUIRE_STATICPHP_PHP83_FRANKENPHP_PATCH_CONTEXT:-}" ]; then
  frankenphp_source_dir=
  previous_arg=
  for arg in "$@"; do
    if [ "$previous_arg" = "--dl-custom-local" ]; then
      case "$arg" in
        frankenphp:*)
          frankenphp_source_dir=${arg#frankenphp:}
          ;;
      esac
    fi
    previous_arg=$arg
  done

  [ -n "$frankenphp_source_dir" ] || exit 79
  [ -f "$frankenphp_source_dir/build/php.m4" ] || exit 80
  patch --dry-run -R -d "$frankenphp_source_dir" -p1 \
    <"$PV_TEST_STATICPHP_PHP83_AVX512_PATCH" >/dev/null || exit 81
fi

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

fn write_fake_codesign(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
printf 'argv=' >>"$PV_TEST_CODESIGN_LOG"
for arg in "$@"; do
  printf '[%s]' "$arg" >>"$PV_TEST_CODESIGN_LOG"
done
printf '\n' >>"$PV_TEST_CODESIGN_LOG"
"#,
    )
}

fn write_fake_first_call_failing_codesign(path: &Utf8Path) -> Result<()> {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu

count=$(cat "$PV_TEST_CODESIGN_COUNT")
count=$((count + 1))
printf '%s\n' "$count" >"$PV_TEST_CODESIGN_COUNT"
[ "$count" -ne 1 ] || exit 71
exit 0
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
    clippy::disallowed_types,
    reason = "release tooling tests create source tarball fixtures directly"
)]
fn write_php_source_archive(path: &Utf8Path) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    append_archive_file(
        &mut builder,
        "php-source/build/php.m4",
        PHP_83_AVX512_ORIGINAL_M4.as_bytes(),
    )?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create source tarball fixtures directly"
)]
fn write_openssl_source_archive(path: &Utf8Path) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    append_archive_file(
        &mut builder,
        "openssl-source/Configure",
        b"openssl configure\n",
    )?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn write_backing_source_archive(path: &Utf8Path, recipe: BackingBuildRecipe) -> Result<()> {
    match recipe.source_kind {
        BackingSourceKind::Redis => write_redis_source_archive(path),
        BackingSourceKind::TarGzBinary => {
            write_single_file_archive(path, recipe.resource, b"binary fixture")
        }
        BackingSourceKind::ZipBinary => write_file(path, "zip fixture\n"),
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create source tarball fixtures directly"
)]
fn write_single_file_archive(path: &Utf8Path, entry_path: &str, content: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_path(entry_path)?;
    header.set_size(content.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append(&header, content)?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create source tarball fixtures directly"
)]
fn write_single_binary_source_archive(path: &Utf8Path, binary_name: &str) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    append_archive_file(&mut builder, binary_name, b"#!/bin/sh\n")?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn write_redis_source_archive(path: &Utf8Path) -> Result<()> {
    write_redis_source_archive_with_options(path, true)
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests create source tarball fixtures directly"
)]
fn write_redis_source_archive_with_options(
    path: &Utf8Path,
    include_fast_float_legal_header: bool,
) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    append_archive_file(
        &mut builder,
        "redis-source/src/redis-server",
        b"redis-server\n",
    )?;
    append_archive_file(&mut builder, "redis-source/src/redis-cli", b"redis-cli\n")?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/hiredis/COPYING",
        b"hiredis license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/lua/COPYRIGHT",
        b"lua license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/hdr_histogram/LICENSE.txt",
        b"hdr histogram bsd license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/hdr_histogram/COPYING.txt",
        b"hdr histogram cc0 license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/fpconv/LICENSE.txt",
        b"fpconv license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/src/fast_float_strtod.c",
        if include_fast_float_legal_header {
            b"/*\nfast_float notice\n*/\n".as_slice()
        } else {
            b"double fast_float_strtod(void) { return 0.0; }\n".as_slice()
        },
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/linenoise/README.markdown",
        b"linenoise notice\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/jemalloc/COPYING",
        b"jemalloc license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/tre/LICENSE",
        b"tre license\n",
    )?;
    append_archive_file(
        &mut builder,
        "redis-source/deps/xxhash/LICENSE",
        b"xxhash license\n",
    )?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn append_archive_file<W: std::io::Write>(
    builder: &mut Builder<W>,
    path: &str,
    content: &[u8],
) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_path(path)?;
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_types,
    reason = "release tooling tests inspect archive contents"
)]
fn archive_entries(path: &Utf8Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = Vec::new();

    for entry in archive.entries()? {
        let entry = entry?;
        entries.push(entry.path()?.to_string_lossy().into_owned());
    }

    Ok(entries)
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

fn codesigned_file_names(sign_log: &str) -> Vec<String> {
    let mut file_names = Vec::new();
    for line in sign_log.lines() {
        if let Some((_prefix, path)) = line.rsplit_once('[') {
            let path = path.trim_end_matches(']');
            if let Some(file_name) = Utf8Path::new(path).file_name() {
                file_names.push(file_name.to_string());
            }
        }
    }
    file_names.sort();
    file_names
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
            ReleaseError::SmokeHookTimedOut { hook, timeout } => Self {
                kind: "SmokeHookTimedOut",
                path: file_name(&hook),
                reason: timeout,
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
