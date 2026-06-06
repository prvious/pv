# PR 14 PHP/FrankenPHP Gateway Design

## Summary

PR 14 implements the first browser-serving runtime slice for PV. It adds PHP and FrankenPHP artifact adapter behavior, generated Gateway and PHP-track worker config, daemon-supervised Gateway and worker processes, and the first Project-serving integration test.

The work follows `DESIGN.md`: Gateway is an always-on PV-managed FrankenPHP/Caddy process that routes and terminates TLS, while Project-serving FrankenPHP workers are separate loopback processes grouped by PHP track. This PR proves that a linked PHP Project can be served through the managed Gateway path. PHP shims, public `pv php:*` behavior beyond existing stubs, and Composer remain PR 15 scope.

## Goals

- Validate PHP and FrankenPHP artifact layouts for installed runtime tracks.
- Generate Gateway root config and per-Project Gateway route config.
- Generate PHP-track worker root config and per-Project worker config.
- Start, adopt, reload, restart, and stop Gateway and PHP-track worker processes through the existing daemon supervisor.
- Persist and reuse Gateway and worker port assignments.
- Route linked Project hostnames to the worker for each Project's resolved PHP track.
- Preserve original host and forwarding headers while proxying through Gateway.
- Keep previous working Gateway/worker config and process state when new config validation or reload fails.
- Add a real-artifact-backed Project-serving integration test for a tiny PHP Project.
- Add small fake-artifact tests only for deterministic failure paths such as invalid layout, validation failure, and readiness failure.

## Non-Goals

- Do not implement the user-facing `php` shim.
- Do not implement `pv php:default`, `pv php:update`, `pv php:uninstall`, or `pv php:list`.
- Do not replace the existing deferred public `pv php:install` command surface.
- Do not implement Composer installation or the Composer shim.
- Do not add custom PHP ini settings, extension management, `phpize`, PECL, Xdebug, or extra PHP artifact flavors.
- Do not implement public artifact release recipes or object-storage publication.
- Do not add per-Project workers; one worker serves all Projects assigned to a PHP track.
- Do not add user-editable Caddy snippets or custom Gateway/worker config.
- Do not make the unknown-host friendly page a PR 14 acceptance criterion.

## Architecture

The implementation uses the existing crate boundaries:

- `resources` owns PHP and FrankenPHP adapter validation and runtime artifact identity.
- `daemon` owns routing reconciliation, config rendering orchestration, process supervision, readiness, reload/restart decisions, and observed failure recording.
- `state` owns Project records, managed resource track installation state, and persisted port assignments.
- `config` owns Project config parsing and document-root validation.
- `platform` remains responsible only for host integration already landed in prior PRs, such as `pf`, DNS, CA, and LaunchAgent behavior.

Gateway and worker config are generated outputs, not source of truth. The source of truth remains `pv.db`, Project config, cached artifact metadata, installed artifact state, local CA files, and current runtime observations.

## Runtime Model

Gateway is a core runtime. It is desired after setup even when no Project is linked. It listens on the persisted high loopback HTTP and HTTPS Gateway ports. macOS `pf` redirects loopback ports 80 and 443 to those high ports.

PHP-track workers are demand-driven. For each concrete PHP track with at least one linked Project, PV starts one FrankenPHP worker process on a persisted loopback high port. All Projects assigned to that PHP track are served by that worker. If no Projects remain on a track, reconciliation stops that worker but keeps the persisted worker port assignment. Explicit uninstall or prune behavior may release the assignment in a later command path.

Project PHP track resolution remains:

```text
Project config php field -> global default PHP track
```

PR 14 should use existing state shape where possible. If the global default PHP track is not yet fully implemented, tests may seed the resolved Project track through existing Project state and artifact track records rather than adding PR 15 command behavior early.

## Artifact Adapters

The PHP adapter validates standalone PHP artifacts. A valid PHP artifact contains an executable PHP binary and any runtime files required by the fixed-extension build. PR 14 does not add the PHP shim, but it must validate enough artifact layout to let future PR 15 command behavior trust installed PHP state.

The FrankenPHP adapter validates FrankenPHP artifacts. A valid FrankenPHP artifact contains the executable used for both Gateway and PHP-track worker processes. The adapter should expose stable helper methods for resolving the executable path from an installed release.

For a PHP track, setup/install desired state requires both `php` and `frankenphp` artifacts for the same concrete track. PR 14 can reconcile already-recorded desired track state and fixture manifests; it should not build the public `pv php:*` command UX.

