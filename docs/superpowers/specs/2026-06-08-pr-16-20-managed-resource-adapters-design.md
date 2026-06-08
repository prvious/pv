# PR 16-20 Managed Resource Adapters Design

## Context

PRs 16 through 20 implement the first backing Managed Resource adapters from `IMPLEMENTATION.md`: Mailpit, Redis, MySQL, Postgres, and RustFS.

The roadmap says these adapters are parallel work after the adapter foundation and Project Resource allocation contracts. It also says adapter PRs should not depend on unpublished object-storage artifacts, and MySQL/Postgres must not begin by copying unfinished code from each other in parallel. This design turns that into a seven-workstream adapter wave with two small shared foundation PRs before the full adapter slices.

The source-of-truth product constraints remain `DESIGN.md`:

- Managed Resources are external artifacts installed and supervised by PV.
- Backing Managed Resources are installed by setup/install commands but do not start until linked Project config demands them.
- Project config demand can install a missing resource track automatically during reconciliation.
- Resource allocation names are stable and stored in `pv.db`.
- Generated local credentials are stored plainly in user-owned PV state for v1.
- Public artifact recipes and object-storage publication are later release work, not adapter PR prerequisites.

## Goals

- Add a shared daemon-local Managed Resource runtime foundation.
- Add a shared SQL foundation before MySQL and Postgres.
- Implement Mailpit, Redis, MySQL, Postgres, and RustFS as full vertical adapter slices.
- Keep each real adapter independently reviewable and testable with fixture artifacts.
- Preserve existing crate boundaries unless a concrete adapter proves a narrow change is required.
- Convert the final approved implementation plan into Solo todos with blocker relationships and worktree ownership metadata.

## Non-Goals

- Do not publish real Managed Resource artifacts.
- Do not add public artifact recipes in these PRs.
- Do not require upstream Mailpit, Redis, MySQL, Postgres, or RustFS binaries for normal PR verification.
- Do not change public Project config syntax.
- Do not casually expand the existing `resources::ResourceAdapter` install trait into a broad runtime trait.
- Do not add rich status, logs, doctor, jobs, or JSON status UX beyond minimal runtime observed state. That remains PR 21 scope.
- Do not run Project application behavior such as migrations, `APP_KEY`, Composer install, or Laravel commands.

## Workstreams

The adapter wave is seven workstreams:

| Workstream | Purpose | Blocked By |
| ---------- | ------- | ---------- |
| Runtime foundation | Generic daemon-local runtime orchestration for backing Managed Resources | None |
| SQL foundation | Shared MySQL/Postgres admin and allocation helpers | Runtime foundation |
| Mailpit adapter | Full Mailpit adapter slice | Runtime foundation |
| Redis adapter | Full Redis adapter slice | Runtime foundation |
| RustFS adapter | Full RustFS adapter slice | Runtime foundation |
| MySQL adapter | Full MySQL adapter slice | Runtime foundation, SQL foundation |
| Postgres adapter | Full Postgres adapter slice | Runtime foundation, SQL foundation |

This gives one root foundation branch, one SQL branch after it, and five adapter branches with explicit blockers.

## Architecture

Runtime orchestration stays in `daemon`.

The `resources` crate continues to own resource identity, registry descriptors, Artifact manifest parsing, download/cache behavior, atomic installs, fixture artifacts, and install validation adapters. It should not gain a broad runtime adapter trait during the foundation PR.

The `daemon` crate owns backing Managed Resource runtime behavior:

- Project-demand detection.
- Demand-driven artifact install before runtime start.
- Named multi-port assignment.
- `ProcessSpec` construction.
- Start, adopt, stop, and ownership verification.
- Readiness checks.
- Log paths and runtime metadata.
- Minimal observed runtime state.
- Resource env context recording hooks.
- Allocation reconciliation hooks for adapter-specific admin behavior.

Adapter PRs add daemon-local resource runtime definitions/builders that consume installed artifact paths and existing registry descriptors.

## Runtime Foundation

The shared runtime foundation replaces non-Gateway resource reconciliation stubs with daemon-local Managed Resource runtime plumbing.

The foundation must support named multi-port resources. Redis, MySQL, and Postgres each need a data port. Mailpit needs SMTP and HTTP UI ports. RustFS needs an S3/API port and, if the packaged runtime exposes it, a console UI port.

