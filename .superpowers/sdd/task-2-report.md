# Task 2 Report: Seed Defaults During PHP Pair Install And Update

## What I Implemented

- Added `ManagedResourceCommands::ensure_php_pair_defaults`.
- Call PHP default seeding before recording PHP pair install/update state.
- Call PHP default seeding before recording Composer-with-PHP-pair state.
- Added integration coverage for:
  - PHP pair install seeds `resources/php/<track>/etc/php.ini` and `conf.d`.
  - Composer-with-PHP-pair install seeds the PHP track defaults.
  - PHP pair update preserves an existing customized `php.ini`.

## What I Tested And Results

- `cargo insta test --accept --test-runner nextest -p resources -- managed_resource_commands_install_php_pair_seeds_track_defaults`
  - PASS: 1 test passed; snapshot accepted.
- `cargo insta test --accept --test-runner nextest -p resources -- managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults`
  - PASS: 1 test passed; snapshot accepted.
- `cargo insta test --accept --test-runner nextest -p resources -- managed_resource_commands_update_php_pairs_preserves_existing_php_ini`
  - PASS: 1 test passed.
- `cargo fmt --all`
  - PASS: no output.
- `cargo nextest run -p resources -E 'test(managed_resource_commands_install_php_pair_seeds_track_defaults) | test(managed_resource_commands_update_php_pairs_preserves_existing_php_ini) | test(managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults)'`
  - PASS: 3 tests passed; 118 skipped.
- `cargo nextest run -p resources --test managed_resource_commands`
  - PASS: 35 tests passed.
- `git diff --check`
  - PASS: no whitespace errors.

## TDD Evidence

### RED

Command:

```shell
cargo nextest run -p resources -E 'test(managed_resource_commands_install_php_pair_seeds_track_defaults) | test(managed_resource_commands_update_php_pairs_preserves_existing_php_ini) | test(managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults)'
```

Output summary:

```text
Starting 3 tests across 8 binaries
FAIL resources::managed_resource_commands managed_resource_commands_install_php_pair_seeds_track_defaults
Error: filesystem error at .../home/.pv/resources/php/8.4/etc/php.ini: No such file or directory (os error 2)

FAIL resources::managed_resource_commands managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults
Error: filesystem error at .../home/.pv/resources/php/8.4/etc/php.ini: No such file or directory (os error 2)

FAIL resources::managed_resource_commands managed_resource_commands_update_php_pairs_preserves_existing_php_ini
stored new snapshot ...managed_resource_commands_update_php_pairs_preserves_existing_php_ini.snap.new

Summary: 3 tests run: 0 passed, 3 failed, 118 skipped
error: test run failed
```

### GREEN

Command:

```shell
cargo nextest run -p resources -E 'test(managed_resource_commands_install_php_pair_seeds_track_defaults) | test(managed_resource_commands_update_php_pairs_preserves_existing_php_ini) | test(managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults)'
```

Output summary:

```text
Starting 3 tests across 8 binaries
PASS resources::managed_resource_commands managed_resource_commands_install_php_pair_seeds_track_defaults
PASS resources::managed_resource_commands managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults
PASS resources::managed_resource_commands managed_resource_commands_update_php_pairs_preserves_existing_php_ini
Summary: 3 tests run: 3 passed, 118 skipped
```

Broader focused verification:

```text
cargo nextest run -p resources --test managed_resource_commands
Summary: 35 tests run: 35 passed, 0 skipped
```

## Files Changed

- `crates/resources/src/command.rs`
- `crates/resources/tests/managed_resource_commands.rs`
- `crates/resources/tests/snapshots/managed_resource_commands__managed_resource_commands_install_php_pair_seeds_track_defaults.snap`
- `crates/resources/tests/snapshots/managed_resource_commands__managed_resource_commands_install_composer_with_php_pair_seeds_track_defaults.snap`
- `crates/resources/tests/snapshots/managed_resource_commands__managed_resource_commands_update_php_pairs_preserves_existing_php_ini.snap`
- `.superpowers/sdd/task-2-report.md`

## Self-Review Findings

- The default seeding happens before `Database::open` and before managed resource state is recorded.
- The same helper is used by install, update, and Composer-with-PHP-pair recording paths.
- Existing seeded `php.ini` content is preserved by the `ensure_php_track_defaults` helper.
- Tests use fallible `php_track_defaults(...)?` per the Task 1 API shape.
- No root `php.ini` sample changes were made.
- No unrelated untracked files were modified or staged.

## Concerns

- None.
