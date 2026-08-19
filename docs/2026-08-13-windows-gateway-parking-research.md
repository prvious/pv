# Windows Gateway And Parking Research Scratchpad

## Status

This document is a discovery-phase scratchpad. It records settled decisions,
current findings, open product questions, and candidate implementation work. It
is not an implementation specification and does not override `DESIGN.md`.

## Settled Decisions

### Use The Caddy Admin API On Every Platform

PV will use the Caddy admin API as the single configuration reload mechanism on
every supported platform for:

- the Gateway Caddy process; and
- Project-serving FrankenPHP workers, which use Caddy configuration internally.

PV will remove the macOS-only `SIGUSR1` reload path instead of keeping separate
signal and admin-API implementations.

The intended properties are:

- one portable reload mechanism across macOS, Linux, and Windows;
- a distinct loopback admin endpoint for each managed Caddy/FrankenPHP process;
- candidate configuration validation before promotion;
- atomic reload with the previous working configuration retained on failure;
- direct daemon-to-admin-API communication rather than shelling out to run a
  reload command; and
- PV remains the source of truth, so Caddy config persistence should be disabled
  where appropriate.

Implementation details still to resolve include admin-port allocation, endpoint
hardening, Caddyfile versus native JSON payloads, and migration of currently
running PV-owned processes.

## Parking Discovery Findings

### Parking Does Not Require PHP In The Gateway

The Gateway only needs a resolved hostname-to-worker mapping. The selected
FrankenPHP worker owns the hostname-to-document-root mapping and PHP serving.
A parked child can therefore flow through the existing runtime plan as a derived
Project-serving entry regardless of whether the Gateway binary is Caddy or
FrankenPHP.

### Directory Events Must Be Wake-Up Signals, Not Desired State

Filesystem notification payloads are not reliable enough to mutate routing
state directly. Platforms can emit different event sequences for the same
operation, rename can look like removal plus creation, notification buffers can
overflow, and some filesystems do not emit native events.

The reliable model is:

1. Persist the parked root as desired state.
2. Enumerate its immediate eligible child directories into a complete snapshot.
3. After a successful scan, transactionally record the last-valid discovered
   snapshot in `pv.db` as observed state.
4. Treat that snapshot as the current desired parked-site set.
5. Use filesystem activity only to request a debounced rescan.
6. Reconcile workers and Gateway routes from the newly computed complete set.

Example snapshots:

```text
Previous: {project-099, project-101}
Current:  {project-099, project-100}

Added:    {project-100}
Removed:  {project-101}
```

PV does not need to infer what an operating-system event meant. `project-100`
is added because it exists in the successful current scan, and `project-101` is
removed because the parked root is readable and it no longer exists.

### Reconciliation Lifecycle

PV should perform a complete parked-root scan:

- when `pv park` records a root;
- when the daemon starts;
- after a debounced filesystem notification or polling change;
- after wake/resume when feasible; and
- periodically as a repair path if native notifications are selected.

For an addition, PV should configure and verify the demanded worker before
publishing the Gateway route. For a removal, PV should remove the Gateway route
before stopping a worker that is no longer demanded. The current runtime
reconciliation already follows this broad ordering.

A rename is naturally one removal plus one addition, applied through one
reconciliation. Bursts such as a clone or project generator should be debounced
and coalesced into one scan.

### Scan Failure Must Not Mean An Empty Parked Root

If a parked root cannot be read because of a transient I/O, permission, network
mount, or removable-drive failure, PV should preserve the last successfully
resolved routes and report the parked root as degraded. It must not interpret a
failed scan as if the user deleted every child.

The last successful discovered-child snapshot must be retained in `pv.db` if
this behavior is expected to survive daemon restart. The parked root is desired
state; the snapshot is observed/last-valid state and is replaced only after a
successful authoritative scan. Generated Caddy configuration must not become an
alternate source of truth.

When the root is readable, an absent child can be treated as an actual removal.
The behavior for a parked root that is itself missing remains an open product
decision; preserving last-known-good routes until explicit `unpark` is the safer
candidate behavior for transient volumes.

## Watcher Options Under Evaluation

### Option A: Poll And Reconcile

