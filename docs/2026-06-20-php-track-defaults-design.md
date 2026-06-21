# PHP Track Defaults Design

## Summary

PV needs PHP track defaults that are stable across Managed Resource artifact revisions and apply consistently to standalone PHP, Composer, and Project-serving FrankenPHP workers.

The current published PHP/FrankenPHP artifacts fall back to compiled ini paths under `/usr/local/etc/php`. That is risky because a user machine may already have files there, and PV should not accidentally load host PHP configuration. A runtime probe against a linked Project also showed that current FrankenPHP workers inherit PV's XDG environment but do not receive `PHPRC` or `PHP_INI_SCAN_DIR`, so browser execution currently reports no loaded `php.ini` and falls back to `/usr/local/etc/php`.

The approved direction is intentionally narrow: PV seeds default PHP configuration once per PHP track, keeps that configuration outside artifact release directories, and points CLI PHP, Composer, and Project-serving FrankenPHP workers at the same track-level defaults through process-level `PHPRC` and `PHP_INI_SCAN_DIR`.

## Goals

- Add PV-owned PHP track defaults for each installed PHP track.
- Store track defaults outside immutable artifact release directories so artifact updates and old-release pruning do not remove them.
- Seed `php.ini` and `conf.d/` only when missing.
- Do not overwrite an existing track default file during install or update.
- Use the same default PHP profile for supported PHP tracks `8.3`, `8.4`, and `8.5`.
- Keep standalone PHP, Composer, and FrankenPHP worker execution on the same track defaults through process-level `PHPRC` and `PHP_INI_SCAN_DIR`.
- Change PHP/FrankenPHP artifact build fallback ini paths away from `/usr/local/etc/php`.
- Add tests that prove CLI and browser execution no longer fall back to `/usr/local/etc/php`.

## Non-Goals

- Do not add Project-specific PHP ini support.
- Do not add a public command for editing PHP defaults.
- Do not add extension management, dynamic extension loading, `phpize`, PECL, Xdebug, or extra PHP artifact flavors.
- Do not render the seeded defaults into Caddyfile `php_ini` directives.
- Do not pass PHP ini discovery paths through Caddyfile `env` directives.
- Do not overwrite existing seeded files to apply new defaults.
- Do not create or depend on files under the compiled fallback ini path.

## Filesystem Layout

Track defaults live under the mutable PHP track directory:

```text
~/.pv/resources/php/<track>/
  current -> releases/<artifact-version>
  releases/
  etc/
    php.ini
    conf.d/
```

`etc/` belongs to the PHP track, not to a specific artifact version. It is preserved across `php:<track>` artifact updates because update cleanup only prunes old directories under `releases/`.

Default `pv uninstall` preserves `resources/`, so track defaults are preserved with other Managed Resource data. `pv uninstall --prune` removes all PV state, including track defaults.

## Seed Behavior

PV creates the track default paths when installing or updating a PHP track:

- create `~/.pv/resources/php/<track>/etc/`
- create `~/.pv/resources/php/<track>/etc/conf.d/`
- create `~/.pv/resources/php/<track>/etc/php.ini` only if it is missing

If `php.ini` already exists, PV leaves it unchanged. If `conf.d/` already exists, PV leaves its contents unchanged.

This is seed-only behavior. It gives PV a stable default file without introducing a managed merge or migration system for future default changes.

If an existing `php.ini` is a regular readable file, PV uses it. If an existing `php.ini` is a directory, symlink, unreadable file, or other non-regular file, PV fails clearly rather than overwriting it. If `conf.d/` already exists and is a directory, PV leaves its contents unchanged. If `conf.d/` exists but is not a directory, PV fails clearly.

## Default Source

The seeded `php.ini` content should come from PV's approved sample `php.ini`.

PV should generate the seeded file by:

- removing comments
- preserving section headers, such as `[PHP]` and `[Date]`
- preserving active assignments in their original order, including intentionally empty values

The same generated default profile applies to PHP tracks `8.3`, `8.4`, and `8.5`. PV seeds it only when `php.ini` is missing. User edits to the seeded file are preserved and become effective for CLI PHP, Composer, and FrankenPHP workers because all three execution paths use `PHPRC` and `PHP_INI_SCAN_DIR`.

PV should not parse an already-seeded, user-edited `php.ini` to produce any other runtime configuration. The file itself is the runtime configuration surface.

## PHP Track Defaults Component

PV should keep PHP default path and environment logic in one small shared component rather than scattering it through CLI and daemon code.

The component should:

- compute `resources/php/<track>/etc`
- compute `resources/php/<track>/etc/conf.d`
- seed `php.ini` and `conf.d/` when missing
- expose the process environment overlay for a resolved PHP track
- validate that existing default paths are usable

## CLI PHP And Composer

Standalone PHP and Composer continue to use process environment because the Caddyfile is not involved in CLI execution.

For an installed PHP track, the `php` shim sets:

```text
PHPRC=~/.pv/resources/php/<track>/etc
PHP_INI_SCAN_DIR=~/.pv/resources/php/<track>/etc/conf.d
```

The Composer shim invokes Composer through PV's PHP shim, so it inherits the same PHP track defaults.

The shim should not point to `resources/php/<track>/releases/<artifact-version>/etc` because release directories are immutable artifact payloads and may be pruned after updates.

## FrankenPHP Workers

