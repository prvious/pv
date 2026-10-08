# Per-Service Monitor Design

Status: accepted for implementation on 2026-10-08. The implementation is not complete. The owner approved the revised plan and its three refinements: explicit release, database commit before daemon release, and refusal of destructive work after unproven cleanup.

DESIGN.md describes current production behavior until the integration PR changes it. This proposal can change when tests or source evidence show a better solution.

## Purpose

PV currently finds surviving runtimes through PID files and process identity checks after a daemon restart. The daemon can detect a later exit through health checks, but it cannot recover an exit result after its own event observer has died.

macOS can report future exit status to an authorized observer that is not the parent. A child-only restriction is not the reason for monitors. PV needs a surviving owner that retains the exact result while the daemon is absent and completes an accepted stop when the daemon dies.

One monitor starts and owns one runtime: the Gateway, a full PHP worker runtime key, or a Managed Resource track. In-process DNS and short-lived validation commands have no monitor. The monitor code belongs in the daemon crate. The existing PV binary supplies hidden monitor and exec-gate entry points. A thin daemon example uses the same implementation in process tests.

The monitor loads no database, artifact manifest, or reconciliation state. One extra process per runtime has a cost. Measure startup time, memory, threads, and idle activity before claiming a small footprint.

## Policy and lifetime

The daemon selects the graceful signal and grace period. The monitor executes the complete stop sequence, including escalation and verification, without another daemon command. Preserve the existing grace periods. Postgres uses SIGINT; other runtimes use SIGTERM.

Production runtimes continue through daemon absence without a timer. Disconnect is not cancellation. After startup acceptance, daemon death leaves a discoverable monitor. Ordinary caller cancellation sends an explicit stop for that instance.

The monitor does not restart a runtime. The daemon keeps readiness, reconciliation, and restart policy. A spontaneous leader exit starts the startup-supplied fallback cleanup sequence for remaining owned group members.

## Identity and startup

Use `~/.pv/run/m/<subject-hash>/` for the control socket and recovery envelope. Store the full subject and a fresh instance token. The short hash reduces socket length; it does not make every home path valid. Reject an overlong socket path before spawning a runtime.

Use a persistent sibling reservation at `~/.pv/run/m/<subject-hash>.lock`. Acquire it before monitor startup and transfer the same locked file description without a gap. The monitor holds it through OS process death. Never unlink it during ordinary replacement.

Send the command, arguments, private environment, log path, and fallback stop parameters through a private startup channel. Keep secrets out of arguments, recovery records, and debug output.

A small exec gate closes the gap between process creation and recovery-record publication:

1. Start the gate as the future runtime group leader. It waits on a private descriptor.
2. Capture its birth identity. Publish the monitor identity, gate identity, intended runtime identity, full subject, instance token, and boot identity.
3. Send permission to exec only after publication succeeds.
4. The gate replaces itself with the runtime. EOF before permission starts no service.
5. Confirm exec through explicit failure reporting and execution identity or terminal state. EOF alone is not proof of success.

Do not block inside `pre_exec`. Use the existing exec helper in the hidden gate mode. Test PID and birth-identity continuity across exec on supported macOS versions.

The monitor starts in a separate session. Its runtime has its own process group. The gate retains startup protection until exec or exit. The runtime must not inherit the reservation descriptor.

Return startup success only after the private request is accepted, the recovery envelope is published, exec is confirmed, and the control socket is available. Service readiness is a separate check. Preserve direct-child cleanup on ordinary startup errors.

## Leader exit and cleanup

Report leader exit separately from cleanup completion. Retain the exact terminal status in memory. A surviving monitor keeps this one result until explicit release. There is no exit file, exit journal, or event history.

Observe the direct child's terminal status without reaping while group signals remain possible. Complete supported containment cleanup, then reap once. No competing Tokio child waiter may reap it first.

Group inspection must distinguish live members, completion, and unknown. Permission failures, incomplete snapshots, and unverified identities are unknown. An empty root group is not universal proof of resource cleanup.

Postgres normally creates children in separate sessions. Clean Postgres exit after SIGINT provides resource-specific evidence. A forced or abnormal postmaster exit does not prove backend cleanup. Keep recovery records and capable binaries, and block replacement, uninstall, and prune when required cleanup remains unproven. Do not add a generic process-tree tracker.

## Control contract and consumption

Use bounded newline-delimited JSON on an owner-only Unix socket. Check the connecting UID. The controller must also authenticate the socket peer against the recorded monitor birth identity and instance. UID alone is insufficient.

Freeze the basic version, state, parameterized stop, explicit release, framing, and recovery-envelope contracts. Permit additive optional fields. Application-version differences alone do not restart a runtime. A truly incompatible required contract needs controlled stop and replacement through reconciliation.

State is authoritative after each connection. Push events only reduce delay. State contains the subject, instance, runtime identity, exact terminal result when available, and independent cleanup state.

A stop request supplies the signal and grace period. Repeated requests must not create competing stop sequences or reset the original deadline. Stop continues after client disconnect.

Use one consumption path:

1. Stop if necessary.
2. Receive the exact terminal result and completed cleanup.
3. Consume the result.
4. Send explicit release for that instance.
5. Verify monitor OS death.
6. Acquire the sibling reservation and compare the full subject and instance again.
7. Remove only that instance's records.

For daemon consumption, commit the instance-scoped latest observation in `pv.db` before release. A failed commit keeps the result available. Diagnostic logging is best effort after the commit.

An explicit CLI maintenance stop can consume its received result without opening or migrating a database. A healthy database is not required to disable or uninstall PV.