Periodically enumerate only the immediate children of every parked root and
compare the successful snapshot with the previous snapshot.

Advantages:

- same behavior on macOS, Linux, and Windows;
- cannot permanently miss a create/delete event;
- naturally handles notification overflow, sleep/resume, and network filesystems;
- reuses PV's current polling and reconciliation style; and
- no recursive watches, so `vendor/` and `node_modules/` activity is irrelevant.

Tradeoff:

- performs directory reads even when nothing changed.

### Option B: Native Notification Plus Authoritative Rescan

Use a maintained cross-platform notification library to watch each parked root
non-recursively. Any relevant event requests a debounced complete scan. Also run
a slower periodic scan to repair missed events.

Advantages:

- low-latency discovery without frequent idle directory reads; and
- the same high-level implementation can use FSEvents/kqueue, inotify, and
  `ReadDirectoryChangesW` through a maintained library.

Tradeoffs:

- more moving parts than polling alone;
- native notification queues can overflow; and
- network, WSL, emulated, and unusual filesystems may not emit notifications.

### Rejected: Apply Raw Event Deltas Directly

PV should not add or remove routes directly from raw create/delete/rename event
payloads. This makes correctness depend on platform-specific event sequences and
makes missed events persistent.

## Current Recommendation

Start with poll-and-reconcile unless measurements show that scanning immediate
children is materially expensive. It is the smallest cross-platform design and
matches PV's existing watcher model. Native notifications can later become an
optimization that only wakes the same authoritative scanner; they should never
become the source of truth.

The polling interval and debounce window are not decided. They should be chosen
from integration measurements rather than copied from the existing Project
config watcher automatically.

## Parking Product Questions Still Open

- Are only immediate, non-hidden child directories eligible?
- Are directory symlinks followed?
- Does an explicitly linked Project win a hostname collision with a parked child?
- Does deleting a parked child immediately remove its route?
- What happens when the parked root itself is deleted or temporarily unavailable?
- Are parked children full PV Projects with resources and env rendering, or only
  derived serving entries until explicitly linked?
- How are stable identity and per-site settings represented for a derived child?
- What happens while a new directory is only partially cloned or generated?
- Can a parked child declare additional hostnames, or only its derived primary
  hostname?

These semantics are not currently covered by `DESIGN.md` and must be approved
before implementation.

## Candidate Work Items

### Admin API Reload Work Item

- Add a distinct persisted admin endpoint assignment for the Gateway and every
  FrankenPHP worker.
- Render the admin endpoint instead of `admin off`.
- Add a daemon-side Caddy admin client that loads validated configuration.
- Preserve the previous active config and runtime when loading fails.
- Disable Caddy config persistence where needed so `pv.db` and generated config
  remain authoritative.
- Remove `ProcessSignal::Reload` and the macOS `SIGUSR1` reload implementation.
- Keep graceful terminate/kill process control separate from config reload.
- Add Gateway and worker integration tests for successful reload, failed reload,
  rollback, adoption, and unique admin endpoints.
- Verify the same path on macOS, Linux, and Windows.

### Parking Research And Design Work Item

- Approve the open parking semantics and update `DESIGN.md`.
- Choose polling alone or native-notification wakeups backed by periodic polling.
- Define parked-root and derived-site state in `pv.db`.
- Reuse hostname normalization, document-root detection, Project config loading,
  PHP runtime grouping, Gateway routing, and TLS planning.
- Extend watcher registration when parked roots or discovered children change.
- Add integration coverage for initial scan, create, delete, rename, collision,
  mixed PHP tracks, partial/invalid projects, failed root scans, daemon restart,
  and missed-event repair.

## Research References

- [Caddy administration API](https://caddyserver.com/docs/api)
- [Caddy global `admin` option](https://caddyserver.com/docs/caddyfile/options#admin)
- [`notify` cross-platform watcher documentation](https://docs.rs/notify/latest/notify/)
- [Windows `ReadDirectoryChangesExW` behavior](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesexw)
- [Yerd parked-root watcher](https://github.com/forjedio/yerd/blob/main/bin/yerdd/src/fs_watch.rs)
- [Yerd parked-site scanner](https://github.com/forjedio/yerd/blob/main/bin/yerdd/src/startup.rs)
