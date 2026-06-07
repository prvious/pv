# PR 15 PHP Shim, PHP Commands, And Composer Design

## Summary

PR 15 completes the developer-facing PHP and Composer command path after PR 14's Gateway and PHP-track worker runtime work. It adds PV-owned `php` and `composer` shims, replaces the previously documented `php:default` command with `php:use --global`, implements the public `pv php:*` command family, and implements Composer track `2` install/update/uninstall behavior.

The design keeps ordinary `php ...` and `composer ...` execution predictable. Shims resolve the intended installed track and run it. They do not download missing runtimes. Explicit PV management commands such as `pv php:use`, `pv php:install`, and `pv composer:install` are responsible for installing missing artifacts.

## Goals

- Add a Project-aware `php` shim under `~/.pv/bin/php`.
- Add a `composer` shim under `~/.pv/bin/composer`.
- Persist the global PHP default track in `pv.db`.
- Replace `pv php:default <track>` with `pv php:use <track> --global` / `-g`.
- Implement Project-level `pv php:use <track>` by updating the discovered Project config file.
- Treat standalone PHP and matched FrankenPHP tracks as a pair for install, update, uninstall, and use operations.
- Implement `pv php:install`, `pv php:update`, `pv php:uninstall`, and `pv php:list`.
- Implement `pv composer:install`, `pv composer:update`, and `pv composer:uninstall` for Composer track `2`.
- Keep default tests on small fixture artifacts and avoid required real PHP/Composer downloads during ordinary CI.

## Non-Goals

- Do not auto-download PHP or Composer from the `php` or `composer` shims.
- Do not add versioned shims such as `php8.4`.
- Do not infer PHP versions from `composer.json`.
- Do not run Project package-manager or Laravel commands automatically.
- Do not add Composer 1 or Composer 3 support.
- Do not add public artifact recipes or object-storage publication; those remain PR 24 and later release work.
- Do not redesign Gateway or PHP worker supervision from PR 14.
- Do not add new macOS system-integration behavior.

## Command Surface

PR 15 intentionally changes the documented PHP default command from `php:default` to `php:use --global`. The implementation PR must update `DESIGN.md` command tables and surrounding prose to match this contract.

```shell
pv php:use <track>
pv php:use <track> --global
pv php:use <track> -g
pv php:install [track]
pv php:update
pv php:uninstall <track> [--prune] [--force]
pv php:list

pv composer:install
pv composer:update
pv composer:uninstall [--prune] [--force]
```

`pv php:use <track>` is Project-scoped by default. It resolves the current linked Project, updates the Project config `php:` field, ensures both standalone PHP and matched FrankenPHP artifacts for the resolved track are installed, and requests Project reconciliation so the Gateway and worker runtime use the new track.

`pv php:use <track> --global` stores the global default PHP track in `pv.db`, ensures both standalone PHP and matched FrankenPHP artifacts for the resolved track are installed, and requests reconciliation for Projects that inherit the global default.

`pv php:install [track]` installs the standalone PHP and matched FrankenPHP pair. If the track argument is omitted, it resolves the manifest default PHP track. If the track is `latest`, it resolves to the manifest default concrete PHP track before writing state.

`pv php:update` updates every installed PHP track and the matched FrankenPHP tracks. It is not limited to the global default.

`pv php:uninstall <track> [--prune] [--force]` removes the PHP/FrankenPHP pair. By default it refuses to uninstall a track that is used by linked Projects or is the global default. `--force` bypasses those guards and records removal intent anyway. `--prune` controls PV-owned runtime artifact/data cleanup where applicable and must not touch Project files.

`pv php:list` lists installed PHP tracks, marks the global default, and may show Project usage counts for each track.

Composer v1 behavior is intentionally simpler. `pv composer:install` installs Composer track `2` and ensures the resolved global/default PHP/FrankenPHP pair is installed because the Composer shim runs through the PHP shim. `pv composer:update` updates Composer track `2` to the latest non-revoked artifact. `pv composer:uninstall [--prune] [--force]` removes the Composer artifact. Without `--prune`, it preserves `~/.pv/composer` home/cache. With `--prune`, it removes PV-owned Composer home/cache.

## Architecture

PR 15 should preserve the existing crate boundaries:

- `cli` owns the user-facing command routing, `php` shim entrypoint, and `composer` shim entrypoint.
- `resources` owns artifact adapters and shared install/update/uninstall helpers.
- `state` owns persisted global PHP default, installed track state, usage counts, and removal intent.
- `config` owns Project config discovery and structured Project config updates.
- `daemon` owns reconciliation after Project config, global default, and installed-track changes.
- `platform` is not part of this feature except through existing host/process helpers.

The global PHP default should be stored in `pv.db` as a narrow setting or preference. Stored values must be concrete tracks, never `latest`. If the setting is absent, PV falls back to the current manifest default PHP track. The fallback is resolved when needed, but commands that write state should store the concrete track.

Project-level `php:use` should reuse `ProjectConfigFile::read_from_root`. If `pv.yml` exists, update it. If only `pv.yaml` exists, update it. If neither exists, create `pv.yml`. If both exist, fail with the existing config-conflict behavior. The update must preserve unrelated semantic config fields such as `hostnames`, `document_root`, `env`, and resource blocks. Comment and formatting preservation is desirable only if the YAML tooling makes it practical; semantic preservation is required.