A stop reply alone never consumes the result. Lost stop replies leave it available. Release is repeatable and instance-specific. Accept release only after verified cleanup. A stale release must not affect a replacement.

The controller removes the directory. The monitor never recursively removes a directory that a replacement can reuse. Listener closure, task cancellation, and a release reply do not prove process death.

## Recovery

Find instances by listing exact monitor directories. Do not scan processes by name. A missing socket or timeout does not prove that the monitor is dead.

Use recorded process birth and boot identities to distinguish a dead monitor from a live unresponsive monitor. After verified monitor death, a small current-binary recovery operation can stop an exactly identified orphan runtime. Missing, changed, or stale identity cannot authorize a signal.

Replacement requires proven runtime cleanup, actual monitor death, the sibling reservation, and a fresh instance comparison. Keep evidence when any required step is uncertain.

There is no exit-status retention guarantee after monitor death, reboot, or kernel failure. Parent ownership does not contain arbitrary descendants that escape their process group.

## Configuration and Caddy

Store prepared-byte proof separately from live-instance applied proof in `pv.db`. Prepared or staged proof can survive runtime death and monitor-record deletion. Applied proof belongs to the captured live instance.

Make latest-observation, apply, rollback, and replacement writes conditional on the instance. A delayed result from instance A must not overwrite running instance B.

Before sending any Caddy admin HTTP bytes, obtain fresh authenticated monitor state and verify the actual Caddy peer and runtime group. Keep the existing peer checks. A cached state snapshot or matching UID is insufficient.

## Captured logs

Attach runtime stdout and stderr directly to append-mode files. Use the same monitor copy-and-truncate rotation for all captured runtime output, including Gateway and workers. Keep Caddy-owned access and error files on Caddy's existing rotation. Each file has one rotation owner.

Use a 10 MiB threshold and five timestamped archives. This is not a hard disk quota. Output can exceed the threshold between checks, and copying needs extra space.

Capture the source length at rotation start. Copy that finite length to a private temporary archive, publish the completed archive, then truncate the same active inode. Temporary names must not match log discovery. If copying fails, retain the active file and expose the failure. Use descriptor-based type and ownership checks with no-follow opens.

Concurrent output can be lost between the captured copy boundary and truncation. This occurs during each concurrent rotation window. Its size is not a fixed number of lines. Accept this diagnostic loss instead of adding an output pipeline.

File work must not block control handling, exit observation, or monitor process exit. A started Tokio blocking task cannot be stopped by aborting its handle. Use a shutdown boundary that permits actual process exit despite held file work. A replacement waits for that exit before it reuses the active file.

Test archive order, retention, copy failures, unsafe paths, append after truncation, and the existing log follower across truncation. Do not test an unsupported promise of zero concurrent-byte loss.

## Disable, uninstall, and lifecycle admission

Fix disable and uninstall before monitor integration. Close lifecycle admission. Check LaunchAgent ownership before unloading it, then verify daemon process death. Hold jobs exclusion and stop every exactly verified recorded runtime. Remove runtime records only after successful cleanup. A missing plist still requires runtime cleanup.

Use one external shared/exclusive admission lock at `~/.pv-runtime-lifecycle.lock`. Foreground state mutations and short daemon bootstrap hold it shared. Disable and uninstall hold it exclusive through final deletion. Keep long read-only commands outside it.

The existing helper lock cannot also be the admission lock: setup and update hold it while they start and health-check the daemon. Shared admission allows that bootstrap without a new handshake. Keep the helper lock for its current purpose. Nested operations use the outer command's admission guard.

The daemon retains its process identity record through process exit. This permits a controller to wait for actual process death after the IPC listener closes. Internal jobs locks can be removed only after the daemon is dead and external admission still excludes new state creation.

## Tests and update cutover

Tests pass a test-process lifeline descriptor before monitor spawn. Lifeline closure starts cleanup and releases a completed test monitor without daemon acknowledgment. Keep the fake's independent safeguard for ordinary tests. Disable that safeguard only in explicit tests that prove the monitor's own cleanup. Outer guards must report cleanup failures.

Test actual subprocess death at startup boundaries and during an accepted stop. Prove monitor exit with held rotation work and with the test lifeline still open. Include a real PV hidden-command smoke test, not only the example binary.

There is no legacy-runtime adoption after cutover. The first monitor update must stop old runtimes and refuse activation over unresolved legacy records. Keep the jobs admission barrier during activation and health checks. Compatible updates preserve runtime PIDs.

Incompatible rollback must keep a capable stop or recovery path. Recovery uses the current binary even after an older monitor's release directory is pruned. Test failed activation and automatic rollback.

Use five PRs: the existing spec/plan PR, a standalone shutdown fix, monitor core, production integration, and obsolete-code removal. Only the last three form the `gh stack` stack. Integration must remain safe and buildable if final removal is delayed.

Linux and Windows remain explicitly unsupported for this macOS implementation. Future Linux containment can use native subreaper and cgroup features when that work starts. Do not add unused Linux scaffolding now.

The detailed tasks and acceptance matrix are in [the implementation plan](../plans/2026-10-08-per-service-monitor.md).

## Source checks

- [Apple process-event implementation](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_event.c): authorized non-parent exit-status observation and registration timing.
- [Apple process-group enumeration](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/libsyscall/wrappers/libproc/libproc.c): count and error behavior.
- [PostgreSQL child initialization](https://raw.githubusercontent.com/postgres/postgres/REL_18_STABLE/src/backend/utils/init/miscinit.c): separate child sessions.
- [PostgreSQL fast shutdown](https://www.postgresql.org/docs/18/server-shutdown.html): SIGINT behavior.
- [Tokio blocking-task contract](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html): started work cannot be aborted through its task handle.