Project-serving FrankenPHP workers should receive PHP defaults through the worker process environment:

```text
PHPRC=~/.pv/resources/php/<track>/etc
PHP_INI_SCAN_DIR=~/.pv/resources/php/<track>/etc/conf.d
```

Each worker process serves one PHP track, so one track-level process environment overlay per worker process matches the runtime model.

PV should pass the same environment overlay to FrankenPHP config validation that it will pass to the worker process. Validation must validate the same generated Caddyfile under the same PHP ini discovery environment that will be used to start or reload the worker.

The Gateway process does not serve Project PHP directly. It should keep only the environment/config it needs for routing, TLS, and storage unless a future feature makes PHP execution in the Gateway process intentional.

PV should not use Caddyfile `env` to set `PHPRC` or `PHP_INI_SCAN_DIR`. FrankenPHP's Caddyfile `env` directives are request/worker environment values made available to PHP as CGI-like variables after the embedded PHP runtime has already started. PHP ini file discovery must be configured through the OS process environment before FrankenPHP initializes PHP.

## Artifact Build Fallback

The PHP/FrankenPHP artifact recipe should stop compiling PHP with `/usr/local/etc/php` as the fallback ini location.

Use a deterministic path that PV will not create and users are extremely unlikely to populate:

```text
/var/empty/com.prvious.pv/php
/var/empty/com.prvious.pv/php/conf.d
```

These paths are only defensive fallbacks. Normal PV execution must provide explicit process environment configuration. Tests should fail if `phpinfo()` or `php --ini` reports `/usr/local/etc/php` for PV-managed artifacts after the fix.

## Data Flow

PHP track install or update:

1. Resolve and install the selected `php:<track>` artifact.
2. Ensure `resources/php/<track>/etc/` and `resources/php/<track>/etc/conf.d/` exist.
3. Seed `resources/php/<track>/etc/php.ini` if missing.
4. Install or update the paired `frankenphp:<track>` artifact.
5. Record both installed tracks in `pv.db`.
6. Reconcile affected Project-serving workers.

CLI `php` execution:

1. Resolve the concrete PHP track from Project state or global default.
2. Verify the installed PHP artifact.
3. Ensure track defaults exist.
4. Execute the selected PHP binary with `PHPRC` and `PHP_INI_SCAN_DIR` pointing at the track defaults.

FrankenPHP worker reconciliation:

1. Group Projects by PHP track.
2. Ensure track defaults exist for each demanded PHP track.
3. Render the worker root Caddyfile without expanding PHP defaults into `php_ini` directives.
4. Validate the Caddyfile with the managed FrankenPHP binary and the track-level `PHPRC` / `PHP_INI_SCAN_DIR` process environment.
5. Start, reload, or restart the worker with the same track-level process environment.
6. Promote runtime state through the existing Gateway/worker reconciliation flow.

## Error Handling

Failure to create the track defaults is an install/reconciliation failure for that PHP track. PV should report the failing path and preserve the previous working artifact/runtime state where existing rollback behavior allows it.

If the track default file already exists but is unreadable or is not a regular file, PV should fail clearly rather than overwrite it. If the `conf.d` path exists but is not a directory, PV should fail clearly rather than replace it.

If the generated FrankenPHP config fails validation under the track-level PHP ini environment, PV should keep the previous active worker config/process and record a worker-scoped runtime error.

## Testing

Prefer integration tests and snapshots following nearby PHP, Composer, and Gateway tests.

Always-run tests should cover:

- PHP pair install seeds `resources/php/<track>/etc/php.ini` and `etc/conf.d/`.
- PHP pair update preserves an existing `php.ini`.
- Seeded `php.ini` removes comments while preserving section headers and active assignments from the approved sample.
- The PHP shim passes `PHPRC` and `PHP_INI_SCAN_DIR` pointing at the track-level `etc` paths.
- Composer shim execution inherits the PHP shim's track-level ini environment.
- Worker process specs include track-level `PHPRC` and `PHP_INI_SCAN_DIR`.
- Worker config validation receives the same track-level `PHPRC` and `PHP_INI_SCAN_DIR` as the worker process.
- Worker config snapshots do not include a generated `frankenphp { php_ini ... }` defaults block.
- Gateway process specs and config snapshots do not add PHP track defaults to the pure routing Gateway unless explicitly needed.

Artifact recipe and smoke tests should cover:

- PHP build metadata no longer contains `/usr/local/etc/php` fallback paths.
- `php --ini` under the PV shim reports the track-level `php.ini` path.
- A real FrankenPHP worker serving `phpinfo()` reports the track-level loaded config file and does not report `/usr/local/etc/php`.
- The real-artifact browser smoke remains opt-in and does not run during ordinary local or branch CI unless explicitly enabled.

Focused verification should prefer:

```shell
cargo nextest run -E 'test(<specific_test_name>)'
cargo insta test --accept --test-runner nextest -- <specific_test_name>
cargo fmt --all -- --check
git diff --check
```

Before publication of revised PHP/FrankenPHP artifacts, run the native artifact workflow smoke for every supported PHP track and platform.

## Documentation Impact

`DESIGN.md` should be updated to describe PHP track defaults as PV-owned track-level configuration seeded under `~/.pv/resources/php/<track>/etc`.

The artifact recipe documentation should describe the safe compiled fallback ini path and state that PV-managed runtime execution must not depend on it.