Runtime start is driven by Project demand only. `pv setup` and resource `*:install` commands install artifacts or record desired installed state, but they do not start backing Managed Resource processes. When no linked Projects need a Managed Resource track, reconciliation stops that runtime while preserving installed artifacts and allocation data.

When Project reconciliation sees a demanded resource track that is not installed, the daemon installs it before starting the runtime. The sequence is:

1. Resolve Project resource track.
2. Install the selected artifact if missing.
3. Assign named ports.
4. Start or adopt the runtime.
5. Verify readiness.
6. Record resource env context.
7. Reconcile adapter-specific allocations.
8. Render Project `.env` only after all required context is ready.

The foundation records minimal observed runtime state: running, ready, failed, or stopped, with a concise message and useful log/runtime metadata references. Rich status formatting, doctor checks, log browsing UX, and JSON status output remain PR 21 scope.

The foundation also adds a test-only fake multi-port Managed Resource runtime. The fake runtime must not appear in normal registry, Project config, CLI help, shell completions, public manifests, or user-facing examples.

## SQL Foundation

The SQL foundation is a small shared PR before MySQL and Postgres.

It may add focused `sqlx` usage for MySQL/Postgres readiness and admin operations. SQL admin queries should be runtime/dynamic queries. PV v1 must not require `sqlx` offline query metadata.

The SQL foundation should cover common behavior only:

- SQL connection/admin scaffolding.
- Shared database allocation helper behavior.
- Stable database name handling using existing generated allocation names.
- Common resource env context expectations.
- Common database create/check behavior.

It must not implement a full MySQL or Postgres adapter.

## Adapter Slices

Each real adapter PR is a full vertical slice after its blockers land.

Mailpit:

- Artifact layout validation.
- Demand-driven install/start from Project config.
- SMTP and UI/dashboard runtime ports.
- HTTP readiness for the dashboard where feasible.
- Resource env context for SMTP host/port and dashboard URL.
- Public `mailpit:*` and `mail:*` command namespace.
- Read-only `mailpit:open` / `mail:open`.
- Install, update, uninstall, list, help, completion, and snapshot coverage.
- No Resource allocations.

Redis:

- Artifact layout validation.
- Demand-driven install/start from Project config.
- Redis runtime port.
- Redis client PING/readiness behavior instead of shelling out to `redis-cli`.
- Prefix allocation creation/reuse using existing allocation state.
- Resource and allocation env contexts.
- Public `redis:*` command namespace.
- Project env and CLI snapshot coverage.

MySQL:

- Artifact layout validation.
- Demand-driven install/start from Project config.
- MySQL runtime port.
- `sqlx` readiness/admin behavior instead of shelling out to `mysql`.
- Database allocation creation/reuse using existing allocation state.
- Resource and allocation env contexts.
- Public `mysql:*` command namespace.
- Project env and CLI snapshot coverage.

Postgres:

- Artifact layout validation.
- Demand-driven install/start from Project config.
- Postgres runtime port.
- `sqlx` readiness/admin behavior instead of shelling out to `psql`.
- Database allocation creation/reuse using existing allocation state.
- Resource and allocation env contexts.
- Public `postgres:*` and `pg:*` command namespace.
- Project env and CLI snapshot coverage.

RustFS:

- Artifact layout validation.
- Demand-driven install/start from Project config.
- S3/API runtime port and console port when available.
- S3-compatible bucket creation/check behavior.
- Prefer `object_store` for bucket operations first.
- Fall back to AWS SDK for Rust only if `object_store` cannot create/check buckets cleanly against RustFS.
- Do not manage buckets through `mc`.
- Bucket allocation creation/reuse using existing allocation state.
- Resource and allocation env contexts.
- Public `rustfs:*` and `s3:*` command namespace.
- Read-only `rustfs:open` / `s3:open`.
- Project env and CLI snapshot coverage.

## State

Stable resource-level credentials and endpoint values use existing `managed_resource_tracks.env_json` in v1.

Examples include SQL root/superuser credentials, RustFS access and secret keys, Mailpit SMTP/UI values, and Redis host/port/url values. New resource-specific state tables should not be added unless a concrete adapter proves it needs structured state beyond resource env context and existing allocation records.

Allocation-specific state uses existing `resource_allocations.generated_name` plus `resource_allocations.env_json`.

SQL database names, Redis prefixes, and RustFS bucket names use the existing stable generated allocation name state. Adapter PRs mark allocations ready with allocation env values through the existing state API.

## Command Surface

