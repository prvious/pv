# Test Strategy: Epic 2 - Store, Host, And Install Infrastructure

## Scope

Epic 2 tests cover:

- canonical host path helpers;
- layout validation;
- store schema version;
- applied migration records;
- migration runner behavior;
- contract versioning decision visibility;
- install plan model and dependency ordering;
- bounded download scheduler;
- install executor failure behavior;
- atomic shim writer;
- durable state persistence and daemon signal seam.

## Test Objectives

- Prevent accidental filesystem layout drift.
- Prove machine-owned state has a visible version/migration path.
- Prove install planning is deterministic before real resources use it.
- Prove failures do not expose incomplete installs.
- Prove daemon signaling happens after durable persistence, not before.

## ISTQB Techniques

| Technique | Epic 2 usage |
| --- | --- |
| Equivalence partitioning | Valid/invalid resource names, valid/invalid versions, runtime/tool/service plan items. |
| Boundary value analysis | Empty version, empty resource name, one worker, zero dependencies, missing dependency. |
| Decision table testing | Install result combinations: download fail, prerequisite fail, shim fail, persist fail, signal success. |
| State transition testing | migration pending -> applied -> recorded; plan pending -> downloaded -> installed -> exposed -> persisted -> signaled. |
| Experience-based testing | Avoid prototype path sprawl and command-specific install glue. |

## Test Matrix

| Area | Required tests |
| --- | --- |
| Path helpers | Every canonical path family, isolated `HOME`, unsafe segment rejection. |
| Layout validation | Reject top-level binaries, data under binary roots, unregistered path families. |
| Store schema | Schema version present, applied migrations recorded, migration failure behavior. |
| Contract version decision | Test or doc assertion that versioning is implemented or explicitly deferred. |
| Plan graph | Duplicate identities, missing dependencies, deterministic topological order. |
| Downloads | Bounded parallelism, cancellation, per-item failures. |
| Installs | Dependency order, failed prerequisite skip, structured results. |
| Shims | Atomic replacement, permissions, temp cleanup. |
| Persistence/signaling | Persist before signal, no signal on failure or dry run. |

## Test Data

- Use `t.Setenv("HOME", t.TempDir())` for path and state tests.
- Do not use `t.Parallel()` with `t.Setenv`.
- Use fake downloader and installer adapters.
- Use deterministic clocks where status/results include time.
- Do not download artifacts.
- Do not run daemon processes.

## Verification Commands

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Exit Criteria

- All Epic 2 tests pass.
- Root verification passes.
- No expensive artifact workflows were run.
- New path and installer APIs are documented enough for Epic 3 resources to use.