The `php` shim should resolve the current Project by walking from the process working directory through linked Project roots. Inside a linked Project, it uses that Project's resolved concrete PHP track. Outside a linked Project, it uses the global default. It then runs the installed standalone PHP executable for that track.

The `composer` shim resolves the installed Composer track `2` PHAR and invokes it through PV's `php` shim. This gives Composer the same Project-aware PHP selection as direct `php` commands.

## Artifact Pairing

PHP track operations treat `php:<track>` and `frankenphp:<track>` as one logical pair.

Install and use operations install both artifacts when either side is missing. Update operations update both artifacts for every installed PHP track. Uninstall operations record removal intent for both artifacts.

This avoids a broken state where CLI PHP exists but Project serving cannot start, or Gateway/workers can serve a track that the `php` shim cannot run.

## Error Handling

The shims must not auto-install missing artifacts.

If the `php` shim resolves a track that is not installed, it exits non-zero with a clear repair command such as:

```text
PHP track 8.4 is not installed.
Run `pv php:install 8.4` to install it.
```

If the `composer` shim cannot find Composer track `2`, it exits non-zero with a repair command such as `pv composer:install`. If Composer is installed but the required PHP track is missing, the error should point to the PHP repair command.

If Project config parsing fails, `php:use` fails before installing artifacts or requesting reconciliation. `php:use` installs the PHP/FrankenPHP pair before mutating Project config or global selection state. If artifact installation fails, the selection remains unchanged and PV reports the installation failure.

If `php:update` updates some tracks and one pair fails, PV reports the partial failure, keeps the last valid installed artifacts for the failed pair, and exits non-zero.

Uninstall guards should consider both explicit Project config tracks and Projects inheriting the global default. `--force` can bypass those checks, but daemon reconciliation remains responsible for safe process stops and cleanup.

## Data Flow

Project-level `pv php:use <track>`:

1. Resolve the current linked Project from the current working directory.
2. Resolve `track` to a concrete PHP track using the artifact manifest.
3. Read and validate the Project config through `ProjectConfigFile::read_from_root`.
4. Write or update the `php:` field in the discovered config file.
5. Install `php:<track>` and `frankenphp:<track>` if needed.
6. Request Project reconciliation.
7. Report the selected track, config path, installed artifacts, and reconciliation result.

Global `pv php:use <track> --global`:

1. Resolve `track` to a concrete PHP track.
2. Store the concrete global default in `pv.db`.
3. Install `php:<track>` and `frankenphp:<track>` if needed.
4. Request reconciliation for Projects that inherit the global default.
5. Report the selected track, installed artifacts, and reconciliation result.

`php` shim execution:

1. Resolve the current Project from the working directory, if any.
2. Resolve the concrete PHP track from Project config/state or global default.
3. Verify the installed PHP artifact and executable path.
4. Replace the shim process with the selected PHP executable, preserving user arguments and relevant environment.
5. Fail clearly without downloading when resolution or installation checks fail.

Composer command execution:

1. Resolve the installed Composer track `2` PHAR.
2. Invoke the PHAR through PV's `php` shim with the original Composer arguments.
3. Let the PHP shim choose the Project or global PHP track.
4. Fail clearly without downloading when Composer or the selected PHP track is missing.

## Testing

Prefer integration tests and snapshots following nearby test style.

Always-run tests should use fake fixture artifacts:

- CLI snapshots for `php:use`, `php:install`, `php:update`, `php:uninstall`, `php:list`, `composer:install`, `composer:update`, and `composer:uninstall`.
- Project config tests proving `php:use` updates existing `pv.yml`, updates existing `pv.yaml`, creates `pv.yml` when neither exists, and fails on the existing dual-file conflict.
- State tests proving global PHP default persistence, fallback behavior, and rejection of `latest` as stored state.
- Resource command tests proving paired PHP/FrankenPHP install/update/uninstall behavior.
- Shim tests proving Project-track resolution, global-default resolution, argument forwarding, and clear missing-track failures.
- Composer shim tests proving Composer track `2` runs through the PHP shim and fails clearly when Composer or PHP is missing.

Real artifacts are optional for PR 15 default verification. If PR 14's opt-in real-artifact test path is available, PR 15 may add or extend a gated smoke test for `php -v` and `composer --version`, enabled only through explicit environment variables. Ordinary local and branch CI runs must not download large PHP, FrankenPHP, or Composer artifacts by default.

Focused implementation verification should prefer:

```shell
cargo nextest run -E 'test(<specific_test_name>)'
cargo insta test --accept --test-runner nextest -- <specific_test_name>
cargo fmt --all -- --check
git diff --check
```

Before the implementation PR is considered complete, run:

```shell
cargo nextest run -p pv -p cli -p resources -p state -p config -p daemon --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## Documentation Impact

The implementation PR must update `DESIGN.md` to replace `pv php:default <track>` with `pv php:use <track> --global`, add Project-level `pv php:use <track>`, and keep the command table consistent with the PR 15 command surface.

The implementation PR should also update any affected help snapshots and `IMPLEMENTATION.md` progress only after the actual PR is merged.

## Open Decisions

None. The approved direction is explicit PV management commands install required artifacts, while direct `php` and `composer` shim execution fails clearly when required artifacts are missing.