## Generated Config

Generated config lives under `~/.pv/config/`:

- Gateway config under `~/.pv/config/gateway/`.
- PHP worker config under `~/.pv/config/workers/php-<track>/`.

Gateway root config imports per-Project route config. Worker root config imports per-Project worker config for Projects on that PHP track. Splitting config by Project keeps snapshots readable and limits the blast radius of config generation changes.

Gateway config should:

- listen on the assigned HTTP and HTTPS loopback ports;
- redirect HTTP Project requests to HTTPS by default;
- use PV's local CA for Project certificates;
- route primary and additional Project hostnames explicitly;
- preserve the original `Host` header;
- set forwarding headers including `X-Forwarded-Host`, `X-Forwarded-Proto`, and `X-Forwarded-For`.

Worker config should:

- listen on the assigned loopback worker port;
- serve only Projects assigned to that PHP track;
- use each Project's configured `document_root`, or the Project root when no document root is configured;
- support static files and front-controller routing for PHP applications;
- include Project hostname context in access logs where FrankenPHP/Caddy supports it cleanly.

Generated writes are atomic. PV renders new config to temporary files, validates the complete config with the managed FrankenPHP/Caddy binary, and promotes it only after validation passes.

## Reconciliation And Process Lifecycle

During system or Project reconciliation, the daemon builds a runtime plan from linked Projects, resolved PHP tracks, installed PHP/FrankenPHP track state, Gateway ports, worker ports, and local CA paths.

Gateway reconciliation:

1. Assign or reuse Gateway HTTP and HTTPS ports.
2. Render Gateway config.
3. Validate the new config with the installed FrankenPHP/Caddy binary.
4. Promote the config atomically after validation.
5. Reload the Gateway when an owned process is already running and reload is supported.
6. Restart the Gateway if reload fails or is unavailable.
7. Start Gateway if it is desired but not running.
8. Check readiness on the assigned Gateway ports.

Worker reconciliation:

1. Group linked Projects by concrete PHP track.
2. Assign or reuse one worker port per concrete PHP track.
3. Render that track's worker config.
4. Validate the new worker config with the installed FrankenPHP/Caddy binary.
5. Promote the config atomically after validation.
6. Reload the worker when an owned process is already running and reload is supported.
7. Restart the worker if reload fails or is unavailable.
8. Start the worker if it is desired but not running.
9. Check readiness on the assigned worker port.
10. Stop workers for PHP tracks no longer needed by any linked Project.

If config validation fails, PV keeps the previous active config and running process. If reload fails after validation, PV keeps or restores the previous active config where possible and reports the runtime as degraded or failed without tearing down unrelated runtimes.

Gateway failure is system-wide. A PHP-track worker failure affects only Projects assigned to that track. Unrelated PHP-track workers remain untouched.

## Logging And Runtime Metadata

Gateway logs are stored under `~/.pv/logs/gateway/` where practical, split into access and error logs when FrankenPHP/Caddy supports that cleanly. Worker logs are split by PHP track, such as `~/.pv/logs/workers/php-8.4.log`.

Process metadata and pid files use the existing supervisor model. Runtime metadata must include enough command, config path, resource name, track, and log path information for daemon restart adoption and ownership verification. PV never kills or adopts a process by PID alone.

## Testing

PR 14 should use real artifacts for the main Project-serving integration test. The test should use local fixture manifests and local fixture archives rather than public object storage. It should install or seed a real PHP and FrankenPHP track, link a tiny PHP Project, reconcile the system or Project, request the app through the Gateway path, and snapshot the observable response and relevant runtime state.

Fake artifacts are still useful for narrow deterministic failure tests:

- PHP artifact missing the expected executable is rejected.
- FrankenPHP artifact missing the expected executable is rejected.
- Config validation failure preserves previous active config.
- Readiness failure records a scoped runtime failure.

Snapshot tests should cover generated Gateway config and PHP-track worker config. Nearby integration and daemon tests should be copied for style, including `insta` snapshots instead of substring assertions where the output shape is meaningful.

Focused verification should prefer:

```shell
cargo nextest run -E 'test(<specific_test_name>)'
cargo insta test --accept --test-runner nextest -- <specific_test_name>
cargo fmt --all -- --check
git diff --check
```

Broader checks such as Clippy and full workspace tests should run before the implementation branch is considered complete.

## Open Decisions

None. The main PR 14 Project-serving test uses real PHP/FrankenPHP artifacts; fake artifacts are limited to failure-path tests.