Each adapter PR adds its own public command namespace. There is no separate shared command-surface PR.

```shell
pv {mailpit|mail}:install [version]
pv {mailpit|mail}:update
pv {mailpit|mail}:uninstall <version> [--prune] [--force]
pv {mailpit|mail}:list
pv {mailpit|mail}:open

pv redis:install [version]
pv redis:update
pv redis:uninstall <version> [--prune] [--force]
pv redis:list

pv mysql:install [version]
pv mysql:update
pv mysql:uninstall <version> [--prune] [--force]
pv mysql:list

pv {postgres|pg}:install [version]
pv {postgres|pg}:update
pv {postgres|pg}:uninstall <version> [--prune] [--force]
pv {postgres|pg}:list

pv {rustfs|s3}:install [version]
pv {rustfs|s3}:update
pv {rustfs|s3}:uninstall <version> [--prune] [--force]
pv {rustfs|s3}:list
pv {rustfs|s3}:open
```

`mailpit:open` / `mail:open` and `rustfs:open` / `s3:open` are read-only. They must not install artifacts, start runtimes, or record desired state. They open the dashboard or console only when the runtime is already running and has a known URL. If not running, they report that plainly and point to Project config demand and install/setup as appropriate.

## Error Handling

Failures are scoped to the affected Managed Resource track and Project.

If install, start, readiness, admin allocation, or env context generation fails, reconciliation records minimal observed failure state and preserves the previous valid Project `.env` block. Other Projects and unrelated resources should continue reconciling where possible.

Project `.env` rendering is all-or-nothing for a Project. PV must not write partial generated resource values.

Runtime adoption must verify PV ownership before reusing or stopping a process. PV never kills a process based on PID alone.

Destructive cleanup remains behind uninstall/prune intent.

## Testing

Prefer integration-style tests and `insta` snapshots following nearby test style.

The runtime foundation should prove application behavior with the test-only fake multi-port runtime:

- Project config demand installs and starts a fake Managed Resource runtime.
- Named ports are assigned and rendered into resource env context.
- Readiness success and failure paths are observable.
- Logs, runtime metadata, and minimal observed state are recorded.
- Runtime adoption is covered where feasible.
- Removing Project demand stops the fake runtime while preserving installed artifacts/state.

Adapter PRs use lightweight fixture artifacts with controllable test binaries/scripts. Normal PR verification must not require real upstream binaries or published object-storage artifacts.

Each adapter PR should include:

- Artifact layout validation tests.
- Demand-driven Project reconciliation tests.
- Readiness tests.
- Resource env context tests.
- Allocation tests when the adapter supports Resource allocations.
- Project env rendering tests.
- CLI help and command snapshot coverage.
- Non-happy-path tests for install/start/readiness/admin failures where feasible.

Real upstream Managed Resource artifacts and public publication belong to later artifact recipe/publication work and release-candidate validation.

## Orchestration

After this design spec and the implementation plan are approved, the final plan will be converted into Solo todos with blocker relationships and worktree metadata.

Current Solo blocker shape:

- `#68` Shared Managed Resource runtime foundation.
- `#67` Shared SQL foundation, blocked by `#68`.
- Mailpit, Redis, and RustFS adapter todos are blocked by `#68`.
- MySQL and Postgres adapter todos are blocked by both `#68` and `#67`.

Implementation agents should be spawned only after the implementation plan is approved. Each agent should receive one worktree, the relevant Solo todo, the shared scratchpad, exact blocker context, and verification commands. Solo scratchpads should hold cross-lane decisions, and Solo timers should be used for orchestrator check-ins instead of polling loops.

## Verification

Foundation and adapter implementation plans should prefer focused checks:

```shell
cargo nextest run -E 'test(<specific_test_name>)'
cargo insta test --accept --test-runner nextest -- <specific_test_name>
cargo fmt --all -- --check
git diff --check
```

Before each PR is considered complete, run the narrowest crate set that covers the touched behavior, then run clippy for the relevant workspace surface:

```shell
cargo nextest run -p daemon -p resources -p state -p config -p cli --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

If dependency changes are needed, avoid broad lockfile churn and use precise updates.

## Open Decisions

None. The approved direction is seven workstreams, daemon-local runtime orchestration, a small SQL foundation, full vertical adapter slices, fixture-based PR tests, existing state tables for resource/allocation env context, and Solo-backed orchestration after the implementation plan is approved.
