# Task 3 Report: Point CLI PHP And Composer At Track Defaults

## What I implemented

- Updated the PHP shim so it ensures PHP track defaults before exec and appends `PHPRC` / `PHP_INI_SCAN_DIR` from `resources::php_track_exec_environment(&paths, &track)?`.
- Removed the old PHP shim release-path env overlay that pointed ini discovery at `~/.pv/resources/php/<track>/releases/<artifact-version>/etc`.
- Kept `InstalledPhp.release` and derive the executable path from the installed artifact release path.
- Updated PHP shim test helpers and assertions to expect track defaults under `~/.pv/resources/php/<track>/etc`.
- Updated Composer shim test helpers and all affected expected env calls to expect track defaults. Composer still execs through the PHP artifact binary via the PHP shim path.
- Added a PHP shim assertion that running the shim seeds the track default `php.ini` with `resources::PHP_TRACK_DEFAULT_INI`.
- Accepted affected insta snapshots for PHP and Composer shim env output.

## What I tested and results

- `cargo nextest run -p cli -E 'test(php_shim_sets_only_php_ini_env_overlay) | test(composer_shim_execs_installed_phar_through_php_shim) | test(composer_shim_sets_pv_owned_env_overlay)'`
  - Result: PASS, 3 passed.
- `cargo nextest run -p cli -E 'test(php_shim) | test(composer_shim)'`
  - Result: PASS, 12 passed.
- `cargo insta test --accept --test-runner nextest -p cli -- php_shim`
  - Result: PASS, 7 passed; accepted affected PHP shim snapshots and `composer_shim_execs_installed_phar_through_php_shim`.
- `cargo insta test --accept --test-runner nextest -p cli -- composer_shim`
  - Result: PASS, 6 passed; accepted remaining Composer shim snapshots.

## TDD Evidence

### RED

Command:

```shell
cargo nextest run -p cli -E 'test(php_shim_sets_only_php_ini_env_overlay) | test(composer_shim_execs_installed_phar_through_php_shim) | test(composer_shim_sets_pv_owned_env_overlay)'
```

Key output:

```text
Summary [0.038s] 3 tests run: 0 passed, 3 failed, 194 skipped
FAIL cli::php php_shim_sets_only_php_ini_env_overlay
FAIL cli::composer composer_shim_sets_pv_owned_env_overlay
FAIL cli::composer composer_shim_execs_installed_phar_through_php_shim
```

The expected failure showed `PHPRC` and `PHP_INI_SCAN_DIR` still coming from:

```text
~/.pv/resources/php/8.4/releases/8.4.8-pv1/etc
~/.pv/resources/php/8.4/releases/8.4.8-pv1/etc/conf.d
```

instead of:

```text
~/.pv/resources/php/8.4/etc
~/.pv/resources/php/8.4/etc/conf.d
```

### GREEN

Command:

```shell
cargo nextest run -p cli -E 'test(php_shim_sets_only_php_ini_env_overlay) | test(composer_shim_execs_installed_phar_through_php_shim) | test(composer_shim_sets_pv_owned_env_overlay)'
```

Output:

```text
Summary [0.039s] 3 tests run: 3 passed, 194 skipped
PASS cli::php php_shim_sets_only_php_ini_env_overlay
PASS cli::composer composer_shim_sets_pv_owned_env_overlay
PASS cli::composer composer_shim_execs_installed_phar_through_php_shim
```

Broader affected shim verification:

```shell
cargo nextest run -p cli -E 'test(php_shim) | test(composer_shim)'
```

Output:

```text
Summary [0.110s] 12 tests run: 12 passed, 185 skipped
```

## Files changed

- `crates/cli/src/commands/php.rs`
- `crates/cli/tests/php.rs`
- `crates/cli/tests/composer.rs`
- `crates/cli/tests/snapshots/composer__composer_shim_execs_installed_phar_through_php_shim.snap`
- `crates/cli/tests/snapshots/composer__composer_shim_forwards_help_and_version_flags.snap`
- `crates/cli/tests/snapshots/composer__composer_shim_uses_cached_manifest_default_without_network.snap`
- `crates/cli/tests/snapshots/php__php_shim_execs_global_default_track_outside_project.snap`
- `crates/cli/tests/snapshots/php__php_shim_execs_resolved_project_track.snap`
- `crates/cli/tests/snapshots/php__php_shim_forwards_help_and_version_flags.snap`
- `crates/cli/tests/snapshots/php__php_shim_uses_cached_manifest_default_without_network.snap`
- `.superpowers/sdd/task-3-report.md`

## Self-review findings

- Shim ini discovery now uses process-level `PHPRC` and `PHP_INI_SCAN_DIR`; no Caddyfile env directives were introduced.
- Composer expected env calls using the shared helper were updated, including help/version and cached-manifest cases beyond the three tests named in the brief.
- The PHP executable still comes from the installed artifact release path.
- Existing seeded `php.ini` preservation remains owned by `resources::ensure_php_track_defaults`; this task calls that API rather than reimplementing seeding.
- No unrelated files were reverted or staged.

## Concerns

None.
