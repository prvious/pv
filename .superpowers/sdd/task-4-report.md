# Task 4 Report: Use Track Defaults For FrankenPHP Worker Validation And Runtime

## What I implemented

- Added fallible FrankenPHP worker private environment wiring that combines the existing XDG environment with PHP track defaults from `resources::php_track_environment`.
- Kept Gateway process private environment PHP-neutral; it still only receives XDG config/data paths.
- Changed worker process spec creation to return `Result<ProcessSpec, DaemonError>` so invalid PHP track/default environment failures are propagated instead of guessed around.
- Ensured PHP track defaults with `resources::ensure_php_track_defaults` before worker config validation and recorded PHP worker runtime errors if default seeding/env construction fails.
- Passed an explicit private environment into config promotion validation so Gateway validation receives Gateway XDG env and worker validation receives worker env with `PHPRC` and `PHP_INI_SCAN_DIR`.
- Added worker Caddyfile coverage to guard against generated `php_ini` defaults appearing in rendered worker configs.
- Updated the process spec snapshot so only the PHP worker shows redacted `PHPRC` and `PHP_INI_SCAN_DIR`.

## What I tested and results

- `cargo fmt --all --check` - PASS
- `cargo insta test --accept --test-runner nextest -p daemon -- frankenphp_command_and_process_specs_are_stable` - PASS, accepted updated process spec snapshot
- `cargo insta test --accept --test-runner nextest -p daemon -- worker_config_renderer_outputs_track_caddyfile` - PASS, no snapshot changes
- `cargo nextest run -p daemon -E 'test(frankenphp_config_validation_receives_xdg_environment)'` - PASS
- `cargo nextest run -p daemon -E 'test(frankenphp_command_and_process_specs_are_stable) | test(frankenphp_config_validation_receives_xdg_environment)'` - PASS
- `cargo nextest run -p daemon -E 'test(worker_config_renderer_outputs_track_caddyfile) | test(gateway_reconciliation_starts_gateway_and_one_worker_per_php_track)'` - PASS

## TDD Evidence

### RED

Command:

```shell
cargo nextest run -p daemon -E 'test(frankenphp_command_and_process_specs_are_stable) | test(frankenphp_config_validation_receives_xdg_environment)'
```

Output summary:

```text
FAIL daemon::gateway_reconciliation frankenphp_command_and_process_specs_are_stable
assertion `left == right` failed
  left: None
 right: Some("/var/folders/.../home/.pv/resources/php/8.4/etc")
Summary: 1/2 tests run: 0 passed, 1 failed, 213 skipped
warning: 1/2 tests were not run due to test failure
```

### GREEN

Command:

```shell
cargo nextest run -p daemon -E 'test(frankenphp_command_and_process_specs_are_stable) | test(frankenphp_config_validation_receives_xdg_environment)'
```

Output summary:

```text
PASS daemon::gateway_reconciliation frankenphp_command_and_process_specs_are_stable
PASS daemon::gateway_reconciliation frankenphp_config_validation_receives_xdg_environment
Summary: 2 tests run: 2 passed, 213 skipped
```

Additional focused command:

```shell
cargo nextest run -p daemon -E 'test(worker_config_renderer_outputs_track_caddyfile) | test(gateway_reconciliation_starts_gateway_and_one_worker_per_php_track)'
```

Output summary:

```text
PASS daemon::gateway_config worker_config_renderer_outputs_track_caddyfile
PASS daemon::gateway_reconciliation gateway_reconciliation_starts_gateway_and_one_worker_per_php_track
Summary: 2 tests run: 2 passed, 213 skipped
```

## Files changed

- `crates/daemon/src/gateway.rs`
- `crates/daemon/tests/gateway_config.rs`
- `crates/daemon/tests/gateway_reconciliation.rs`
- `crates/daemon/tests/real_artifact_gateway_e2e.rs`
- `crates/daemon/tests/snapshots/gateway_reconciliation__frankenphp_command_and_process_specs_are_stable.snap`
- `.superpowers/sdd/task-4-report.md`

## Self-review findings

- Gateway runtime remains PHP-neutral: no `PHPRC` or `PHP_INI_SCAN_DIR` are added to `gateway_process_spec`.
- Worker runtime uses process-level PHP ini discovery paths through private environment, not Caddyfile `php_ini` directives.
- Worker config validation uses the same environment helper as worker process startup.
- Defaults are seeded before worker validation; failures are recorded against the PHP worker runtime subject before returning.
- No `panic!`, `unreachable!`, `.unwrap()`, `.expect()`, unsafe code, or clippy rule ignores were added.
- `crates/daemon/tests/real_artifact_gateway_e2e.rs` needed a mechanical call-site update because `worker_process_spec` is now fallible.
- Existing unrelated untracked files `docs/superpowers/plans/2026-06-21-php-track-defaults.md` and `php.ini` were left untouched and will not be staged.

## Concerns

None for the implemented task.
