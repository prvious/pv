# Task 5 Report: Move PHP Artifact Fallback Ini Paths Away From /usr/local

## What I Implemented

- Added StaticPHP build flags in `release/artifacts/recipes/php/build.sh`:
  - `--with-config-file-path=/var/empty/com.prvious.pv/php`
  - `--with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d`
- Updated `release/artifacts/recipes/php/smoke.sh` so standalone PHP rejects `/usr/local/etc/php` in `php --ini` output with exit code 46.
- Updated the FrankenPHP smoke page to include `phpinfo(INFO_CONFIGURATION)` and reject `/usr/local/etc/php` in the served response with exit code 46.
- Updated `crates/pv-release/tests/smoke.rs`:
  - Renamed the focused build smoke test to `php_build_recipe_smoke` so the required nextest filter runs it.
  - Required the safe StaticPHP fallback argv flags and rejected `/usr/local/etc/php` in the `spc` argv log.
  - Added fixture coverage for unsafe standalone `php --ini` output.
  - Added fixture coverage for unsafe FrankenPHP `phpinfo()` response output.
  - Updated the positive FrankenPHP fixture to verify the generated smoke page includes `phpinfo(INFO_CONFIGURATION)`.

## What I Tested and Results

- `cargo nextest run -p pv-release -E 'test(php_build_recipe_smoke)'` - PASS
- `cargo nextest run -p pv-release -E 'test(php_smoke_rejects_usr_local_ini_path_from_php_ini_output)'` - PASS
- `cargo nextest run -p pv-release -E 'test(php_smoke_rejects_usr_local_ini_path_from_frankenphp_response)'` - PASS
- `cargo nextest run -p pv-release -E 'test(php_smoke_validates_frankenphp_when_cli_binary_is_also_present)'` - PASS
- `cargo nextest run -p pv-release -E 'test(php_smoke_normalizes_realistic_module_output)'` - PASS
- `cargo nextest run -p pv-release -E 'test(php_smoke_allows_extra_extensions)'` - PASS
- `sh -n release/artifacts/recipes/php/build.sh` - PASS
- `sh -n release/artifacts/recipes/php/smoke.sh` - PASS
- `shellcheck release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh` - PASS
- `cargo fmt --all` - PASS
- `git diff --check` - PASS

## TDD Evidence

### RED: StaticPHP argv flags

Command:

```sh
cargo nextest run -p pv-release -E 'test(php_build_recipe_smoke)'
```

Result: FAIL as expected.

Key output:

```text
FAIL pv-release::smoke php_build_recipe_smoke
assertion `left == right` failed
left:  argv=[build:php][json][--build-cli][--build-frankenphp][--enable-zts][--dl-with-php=8.4.20]...
right: argv=[build:php][json][--build-cli][--build-frankenphp][--enable-zts][--with-config-file-path=/var/empty/com.prvious.pv/php][--with-config-file-scan-dir=/var/empty/com.prvious.pv/php/conf.d][--dl-with-php=8.4.20]...
```

Note: the task-specified filter initially matched zero tests because the existing test had a longer descriptive name. I renamed that existing test to `php_build_recipe_smoke`, then reran the same command to capture the meaningful RED above.

### RED: standalone PHP unsafe ini output

Command:

```sh
cargo nextest run -p pv-release -E 'test(php_smoke_rejects_usr_local_ini_path_from_php_ini_output)'
```

Result: FAIL as expected.

Key output:

```text
smoke hook should reject unsafe PHP ini fallback: (
    true,
    Some(0),
    "",
    "",
)
```

### RED: FrankenPHP unsafe ini output

Command:

```sh
cargo nextest run -p pv-release -E 'test(php_smoke_rejects_usr_local_ini_path_from_frankenphp_response)'
```

Result: FAIL as expected.

Key output:

```text
smoke hook should reject unsafe FrankenPHP ini fallback: (
    true,
    Some(0),
    "",
    "",
)
```

### RED: FrankenPHP phpinfo response coverage

Command:

```sh
cargo nextest run -p pv-release -E 'test(php_smoke_validates_frankenphp_when_cli_binary_is_also_present)'
```

Result: FAIL as expected.

Key output:

```text
smoke hook should serve phpinfo(INFO_CONFIGURATION): php-cli -r version
php-cli -r extensions
php-server 127.0.0.1:49647 missing-phpinfo
```

### GREEN

Command:

```sh
cargo nextest run -p pv-release -E 'test(php_build_recipe_smoke)'
```

Result:

```text
PASS pv-release::smoke php_build_recipe_smoke
Summary: 1 test run: 1 passed, 170 skipped
```

Command:

```sh
cargo nextest run -p pv-release -E 'test(php_smoke_rejects_usr_local_ini_path_from_php_ini_output)'
cargo nextest run -p pv-release -E 'test(php_smoke_rejects_usr_local_ini_path_from_frankenphp_response)'
cargo nextest run -p pv-release -E 'test(php_smoke_validates_frankenphp_when_cli_binary_is_also_present)'
```

Result: all PASS.

Shell checks:

```sh
sh -n release/artifacts/recipes/php/build.sh
sh -n release/artifacts/recipes/php/smoke.sh
shellcheck release/artifacts/recipes/php/build.sh release/artifacts/recipes/php/smoke.sh
```

Result: all PASS with no output.

## Files Changed

- `release/artifacts/recipes/php/build.sh`
- `release/artifacts/recipes/php/smoke.sh`
- `crates/pv-release/tests/smoke.rs`
- `.superpowers/sdd/task-5-report.md`

## Self-Review Findings

- Runtime Caddyfile environment handling and runtime `php_ini` behavior were not touched.
- The root `/Users/clovismuneza/Apps/pv/php.ini` sample was not modified or tracked.
- Shell changes preserve POSIX `sh` style and pass `sh -n` plus `shellcheck`.
- Rust changes are test-only and do not introduce `panic!`, `unreachable!`, `.unwrap()`, `.expect()`, unsafe code, or clippy rule ignores.
- Existing insta snapshots were preserved with explicit names after renaming the focused test for the required nextest filter.

## Concerns

- None outstanding. Verification was focused to the task-requested checks and related PHP smoke fixtures; the full workspace test suite was not run.
