# Test Issues Checklist: Epic 2 - Store, Host, And Install Infrastructure

## Test Issue #135: Store Migration And Filesystem Layout

Labels: `test`, `priority-high`, `control-plane`, `ready-for-agent`

Blocked by: #131, #132, #133, #134.

Required coverage:

- [ ] Every canonical path family returns the expected path under isolated `HOME`.
- [ ] Tests that set `HOME` do not call `t.Parallel()`.
- [ ] Unsafe resource names are rejected.
- [ ] Unsafe version path segments are rejected.
- [ ] `~/.pv/bin` cannot be used as a real binary root.
- [ ] Data paths cannot be produced under runtime/tool/service binary roots.
- [ ] Store exposes schema version.
- [ ] Applied migration records are persisted in order.
- [ ] Migration runner executes pending migrations in order.
- [ ] Migration failure returns a clear error.
- [ ] Contract-version decision is documented as `version: 1` owned by Epic 4 issue #171.

Exit evidence:

- [ ] Path, layout, migration, and contract-version tests or doc assertions are present.
- [ ] Root verification passes.

## Test Issue #141: Install Planner Scheduling And Failure Behavior

Labels: `test`, `priority-high`, `control-plane`, `ready-for-agent`

Blocked by: #136, #137, #138, #139, #140.

Required coverage:

- [ ] Plans accept runtime, tool, and service item kinds.
- [ ] Duplicate plan identities are rejected.
- [ ] Missing dependencies are rejected before work starts.
- [ ] Topological order is deterministic.
- [ ] Download parallelism never exceeds the configured bound.
- [ ] Context cancellation stops scheduling.
- [ ] Per-item download failures are reported.
- [ ] Installer execution follows dependency order.
- [ ] Failed prerequisites skip dependent installs.
- [ ] Results distinguish ready, skipped, and failed items.
- [ ] Atomic shim writer writes temp file, sets executable permissions, renames into place, and cleans failed temp files.
- [ ] Successful durable plans persist state before signaling.
- [ ] Failed and dry-run plans do not signal and do not advertise completed work.

Exit evidence:

- [ ] Tests use fake resolvers, downloaders, installers, and signal adapters.
- [ ] No artifact downloads run.
