# PV Per-Service Monitor: Implementation and Review Plan

Date: 2026-10-08.

Status: approved for implementation on 2026-10-08. The owner accepted the revised plan and its three refinements, then requested implementation. Three independent design reviews completed. Opus timed out; its retry still awaits explicit source-sharing approval. The standalone shutdown fix is implemented in [PR #411](https://github.com/prvious/pv/pull/411); CI and merge review are pending. Monitor implementation has not yet started.

This plan records the owner's choices from the current discussion. It replaces older recommendations where they conflict. The monitor spec on PR #406 now reflects these choices. Source evidence and tests can still justify improvements.

Implementation record, 2026-10-08:

- PR 1, #406: the approved spec and plan are published.
- PR 2, #411: disable and uninstall now wait for daemon process exit and stop verified recorded runtimes before deletion. Uncertain cleanup preserves recovery files. The separate external admission lock closes state-recreation races.
- Local validation: 1066 selected affected-crate tests passed, plus both real Postgres checks. Formatting, workspace Clippy, and the dependency audit passed. One native listener test also fails on unchanged main on this Mac and remains excluded; its snapshot is unchanged.
- The wider run exposed a fixture port race after a worker stop releases its allocation. The fixture now seeds a fresh reserved port before each restore. Code and test reviewers passed that fix.
- Linux cross-compilation needs the CI host: the local Mac lacks `x86_64-linux-gnu-gcc`. CI covers Linux, Windows, and the supported macOS hosts.
- PRs 3–5 remain to be implemented and linked with `gh stack`. PR 2 must land before the core PR targets main.

## 1. Purpose and terms

PV already keeps services running during a daemon restart. The current daemon uses pid files and runtime metadata to find those processes again. It then checks process identity before control operations.

A monitor gives each runtime a stable parent. The daemon controls the runtime through that monitor. This reduces repeated process adoption and gives future platform implementations a smaller process-control boundary.

Immediate exit detection alone does not justify the monitor. macOS kqueue can report exit status for a non-child process when the observer has the required permission. A smaller macOS-only solution remains a valid alternative for reviewers to assess.

| Term | Meaning |
| --- | --- |
| Runtime | The Gateway, one PHP runtime worker, or one Managed Resource track process. |
| Runtime subject | The stable service identity, such as `postgres:18` or a full PHP runtime key. |
| Runtime instance | One start of that subject. A replacement has a different instance identity. |
| Monitor | A small process that starts one runtime and remains its parent. |
| Leader exit | The runtime's root process has exited. Other group members can still exist. |
| Cleanup complete | PV has completed and verified the supported containment cleanup. |
| Exit acknowledgment | The daemon has recorded the result and permits release of the completed monitor. |

The monitor also retains a result while the daemon is absent and finishes an accepted stop if the daemon dies. A daemon-owned exit watcher cannot provide those two guarantees on its own. Linux work is expected in the next few months and strengthens the case for this small process boundary. macOS remains the implementation target for this plan.

## 2. Agreed choices and review authority

### Choices from the owner discussion

- Use one monitor for each runtime. Do not add a monitor for in-process DNS or short-lived validation commands.
- Ship monitor mode in the existing `pv` binary. Put the code in the daemon crate.
- Use a thin daemon example binary for monitor tests. Both entry points call the same code.
- Production runtimes continue when the daemon disconnects. Do not add a daemon-absence timer.
- Keep the exact exit result in monitor memory across daemon downtime. Do not add an exit-status file or database event journal.
- Report leader exit separately from cleanup completion.
- The daemon selects the stop signal and grace period. The monitor executes the entire sequence without further daemon commands.
- Keep stdout and stderr attached directly to append-mode files. Do not add runtime-output pipes.
- Use one common monitor rotation implementation for captured stdout and stderr from every runtime, including workers and the Gateway.
- Keep the Gateway's existing Caddy-owned access and error files and their native rotation.
- Do not add a separate native worker logging arrangement merely to solve rotation.
- Accept possible loss of concurrent diagnostic output during copy-and-truncate rotation. Do not describe the loss as a guaranteed number of lines.
- Use `~/.pv/run/m/<id>/` for monitor control and recovery records.
- Tests give monitors an inherited test-process lifeline descriptor before spawn. Keep `pv-fake`'s own safeguard.
- Do not adopt old pid-file runtimes after the monitor cutover. Stop them before the first monitor-based update.
- Use five PRs for macOS. The last three form a stack managed with `gh stack`.

These choices are open to evidence-based criticism. The owner explicitly permits proposals that oppose DESIGN.md. Reviewers must assess whether the solution is good, not only whether this document describes it correctly. A proposed change must state its benefit, cost, and required owner decision.

### Refinements approved after review

Keep the three main choices: memory-only exit retention, monitor rotation of direct append files, and monitor code in the daemon crate. The owner also approved these three refinements:

1. Use an explicit instance-scoped release after both daemon stops and CLI stops. A successful stop reply alone must not consume the result.
2. Require the daemon's database observation commit before release. Keep diagnostic logging best effort. An explicit CLI maintenance stop can consume its received result without a healthy database. Do not make disable or uninstall migrate a database merely to stop runtimes.
3. Block replacement, uninstall, and prune when required cleanup remains unproven. This includes Postgres backends that normally leave the postmaster's process group.

The tasks below use these approved refinements. The startup gate, lock scope, identity checks, and bounded file-copy work are correctness details needed to meet the existing requirements. Their platform behavior still needs the specified integration tests.

## 3. Success conditions and scope

The macOS work is complete only when all of these conditions hold:

1. An ordinary daemon restart preserves runtime PIDs and service availability.
2. Runtime exit is visible without waiting for the 30-second health tick while the daemon is connected.
3. A surviving monitor returns its exit result after the daemon returns.
4. A stop continues through escalation when the requesting daemon dies.
5. A replacement cannot overlap unresolved cleanup or old monitor work, or remove another instance's records.
6. Disable and uninstall verify runtime stop before destructive state removal.
7. Config reload, rollback, pending replacement, and retained staged proof still work.
8. Captured runtime output rotates without routing writes through the monitor.
9. Test-process death starts monitor cleanup, including SIGKILL of the test process.
10. Every stack PR builds and passes its relevant tests without a later PR.
11. Linux and Windows still compile through explicit unsupported implementations.

This plan does not implement Linux or Windows service supervision. It does not add automatic runtime restart inside a monitor. It does not promise complete log capture, exit retention after monitor death, or containment of arbitrary descendants that escape the supported process group.

Do not change release recipes, publish a release, alter the owner's installed PV state, or include another session's PR #402 in this work.

## 4. PR structure and GitHub stack

| PR | Proposed title | Base | Deliverable |
| --- | --- | --- | --- |
| 1, existing #406 | `docs: specify per-service runtime monitors` | `main` | Revised design, this plan, and cutover requirements. |
| 2, new | `fix: stop managed runtimes before disabling PV` | `main` | A working shutdown fix using the current supervisor. |
| 3, new | `feat: add per-service runtime monitors` | `main` after PR 2 | The monitor engine, hidden entry points, and process-level contract tests. Production daemon supervision stays on its current path in this PR. |
| 4, new | `feat!: supervise managed runtimes through monitors` | PR 3 branch | Production integration and the explicit one-time cutover. |
| 5, new | `refactor: remove legacy runtime adoption` | PR 4 branch | Removal of obsolete adoption and script fallback code, plus final documentation. |

Five PRs includes the existing proposal PR. PRs 1 and 2 are independent. The implementation stack contains only PRs 3, 4, and 5.

Proposed implementation branches use the required prefix: `feat/service-monitor-core`, `feat/service-monitor-integration`, and `feat/remove-legacy-runtime-adoption`. Choose a separate `feat/` branch for PR 2. Do not create these branches during planning.

Use separate worktrees or an existing suitable managed worktree for implementation. Keep PR #406's current checkout for the planning documents. Never reset another session's checkout. Use `git -C <absolute-worktree>` for all Git commands.

After the three implementation PRs exist, link them in bottom-to-top order:

```shell
gh stack link <core-pr-url> <integration-pr-url> <removal-pr-url> --base main
gh stack view
```

Use the verified PR URLs rather than ambiguous numeric stack identifiers. Check that each PR has the intended base. Attach every created PR to this Codex chat. Use `gh stack` to maintain the links when bases or branches change.

Do not merge during this planning task. After checks, review, and explicit owner approval, merge with:

```shell
gh stack merge <verified-stack-number> --merge --yes
```

The extension uses atomic stack merge. It can merge the full stack or the lower part through a selected PR. The selected merge scope must be explicit. Do not enable the app's CI-monitor switch.

## 5. PR 1: revise the proposal

Relevant files:

- `docs/superpowers/specs/2026-10-08-per-service-monitor-design.md`
- This plan.
- DESIGN.md: Platform Scope, Daemon Lifecycle, Gateway and worker reload contract, Filesystem Layout.

Tasks:

- [ ] Replace the old parent-only macOS exit-status argument with the stable-parent and future-platform reasons.
- [ ] Apply the choices in section 2. Remove the output-pipe and connection-timer designs.
- [ ] Specify a stop request with daemon-supplied signal and grace period.
- [ ] Specify independent leader-exit and cleanup states.
- [ ] Keep exit retention in monitor memory until acknowledgment. Define the explicit-stop and test-lifeline cases.
- [ ] State how directory ownership, instance identity, and record removal prevent replacement races.
- [ ] Define config proof's daemon-owned home and the separate Caddy peer verification requirement.
- [ ] Define stable state, stop, acknowledgment, and recovery-file behavior across application updates.
- [ ] Remove automatic restart on an application-version difference. A truly incompatible required monitor contract needs a controlled replacement through reconciliation.
- [ ] State that a short hash reduces socket path length but cannot make every arbitrary home path fit. Reject an overlong path before runtime spawn.
- [ ] Keep DESIGN.md unchanged until production monitor behavior is implemented. If PR 4 becomes independently mergeable, update its affected current-behavior text there; use PR 5 for final removal wording.

Verification: inspect the complete document for conflicting rules, run `git -C <worktree> diff --check`, and complete the counselors design review. Runtime tests are unnecessary for this documentation-only PR.

## 6. PR 2: fix disable and uninstall before monitors

Current evidence:

- `crates/cli/src/commands/daemon.rs`: `disable` unloads the LaunchAgent and removes its plist. It does not stop supervised runtimes.
- `crates/cli/src/commands/setup.rs`: `uninstall` calls disable, then can remove `run/`, configs, and binaries.
- `crates/daemon/src/server.rs`: shutdown can drain background work. A closed daemon socket alone does not prove process termination.
- `crates/state/src/update_lock.rs`: update, jobs, and helper lifecycle locks already exist.

Required flow:

1. Check ownership of the LaunchAgent. Keep existing conflict refusal.
2. Serialize lifecycle mutations. Stop launchd from restarting the daemon, and verify daemon termination before final runtime enumeration.
3. Acquire jobs exclusion after the daemon has terminated. Stop every verified PV-owned runtime with its resource stop policy.
4. Confirm supported group cleanup and required listener disappearance.
5. Remove exact runtime records only after successful verified stop.
6. Remove the owned plist and report success. Uninstall may then remove state.

The missing-plist and already-unloaded cases must still stop remaining owned runtimes. A missing daemon is not proof that no runtime exists. Normal `daemon:restart` and self-update must keep their runtime-preserving behavior.

Implementation tasks:

- [ ] Add the smallest shared stop-all operation in the daemon/supervisor layer. Do not copy process-control code into CLI commands.
- [ ] Enumerate exact recorded runtime paths without opening or migrating the database merely to stop them. Do not discover ownership through process-name scans.
- [ ] Keep recorded identity checks immediately before signals and record removal.
- [ ] Use adapter-selected Postgres SIGINT and the existing grace periods. Do not add SIGQUIT without a separate decision and real Postgres evidence.
- [ ] Preserve records and binaries if any stop cannot be verified. Return a failure that names all failed subjects.
- [ ] Ensure disable and uninstall share the operation without nested lock acquisition.
- [ ] Test admission races with setup, daemon enable/restart, and application update.

### Lifecycle exclusion

Use one external `~/.pv-runtime-lifecycle.lock` as the outer admission lock. Source tracing ruled out reuse of the helper lock: setup and update hold that lock while starting and health-checking the daemon. Daemon bootstrap needs shared admission at the same time. A separate native file lock permits both without a new handshake. Keep the existing helper lock for helper operations.

- [ ] Hold the lifecycle lock shared for foreground operations that can create or modify PV state. Audit writable database opens, not only commands named install or update.
- [ ] Hold it shared for setup, application update, enable, restart, and short daemon bootstrap. Hold it exclusive for disable and uninstall. Acquire it before the first layout, shim, database, or other state mutation.
- [ ] Acquire outer admission before existing update, jobs, and helper guards. Keep existing nonblocking inner coordination; nested helpers must not reacquire outer admission.
- [ ] Pass an already-held guard to nested operations. Setup and uninstall must not reacquire it through enable or disable.
- [ ] Keep daemon jobs on the existing jobs lock. Release a foreground jobs guard before requesting daemon reconciliation.
- [ ] Preserve the application's update-phase boundary. Release the parent phase's guards before starting a continuation that must acquire its own guards.
- [ ] Hold exclusive lifecycle admission through final deletion. Stop and verify the daemon before final enumeration. With those participants excluded, no valid command can recreate `run/jobs.lock` during removal.
- [ ] Keep read-only status, jobs, and log-follow operations read-only. Do not make an indefinite `logs --follow` session block uninstall.
- [ ] Trace daemon bootstrap separately. It currently opens the database before jobs admission. Prove that manual `daemon:run` and launchd startup cannot recreate state after final stop. Also prove that the outer guard does not prevent enable/update health checks from completing. Do not hold a shared lifecycle guard for the daemon's whole lifetime.

Both ordinary uninstall and prune remove `run/`. A guard on an unlinked lock inode alone cannot exclude a new file at the same path. The external admission lock prevents that path recreation. A busy lock returns a clear coordination failure before partial deletion.

Caller map: the common CLI router holds admission for each state-changing command; nested setup, update, and daemon helpers use that outer guard. Production daemon entry holds shared admission through bootstrap and process-record publication, then releases it. Daemon jobs keep their existing jobs lock. Disable and uninstall capture daemon identity before unload, wait for OS process exit, then enumerate runtimes under jobs exclusion. Keep the daemon identity record through process exit, including the period after listener closure. Tests must prove these scopes; do not add a general lease protocol.

### Cleanup observation

- [ ] Replace leader-only group completion with one platform-owned observation: members remain, complete, or unknown.
- [ ] Treat permission errors and incomplete enumeration as unknown. A zero count from an API that also maps errors to zero is not sufficient proof.
- [ ] Use exact group membership enumeration, not process-name discovery. Confirm behavior on the supported macOS versions.
- [ ] Keep leader identity valid through all signaling. In PR 3, use non-reaping child exit observation, then reap once after group cleanup. Do not let a Tokio waiter reap that child first.
- [ ] Keep the weaker current-supervisor and dead-monitor cases conservative. A recorded PGID without a verifiable live identity cannot authorize a later signal.
- [ ] Verify required listener disappearance in addition to group cleanup.

Postgres normally starts children in separate sessions. Its root group is therefore not complete resource containment. Keep SIGINT for normal fast, clean shutdown. Require a real Postgres check with a connected client and a separate forced-postmaster-failure check. If the resource's remaining shutdown cannot be proved, retain records and recovery tools and refuse replacement or destructive removal. Do not add a generic whole-process-tree tracker for this case.

Tests: extend `crates/cli/tests/daemon.rs`, `crates/cli/tests/setup.rs`, and nearby daemon process fixtures. Use native `pv-fake` processes and exact ownership guards. Cover live daemon, absent daemon, missing plist, stale record, foreign/reused PID, stop failure, and uninstall with and without prune. Use the nearby `insta` style for output and state snapshots.

Acceptance: PR 2 works without monitor code and can merge independently. Its shutdown fix is installed or otherwise applied before the production cutover rehearsal.

## 7. PR 3: monitor engine and process contract

Likely files:

- New `crates/daemon/src/monitor.rs`; add small child modules only if actual code size requires them.
- New `crates/daemon/examples/pv-monitor.rs`.
- New `crates/daemon/tests/monitor.rs`.
- `crates/daemon/src/lib.rs`, daemon Cargo example wiring, and the existing CLI command routing.
- `crates/platform/src/process.rs` and its macOS/unsupported implementations for new host-specific primitives.
- `crates/state/src/paths.rs` and `fs.rs` for paths and safe file operations.

Reuse Tokio, serde, serde_json, rustix, and existing state filesystem helpers. Do not add a new crate or dependency for this work without demonstrated need. Use typed domain errors.

### 7.1 Startup and instance ownership

- [ ] Start the installed release executable directly, rather than relying on a symlink that can change during update.
- [ ] Pass command, arguments, private environment, captured log path, and default fallback stop parameters through a private startup channel. Keep secrets out of argv, records, and diagnostic formatting.
- [ ] Validate the request, ownership, socket length, and lifeline before spawning a runtime.
- [ ] Choose the instance token before startup. Acquire the subject reservation before starting the monitor and transfer the locked descriptor without a gap.
- [ ] Place the monitor in a separate session and the runtime in its own process group.
- [ ] Record monitor identity and runtime identity for offline recovery. Use a new instance token for every start. Include a host boot identity or equally strong proof that rejects a previous-boot record.
- [ ] Publish state atomically through the existing filesystem helpers.
- [ ] Report successful startup only after exec is confirmed, recovery records exist, and the control socket is available. Return the runtime PID and instance identity, or a typed startup error. Service readiness remains a separate check.
- [ ] Preserve owned-child cleanup for ordinary startup errors. A caller cancellation after acceptance sends an explicit instance-scoped stop.
- [ ] Give the daemon a coherent initial state even when the runtime exits immediately after spawn.

Use a small hidden exec-gate mode in the same binary to close the spawn/publication gap:

1. Spawn the gate as the future runtime group leader. It waits on a private descriptor.
2. Inspect its birth identity and publish the recovery envelope. The envelope identifies both the gate and intended runtime phases.
3. Send permission to exec only after publication succeeds.
4. The gate replaces itself with the runtime. EOF before permission makes it exit without starting the service.
5. Report exec failure explicitly. Channel closure alone does not prove that the runtime executed; check execution identity or terminal state before reporting startup success.

The gate must not wait inside `Command::pre_exec`. That can prevent the parent's `spawn` from returning before publication. Reuse the existing exec helper in an ordinary hidden entry point. Test that PID and the selected birth identity remain continuous across exec on supported macOS versions.

After the monitor accepts the complete startup request and owns the reservation, daemon death leaves a discoverable instance. Socket EOF is not a cancel request. Before acceptance, incomplete startup input starts no resource. A monitor death before exec permission starts no resource; after permission, recovery records already exist.

### 7.2 State directory and recovery

Use a short deterministic subject hash for `<id>`. Store the full subject and the unique instance token in the record. Verify the full subject to detect collisions.

The controller owns the directory's final removal. The monitor exits after release and never recursively removes a directory that a replacement might reuse.

Use a persistent sibling lock at `~/.pv/run/m/<id>.lock`, outside the reused directory. The controller acquires it before startup and transfers the same locked file description to the monitor. The monitor holds it until process exit. The gate keeps startup protection until exec or exit; the resource must not inherit the reservation descriptor. Never unlink this lock during ordinary replacement. State and stop RPCs do not acquire this lifetime lock.

- [ ] Refuse a second start while the existing instance is live or its cleanup is unresolved.
- [ ] A missing socket or a request timeout does not prove that the monitor is dead.
- [ ] Separate a live unresponsive monitor from a dead monitor using recorded process identity.
- [ ] Use a small recovery operation to stop a verified orphan runtime after monitor death.
- [ ] Refuse signals when identity is missing, changed, from a previous boot, or otherwise unproven.
- [ ] Keep records after an uncertain cleanup outcome. Do not start a replacement over them.
- [ ] Ensure an old cleanup request or old acknowledgment cannot remove a newer instance.

Removal and replacement require proven runtime cleanup, verified monitor death, acquisition of the sibling lock, and a fresh comparison of the captured full subject and instance. Remove only that instance's records. Socket closure alone is insufficient. Uninstall may remove the sibling locks only after all monitors stop and external lifecycle admission remains closed.

Reuse existing OS-lock helpers with the narrow descriptor handoff needed here. Do not introduce a general lease service.

### 7.3 Stop execution and cleanup evidence

The stop request supplies a symbolic stop signal and grace duration. Current policy is SIGINT for Postgres and SIGTERM for other runtimes. Existing call sites have different grace periods; do not replace them with one hard-coded value.

- [ ] The monitor accepts one stop sequence and keeps executing it after client disconnect.
- [ ] Repeat requests for the same instance do not start competing stop sequences or reset the grace deadline.
- [ ] Send the graceful signal, wait, send SIGKILL if required, and perform a bounded final cleanup check.
- [ ] Observe the direct child's exact exit result without reaping while further group signals are possible. After cleanup observation, reap it exactly once.
- [ ] Publish leader exit promptly, even if another group member remains.
- [ ] Treat permission errors as uncertainty, never as authority to signal a numeric PID or group.
- [ ] Return a typed cleanup failure if group disappearance cannot be verified. Keep recovery evidence and prevent replacement.
- [ ] Use platform-owned process-group observation when leader-only checks cannot prove member disappearance. Do not use pipe EOF as containment proof.

For a spontaneous leader exit, the monitor must also address remaining members. Proposed behavior: run the startup-supplied fallback stop plan for residual owned group members, report progress separately, and retain unresolved state on failure. The daemon still owns normal stop policy and sends current parameters for requested stops. Review this fallback carefully; a daemon-absence interval must not leave cleanup policy undefined.

Arbitrary descendants that escape their owned group remain outside the current generic guarantee. Real Postgres tests must verify its resource-specific shutdown behavior. Do not claim that a parent monitor alone contains such descendants.

### 7.4 Exit retention and release

The monitor stores terminal status in memory. Pushed events are advisory; `state` after each connection is authoritative for that instance.

- [ ] Return exit code or signal, requested-stop status, and cleanup state from `state`.
- [ ] Preserve the result when daemon connections fail or disappear.
- [ ] Use one protocol path: stop if needed, read terminal state with final cleanup, explicitly release that instance, verify monitor death, then remove records.
- [ ] The daemon commits the instance-scoped observation before release. If the commit fails, retain the result. Diagnostic logging remains best effort under the recommended refinement in section 2.
- [ ] An explicit CLI maintenance stop consumes the result only after the client receives terminal state and requests release. It must not require a writable or current-schema database. Record an observation when the normal state path permits it; do not add an exit file for this case.
- [ ] Make release instance-specific and repeatable. A stale release cannot affect a different instance.
- [ ] Accept release only after supported cleanup completes. An early request cannot pre-authorize release before the final cleanup result is recorded.
- [ ] A successful stop reply alone never consumes the result. Lost stop replies leave state available. After a lost release reply, the caller can retry for the same instance or verify monitor death. The daemon already has its committed observation; the explicit CLI stop has already received and consumed its result.
- [ ] Keep one immutable terminal result and a small separate cleanup state. Do not add another retry model or an event history.
- [ ] Test lifeline closure completes cleanup and releases the monitor without a daemon acknowledgment. Cleanup failure must remain visible to the test's outer guard.

There is no guaranteed exit-status retention after monitor death or reboot. Do not add a disk journal to broaden that guarantee. Existing v1 status stores the latest observation, not an event history.

### 7.5 Protocol and update behavior

- [ ] Use bounded newline-delimited JSON and the existing local IPC conventions.
- [ ] Check peer UID before processing requests. Apply owner-only directory and socket permissions.
- [ ] Have the controller verify the connected monitor peer against the recorded monitor birth identity and expected instance. A matching UID alone is insufficient.
- [ ] Define `version`, `state`, parameterized `stop`, and instance-scoped release behavior before freezing the core.
- [ ] Define typed errors, retry behavior, required fields, and bounded request handling.
- [ ] Freeze the offline recovery envelope as well as the basic requests.
- [ ] Permit additive optional fields. Do not restart a runtime because the application version changed.
- [ ] Replace a runtime only when a truly incompatible required contract prevents safe operation. Use normal reconciliation and preserve update lock ordering.
- [ ] Keep a frozen stop/release operation or verified offline recovery available for incompatible cases. Reconciliation cannot safely replace a runtime it cannot stop.
- [ ] Do not expose an unrestricted signal RPC if the complete stop operation meets all current callers.
- [ ] Keep Linux and Windows unsupported before side effects. Do not use `cfg(unix)` as a substitute for macOS support.

### 7.6 Captured log rotation

Keep runtime stdout and stderr attached to append-mode files. Use one common monitor rotator for these files. Leave Gateway Caddy access/error files with Caddy. Do not rotate the same file through both owners.

- [ ] Trigger rotation from a fixed-size check. Use a 10 MiB threshold and five retained archives. Select a simple check interval during implementation and document threshold overshoot.
- [ ] Keep the same active inode open for the runtime. Copy, then truncate that inode. Do not rename the active file out from under its writer.
- [ ] Use a unique timestamped archive name that sorts in time order and matches `pv logs` discovery.
- [ ] Keep files `0600` and directories `0700`. Reject unexpected file ownership and unsafe path entries.
- [ ] Finish the copy successfully before truncation. On failure, retain the active file and expose the failure through state or existing diagnostics.
- [ ] Copy only the source length captured at rotation start. Publish a completed archive before truncating the same retained active descriptor. Use a temporary archive name that `pv logs` does not discover.
- [ ] Use no-follow opens and descriptor-based ownership/type checks for the active file and archive. Securing a path after opening it does not itself verify the opened inode.
- [ ] Keep one rotation operation per captured file. Prevent stale monitor work from truncating a replacement's output after instance release.
- [ ] Keep file work outside the control loop. A stalled copy must not prevent stop handling or keep a released monitor process alive indefinitely.
- [ ] Make final process shutdown independent of blocked rotation and lifeline work. Aborting a started `spawn_blocking` task does not stop it, and ordinary Tokio runtime drop can wait forever. Verify actual monitor exit before replacement.
- [ ] Bound copying work against an endlessly growing source. Document any extra loss window from the chosen finite-copy method.
- [ ] Preserve Caddy's existing separate retention behavior. Do not claim that every PV log, including daemon and launchd logs, is covered by per-runtime monitors.

The threshold is not a strict disk quota. Log output can exceed it between checks. Copying needs extra disk space. Failure to copy must not cause active-file truncation. Direct files remove monitor-reader dependence; they do not make a stalled disk harmless to a runtime's own writes.

Test normal archive order, retention, append after truncation, copy failure, unsafe paths, rotation cancellation, and actual process exit during held file work. Also test `pv logs --follow` across truncation. Do not write a flaky test that expects zero lost concurrent bytes.

### 7.7 Test ownership and footprint

- [ ] Pass the existing test-process lifeline read descriptor before monitor spawn. Keep the write end in the test process with close-on-exec behavior.
- [ ] Let the runtime inherit the descriptor where the fake needs it. Do not close it and reuse its number before runtime startup.
- [ ] Keep production independent of this test-only descriptor.
- [ ] Test closure before spawn, during startup, during readiness, and after daemon restart.
- [ ] Prove normal release exits the monitor while the test process and its lifeline remain open. Do not let an uninterruptible lifeline reader keep the monitor alive.
- [ ] Disable the fake's independent lifeline and parent-death safeguard when proving the monitor's own cleanup. Keep normal fixture safeguards enabled in other tests.
- [ ] Use explicit outer fixture cleanup and exact process ownership so a failed monitor test cannot leak a runtime.
- [ ] Test at least one installed real artifact where practical, using temporary data and test home directories.
- [ ] Add one real `pv` subprocess smoke test for hidden-command routing, inherited descriptors, startup, state, stop, release, and process exit. The example alone cannot verify CLI packaging.
- [ ] Measure startup, resident memory, threads, and idle wakeups. Do not infer a small footprint from using a module or crate.

Acceptance: PR 3 exposes a tested monitor engine and hidden entry point. Existing production supervision still works without a feature flag or a second production policy. No speculative backend abstraction is needed.

## 8. PR 4: connect the daemon and perform the cutover

Main code areas:

- `crates/daemon/src/supervisor.rs`, `gateway.rs`, and `managed_resources/mod.rs`.
- `crates/daemon/src/health.rs`, `server.rs`, and startup/reconciliation code.
- `crates/daemon/src/caddy_admin.rs`.
- `crates/state/src/database.rs`, `migrations.rs`, and a new versioned SQL migration.
- Disable/uninstall's shared stop path from PR 2, resource status paths, and affected tests.

Tasks:

- [ ] Replace ordinary runtime spawn/adopt/stop with the monitor client. Cover all Gateway, worker, and Managed Resource callers.
- [ ] Keep short-lived validation on its current owned-child cleanup path.
- [ ] On startup, list monitor directories, authenticate each monitor, and query its current state.
- [ ] Subscribe before readiness waiting, or use a state/subscription handshake that cannot miss a fast exit.
- [ ] Treat an unknown but verified monitor subject through desired-state reconciliation. Do not ignore a runtime started just before daemon death.
- [ ] Use events to request the existing targeted reconciliation. Preserve queue coalescing and backoff counting; replayed state must not cause duplicate attempts.
- [ ] Add instance identity to latest runtime observations. Use conditional writes so delayed state from instance A cannot overwrite running instance B. Do not add a historical event table.
- [ ] Keep the health tick for readiness and drift. A live monitor is not proof of a ready service.
- [ ] Make client cancellation stop a newly created runtime when the current operation owns that startup. Daemon process death instead leaves a discoverable monitor.
- [ ] Make disable and uninstall stop monitors after the daemon cannot start new runtimes. Keep the absent-daemon recovery path.
- [ ] Audit status/doctor/runtime lookup and test fixture code for legacy file assumptions.
- [ ] Enforce the first cutover in the updater: refuse monitor startup over unresolved legacy runtimes. A manual runbook alone is insufficient.
- [ ] Preserve the jobs admission barrier during activation and health checks. The new daemon cannot start replacements inside a rollback window before it acquires jobs ownership.
- [ ] Test automatic activation failure, compatible update, incompatible rollback, and pruning of a still-running monitor's old release path.
- [ ] Keep a capable recovery binary and records if incompatible rollback cleanup fails. A live monitor must be self-contained and must not need to re-exec a pruned release path.

### Configuration proof

Move applied, desired, staged, and replacement-required proof to the state crate and `pv.db`. Separate proof of a live instance from proof of prepared bytes retained after a failed replacement.

- [ ] Bind applied proof to the exact monitor/runtime instance. A replacement must not inherit an old applied fingerprint.
- [ ] Initialize pending proof for the chosen token before startup. Compare that token on every apply, rollback, staged-marker, and replacement-required update.
- [ ] Keep retained staged proof after process death where source routes still require it.
- [ ] Do not cascade deletion of monitor control records into deletion of retained prepared-byte proof. Verify the retained root and fragments before reuse.
- [ ] Preserve atomic transitions before and after `POST /load`.
- [ ] Preserve rejection rollback, unknown-outcome replacement, and post-commit housekeeping behavior.
- [ ] Add the next migration number at implementation time. Do not assume 013 is still free.
- [ ] Do not build a historical runtime-event table.
- [ ] Do not import old pid-file process ownership as a compatibility bridge during cutover.

### Caddy administration

Read fresh instance/PID/group information from the authenticated monitor. Verify current OS identity and continue checking the actual peer of each Caddy admin connection before HTTP bytes are sent. Recheck relevant instance state after configuration operations as required by the current contract.

Monitor UID authentication alone does not authenticate Caddy's separate admin socket. Preserve rejection of a foreign peer, an exited runtime, a changed instance, and an uncertain ownership result.

Acceptance: every runtime uses monitors. The config transaction and backoff tests pass. The PR includes a complete cutover runbook and updates current-behavior documentation necessary for an independently mergeable integration PR. Remove newly unused private code and imports in this PR. Only safely retained legacy structure may wait for PR 5. Do not add dummy callers or lint exceptions to preserve the split. Production must not fall back to old adoption.

## 9. PR 5: remove obsolete adoption code

- [ ] Remove old pid-file adoption and the supervisor's script-identity fallback after checking every caller.
- [ ] Remove imports, paths, fields, and tests made unused by this cutover.
- [ ] Retain exact identity checks needed by monitor-failure recovery and Caddy peer verification.
- [ ] Retain short-lived validation and directly owned child cleanup.
- [ ] Replace obsolete supervision tests with monitor integration coverage. Do not delete a failure case merely because its helper was removed.
- [ ] Update DESIGN.md and CONTRIBUTING.md with final monitor, log, and fixture behavior.
- [ ] Search the full workspace for old ownership assumptions. Leave unrelated pre-existing dead code alone.

Acceptance: no production caller adopts an old pid-file runtime or uses the script fallback. Required offline recovery and admin safety checks remain. Native platform compile checks and the macOS suite pass. PR 4 is safe if this cleanup PR is delayed; do not move a safety fix here merely to keep five PRs.

## 10. Required behavior checks

Prefer process-level integration tests. Copy the style of adjacent fixtures and `insta` snapshots. Use event barriers rather than guessed sleeps.

| Scenario | Required observation | PR |
| --- | --- | --- |
| Disable with an absent daemon or plist | Owned runtimes stop before success. | 2, then 4 |
| Uninstall stop failure | Runtime evidence and required binaries remain; uninstall fails. | 2, then 4 |
| Admission is paused before state removal | Setup, enable, update, writable-state commands, and daemon startup cannot recreate removed state. Cover default uninstall and prune. | 2, then 4 |
| Lifecycle owner starts the daemon | Enable and update health checks complete without nested or lifetime-lock deadlock. | 2, then 4 |
| Foreign or reused PID in a record | No signal to the unverified process. | 2, 3 |
| Runtime cannot spawn | Startup returns a typed error and leaves no active instance. | 3 |
| Runtime exits before readiness | No false readiness result; exact exit is available. | 3, 4 |
| Daemon exits during a stop | Monitor completes the same stop deadline and escalation. | 3, 4 |
| Leader exits but group member remains | Exit is reported promptly; replacement is blocked pending cleanup. | 3 |
| Cleanup is unproven | Error and recovery records remain; no false stopped result. | 3, 4 |
| Real Postgres clean and forced shutdown | Connected clients stop cleanly with SIGINT. Forced leader failure cannot authorize replacement or prune while backend cleanup is uncertain. | 2, 4 |
| Daemon restarts while runtime runs | Runtime PID and service connection remain. | 4 |
| Runtime exits while daemon is absent | Surviving monitor returns code/signal after reconnect. | 3, 4 |
| Observation write fails | Exit is retained; acknowledgment is withheld. | 4 |
| Explicit maintenance stop with unavailable database | Owned runtime cleanup and explicit release work from control/recovery records without a database migration. | 4 |
| Old instance A reports after B starts | Conditional writes preserve B's current status and applied proof. | 4 |
| Acknowledgment or stop reply is lost | Retrying converges without releasing another instance. | 3, 4 |
| Old cleanup races replacement | New records, socket, and log output are not removed or truncated. | 3, 4 |
| Monitor dies after completed startup | Recovery checks exact recorded identity before cleanup. | 3, 4 |
| Monitor dies during startup | Before exec permission, no service starts. After permission, published records identify the instance. Exercise both sides of publication. | 3 |
| Test process gets SIGKILL | Monitor cleanup works without the fake's own safeguard. | 3 |
| Test daemon restarts | Test lifeline stays live and runtime survives. | 3, 4 |
| Copy fails or disk is full | Active log is not truncated; failure is visible. | 3 |
| Rotation worker is held | Stop/control stays responsive; release causes actual monitor exit before replacement. | 3 |
| Test lifeline stays open after release | The completed monitor still exits; no blocked lifetime reader prevents it. | 3 |
| Log follow spans truncation | `pv logs --follow` continues to show new appended output. | 3 |
| Config load is rejected or outcome unknown | Existing rollback/replacement contract still holds. | 4 |
| Failed replacement retains source routes | Staged proof survives runtime/monitor record cleanup. | 4 |
| Foreign Caddy admin peer | Request is rejected before HTTP bytes are sent. | 4 |
| Application version changes with compatible monitor | Runtime is not restarted solely for the app version. | 4 |
| Old release file is pruned while monitor runs | Monitor continues; current binary can recover its records. | 4 |
| Activation fails before jobs admission | Rollback leaves no new runtimes from the failed release. An incompatible rollback keeps recovery tools until verified cleanup. | 4 |
| Real hidden CLI entry | Production routing and inherited descriptors complete startup/state/stop/release and process exit. | 3 |
| Unsupported host command | Typed unsupported error before side effects. | 3, 4, 5 |

During implementation, run focused nextest filters for changed behavior first. Do not use `--lib` for monitor/fake process suites because it can omit required example binaries.

```shell
cargo nextest run --profile ci -E 'test(monitor)'
cargo nextest run --profile ci -E 'binary(daemon) | binary(setup)'
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

The filter names above are proposed entry points. Confirm actual test binary names with nextest when implementation adds the cases. Run affected config/recovery suites, then the required full macOS suite before declaring the production cutover ready. Use existing native CI for Linux and Windows compile validation.

Never rebuild while another nextest run in the same worktree is using those binaries. Scope process checks to the exact fixture or worktree parent chain. Never signal another session's processes. Preserve primary test errors and also report cleanup errors.

Real-artifact checks must use installed binaries with temporary data first. Do not print the private artifact-manifest URL. Do not touch the owner's production database or service state.

## 11. Cutover and rollback runbook

This is a planned procedure. Do not run it during planning or review.

1. Merge and apply the shutdown fix before changing production supervision.
2. Stop the old daemon and old runtimes with the fixed command. Verify the exact owned groups and listeners are gone.
3. Preserve user data and the migration backup. Record the old application release for recovery.
4. Install the monitor-capable build and enable PV. Do not adopt old pid-file runtimes.
5. Confirm one monitor per desired runtime, correct readiness, correct log paths, and no duplicate listener.
6. Restart the daemon. Verify that runtime PIDs and service availability remain stable.
7. Exercise stop/start and application-update behavior on the test installation.
8. Run the selected real Postgres clean-stop check, including a connected client.
9. Use the first machine before applying the same change to the second machine a few days later.

Rollback is not automatic process adoption. Stop monitor-managed runtimes with the monitor-capable binary first. Keep that recovery binary available until rollback verification completes. Restore an older application and database only with a verified compatible schema or the recorded migration backup. Then start old runtimes through the older daemon. Do not leave active monitors while deleting their control records or required recovery tools.

## 12. Review questions that can change the plan

Counselors must answer these questions with source evidence:

1. Is a monitor worth its total cost with Linux work expected within a few months? Would kqueue, direct-file rotation, and the shutdown fix meet the real requirements more simply?
2. Is in-memory exit retention useful enough to justify release/acknowledgment handling? Can existing `state` and `stop` behavior make it simpler without losing the stated guarantee?
3. Does the proposed directory ownership rule work after daemon SIGKILL, monitor SIGKILL, stale replies, concurrent disable, and replacement?
4. What is the smallest correct lock arrangement for lifecycle changes and per-subject record deletion? Does it survive prune without nesting the current locks incorrectly?
5. Does the startup gap need an inherited exec gate? Is there an existing platform primitive or helper that gives a smaller correct solution?
6. Can macOS cleanup verify remaining group members with acceptable complexity? Which guarantees still require real-resource behavior rather than group containment?
7. Does append-file rotation remain the best trade for these development logs? Does the plan add avoidable logging ownership or retention code?
8. Does the config-proof data model preserve staged proof after the process dies while preventing reuse of applied proof across instances?
9. Does Caddy admin verification still reject a different same-user process on the socket? Is a monitor snapshot sufficient for the current checks?
10. Can each PR merge safely on its own? Is the integration PR too large or does the stack contain temporary code that should be removed sooner?
11. Are daemon example tests and the production CLI exercising the same behavior? What minimal smoke test proves the production entry point?
12. Are the update, old-release pruning, and rollback rules complete without unnecessary version-driven runtime restarts?

Classify each finding as a correctness blocker, a decision change, a useful simplification, or an optional improvement. Do not rank a wording issue above a process-control defect.

## 13. Independent review results

The original draft and raw reports are saved under `agents/counselors/1791478121-independent-design-and-implementation-p/`. Read `synthesis.md` there for the source checks, disagreements, and remaining gates.

| Counselor | Actual result | Design recommendation |
| --- | --- | --- |
| claude-fable | Completed | Proceed after corrections; prefer an exit-result file and daemon-owned rotation. |
| codex-sol | Completed | Proceed after corrections; keep all three main choices. |
| codex-astra | Completed | Proceed after corrections; keep all three main choices. |
| claude-opus | Timed out after 15 minutes with no report | No verdict. A focused retry awaits explicit approval to send the plan and source to this external counselor. |

Adopt the confirmed startup, cleanup, instance, update, and testing corrections above. Keep the three original choices. Do not treat reviewer count as evidence of correctness.

Fable's claims that macOS exit-status observation is child-only and that normal Postgres children remain in the postmaster group conflict with current primary source. Do not use those claims to select the architecture. The synthesis links the source and states the supported-version test limits.

The owner approved the three refinements in section 2 and requested implementation. The caller map above selects the smallest lock arrangement found through source tracing. Bootstrap, update health, and deletion admission still require tests. This record does not claim that unimplemented monitor behavior already works.

## 14. Current planning task and completion record

- [x] Read the handoff, repository rules, related process code, and selected prior review reports.
- [x] Confirm that the local `gh stack` extension supports linking existing PRs and atomic stack merge.
- [x] Write this plan with the owner's current choices and explicit unresolved review gates.
- [x] Obtain the required counselors agent-list confirmation.
- [x] Send the complete plan to claude-opus, claude-fable, codex-sol, and codex-astra for independent review.
- [x] Check the three completed reports and inspect source evidence for material findings.
- [x] Record a verdict table and update this plan for confirmed correctness fixes.
- [x] State proposed changes to owner-selected decisions separately from approved choices.
- [ ] Complete the focused Opus retry, if source-sharing approval is given; otherwise record that only three reviews completed.
- [x] Give the owner the revised plan, the three completed review results, the remaining decision list, and the actual PR dependency graph. State the missing Opus result explicitly.

The planning task is complete. Implementation is now authorized under the five-PR sequence. Merge, release, and installed-state changes still require explicit owner instruction. Counselor reports remain untracked under `agents/`. Do not add those reports or `docs/superpowers/plans/2026-10-01-pv-fake-step-4.md` to a commit.
