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
- Add an opt-in real-artifact-backed Project-serving integration test for a tiny PHP Project.
- Keep always-run tests on small fake fixture artifacts for deterministic coverage and normal CI speed.

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
- Do not commit large PHP or FrankenPHP binary artifacts into this repository.
- Do not require 200MB+ runtime artifact downloads during default local test or branch CI runs.

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

## Artifact Test Strategy

PR 14 distinguishes real runtime artifacts from always-run tests.

Real artifacts are required for the browser-serving E2E path, but they live outside git. The E2E test consumes a normal PV artifact manifest URL and installs real PHP and FrankenPHP `.tar.gz` archives from that manifest. Candidate artifacts may live in a private Cloudflare R2 bucket or another maintainer-controlled artifact location. The test must exercise the real manifest, download, checksum, unpack, adapter validation, config generation, process supervision, and serving flow.

The real-artifact E2E is opt-in. It runs only when explicit environment variables are present. `PV_E2E_REAL_ARTIFACTS=1` enables the test, and `PV_E2E_ARTIFACT_MANIFEST_URL` provides the candidate artifact manifest URL.

The test should skip with a clear message when those variables are absent. Branch CI and ordinary local `cargo nextest` runs should not download large PHP/FrankenPHP artifacts by default. CI may run the opt-in E2E on manual dispatch, nightly, release-candidate validation, or a dedicated artifact-validation workflow. Artifact downloads should use PV's normal checksum-addressed cache so repeated opt-in runs do not redownload unchanged artifacts.

Always-run tests use small fake fixture artifacts. Fake artifacts should be limited to behavior that does not need real PHP execution, such as artifact layout validation, config rendering snapshots, config validation failure, readiness failure, and supervisor/reload failure handling. Fake artifacts must not be used as the only proof that a linked Project can be served.

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

PR 14 should use real artifacts for an opt-in Project-serving integration test. The test should use a manifest URL supplied by environment variable rather than public object storage defaults or committed archives. It should install or seed a real PHP and FrankenPHP track, link a tiny PHP Project, reconcile the system or Project, request the app through the Gateway path, and snapshot the observable response and relevant runtime state.

Fake artifacts are used for always-run deterministic tests:

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

Broader checks such as Clippy and full workspace tests should run before the implementation branch is considered complete. The opt-in real-artifact E2E should run before marking PR 14 complete when candidate artifacts are available. If candidate artifacts are not available yet, the implementation should leave the gated test in place and explicitly report that the real-artifact E2E remains pending.

## Open Decisions

None. PR 14 uses an opt-in real-artifact E2E for the Project-serving proof, keeps large runtime artifacts outside git, and uses fake artifacts only for always-run deterministic tests.
