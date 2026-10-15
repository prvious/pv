# Runtime Reload Simplification Design

Status: proposed 2026-10-15. The owner decided D1–D5 on 2026-10-15.

## Summary

PV reloads the Caddy Gateway and FrankenPHP workers through a transaction that tries to undo a config Caddy has already accepted when a readiness check fails afterwards. Real Caddy can't get into that state, and the tests reach it only through a pv-fake mode that real Caddy doesn't have. This design removes that rollback. Once Caddy accepts a config, PV keeps it.

The design also makes four related changes:
- It splits the 60-second readiness budget into a start budget and a short after-reload budget.
- It stops capping admin requests with the readiness budget.
- It checks runtime ownership once per change instead of before every admin request.
- It gives workers a Caddy-level health route, like the Gateway's.

## Motivation

- **Flaky tests.** Several `gateway_reconciliation` tests fail on slow CI runners:
  - CI run 38589045125, plus 2026-09-22, 2026-10-01 and two runs on main on 2026-10-14.
  - A test passes a 150–250 ms readiness budget so a readiness check fails fast. The same budget also caps the admin `POST /load`, which has to succeed (`CaddyAdminClient::new().with_timeout(readiness.timeout)`, `crates/daemon/src/gateway.rs`).
  - Running the 18 tests that pass short budgets under `taskpolicy -b` with `--stress-count 40 -j 32`, 3 of 40 rounds failed, with 6 test failures in all. Seven of the eleven affected tests check the post-load rollback.
- **The rollback has no real trigger.**
  - Caddy 2.11.4 binds a new config's ports before it answers `/load`, closes the old ports first, and rolls back its own failed starts (`caddy.go` `changeConfig`/`unsyncedDecodeAndRun`; recorded in #379).
  - The rollback tests get there with `retain_previous_listeners`. That control was added in #379 only so tests written for the old Python fixture keep passing.
- **Most of the machinery answers review comments, not incidents.**
  - The restore load, the tampered-backup check, the "outcome unknown" fencing and the peer process-group check came from pullfrog and Codex comments on #322 and #341–#347.
  - The tampered-backup check itself caused two follow-up bugs: `.DS_Store` files, and an intact backup being refused.
  - Only the pf exception (`PreserveRuntime`) comes from a real outage (#304, #305).
- **The 60-second budget hurts users.**
  - It went from 15 s to 60 s for slow privileged CI runners (#286). It was restored after a Codex comment on #344.
  - The daemon runs one job at a time while holding `jobs.lock`. A readiness check that never passes delays every queued job for a minute, and `pv php:install`, `pv composer` and `pv update` fail at once with "lock held".
  - DESIGN.md still says 15 s.
- **The real gap is elsewhere.**
  - A worker's readiness is a TCP connect, so it only proves the port is open.
  - A FrankenPHP reload restarts PHP. If that restart fails, Caddy keeps answering while PHP requests fail.

This replaces the earlier test-only plan to raise the short budgets to 1 s.

## Goals

- Once Caddy accepts a config, PV keeps it. The only rollback is the one Caddy does itself.
- A readiness failure after an accepted reload replaces the runtime on the next pass, the same as an unknown `/load` outcome. Exception: the Gateway while pf is `drifted` or `unknown`.
- Starting a runtime gets 60 s; a reload gets 5 s.
- Admin requests keep their own timeouts, whatever readiness budget a test passes.
- Ownership is checked when a change starts and before it is recorded as applied, not before every admin request.
- Every worker answers a PV-owned health route served by Caddy. Every worker readiness probe uses it.
- Delete the tests and pv-fake controls that only exist for the removed behavior.

## Non-Goals

- Changing the pf readiness policy. `PreserveRuntime` for the Gateway in `drifted`/`unknown` stays as it is. It came from a real outage (#304, #305), and it keeps pf trouble alone from stopping or restarting the Gateway.
- Changing fresh starts. A new process that never becomes ready is still stopped, and the previous files are restored.
- Changing rejected loads. Caddy keeps the old config, and PV restores the previous files and records the previous fingerprint.
- Detecting PHP that is stopped behind a running Caddy. That needs a route that runs PHP; the owner chose a Caddy-level route (D3).
- The ownership checks inside readiness polling (`wait_for_owned_readiness`). They let PV stop waiting as soon as a runtime exits.
- Managed Resource timeouts, which stay at 15 s.
- The 200-with-error `/load` parsing (`LoadReportedFailure`). Caddy 2.11.4 needs it (caddy#7246, fixed in 2.11.6). Remove it once both Caddy and FrankenPHP ship the fix.

## Decisions

| | Decision | Outcome |
|---|---|---|
| D1 | Remove the rollback after Caddy accepts a config | Yes (owner, 2026-10-15) |
| D2 | Readiness budgets | 60 s to start, 5 s after a reload (owner, 2026-10-15) |
| D3 | Worker health check | A Caddy `respond` route; no PHP file (owner, 2026-10-15) |
| D4 | Ownership checks | Once per change, not per request (owner, 2026-10-15) |
| D5 | Readiness fails after an accepted reload | Restart the runtime on the next pass (owner, 2026-10-15) |

## Design

### Accepted configs are kept

When a matching runtime accepts `POST /load` and its readiness check then fails:
- PV keeps the accepted files on disk.
- PV leaves the replacement marker and staged fingerprint set, and records the readiness failure. This is today's `PreservePending` disposition.
- The next reconciliation stops the marked runtime and starts a fresh one from the accepted files. It never sends a second `/load` to that process.

An unknown `/load` outcome already works this way, so the two cases share one rule.

The Gateway while pf is `drifted` or `unknown` is the exception. It keeps today's `PreserveRuntime` path: once admin readiness passes, PV commits the config as applied and reports routing as degraded, without restarting.

Removed:
- `RuntimeRecovery::RollbackAndRestore` and `pending_requiring_restore`.
- `restore_runtime_after_failed_load` and `restore_runtime_after_load`.
- The `restoration_readiness` plumbing in `prepare_promoted_runtime`, `PendingPromotedRuntime`, `CompletedPromotedRuntime` and `PromotedRuntimeRecovery`.
- `previous_runtime_readiness`, `previous_runtime_readiness_from_parts`, `previous_readiness_check` and `fragment_port`.
- `verified_previous_config_fingerprint`. The rejected-load arm (`RollbackPending`) records the previous applied fingerprint directly. Caddy's memory still holds it, and anything that later reads the files re-hashes them.
- `ProcessSupervisor::mark_restoration_required` and the `restoring_previous` parameter of `mark_runtime_config_pending`.
- `PromotedConfigTree::previous_tree_contents` and `previous_fragment_contents`, and `PromotedConfigDir::previous_contents`.

Kept:
- `replacement_required`.
- The applied, staged and desired fingerprints.
- `record_restored_config`, `clear_replacement_required` and `preserve_staged_config`.

The staged fingerprint still matters for worker moves. A Project moving from worker 8.4 to 8.5 keeps its fragment on 8.4 until the Gateway commits. If 8.4's reload has an unknown outcome, the staged proof lets the retry find that fragment again (`verified_recorded_runtime_config`). Without it, the move never finishes.

`CaddyAdminError::RestoredConfigReloadFailed` stays, because disk-rollback failures still produce it (`runtime_config_rollback_failed_error`, `gateway_config.rs` `rollback_failed_error`). It is renamed `RollbackFailed`, since PV no longer reloads a restored config.

### Readiness budgets

- `RuntimeReadinessPlan` carries a start budget and a reload budget, because it is built before PV knows whether it will start or reload.
- **Start budget: 60 s.** Used for fresh-start admin readiness (`GET /config/`) and fresh-start service readiness. A first launch of a newly installed binary can be slow while macOS scans it.
- **Reload budget: 5 s.** Used for service readiness after `/load` on a matching runtime. A running Caddy answers within milliseconds; FrankenPHP finishes restarting PHP before `/load` returns.
- Unchanged:
  - The `PreserveRuntime` cap of 2 s (`PF_PUBLIC_READINESS_TIMEOUT`).
  - The fast-path probe and the targeted skip probe, at `min(budget, 1 s)`.
  - The foreign-listener probe, at 100 ms.
- The `#[doc(hidden)]` test entry points keep one `readiness_timeout` argument, which overrides both budgets.

### Admin requests keep their own timeouts

- Admin clients no longer call `.with_timeout(readiness.timeout)`.
- `POST /load` uses the client defaults: connect 2 s, write 5 s, read 10 s, 15 s overall.
- Readiness polling still bounds each `GET /config/` by the time left in its readiness budget (`wait_until_ready_with`).
- A test can therefore shorten readiness without cutting `/load` short. That removes the flake class above.
- Tests that need an unknown `/load` outcome trigger it with `load_response_body: ["not json"]`. A 200 whose body PV can't parse is `RequestOutcomeUnknown` (`caddy_admin.rs` `successful_load_response_error`), so the result doesn't depend on timing.

### Ownership once per change

- PV checks ownership when a change starts (`begin_runtime_transaction`). It checks again when it records the config as applied, because `record_applied_config` refuses a runtime that is no longer owned.
- Removed:
  - The per-request verifier: `CaddyAdminVerifier`, `runtime_ownership_verifier`, `verify_before_request` and the `load_caddyfile_with`/`wait_until_ready_with` verifier arguments.
  - The Unix-socket peer process-group check (`verify_peer_process_group`).
- Both checks date from when the admin API used TCP ports. The admin socket now lives in the owner-only `~/.pv/run/`, and PV deletes and recreates it on every start. Under load, the per-request check also used most of a short budget: 116 ms of 150 ms in one stress failure.

### Worker health route

Every worker root config gets a PV-owned site, rendered like the Gateway's (`gateway_config.rs` `render_gateway_config`):

```caddyfile
http://pv-worker.localhost:<port> {
    bind 127.0.0.1
    respond /__pv/health "pv-worker-health-v1:<port>" 200
}
```

- A readiness check sends `GET /__pv/health` with `Host: pv-worker.localhost` and expects that exact body. It is plain HTTP only, and shares the body comparison with `GatewayIdentity`.
- It replaces `Tcp` everywhere PV checks a worker, which must all move together:
  - fresh-start and after-reload readiness, and the fast-path probe, which all use the worker's readiness plan;
  - the health tick's worker probe (`health.rs`).
  If one of them still used `Tcp`, recovery would see a "ready" worker and stop retrying. (The targeted skip probe checks the Gateway, not workers.)
- The foreign-listener probe keeps `Tcp`, because a foreign process wouldn't return the body.
- Changing the root changes every worker's fingerprint, so each running worker reloads once after the upgrade.
- What it proves: Caddy on the worker's port is serving PV's config for that port. What it doesn't prove: that PHP is running (see Non-Goals).

## Implementation Sequence

Four commits on one branch and PR, in this order (the owner chose one PR over a stack).

1. **Separate admin timeouts and split readiness budgets.**
   - Remove `.with_timeout(readiness.timeout)` from admin clients.
   - Add the 60 s start and 5 s reload budgets.
   - Switch the three unknown-outcome tests (`gateway_runtime_move_retains_source_until_gateway_commit` ×2, `gateway_reconciliation_replaces_runtime_before_newer_desired_state`) to `load_response_body`.
   - Delete pv-fake's `late_accept`, `late_apply_delay_ms`, `apply_after` and their contract test (`late_accepted_load_applies_after_its_client_disconnects`).
   - Update DESIGN.md line 263 with the Gateway/worker exception.
2. **Keep accepted configs.**
   - Remove the rollback machinery listed above, and route post-load readiness failures to `PreservePending`.
   - Delete tests:
     - `gateway_reconciliation_reports_compound_restore_failure`
     - `gateway_reconciliation_refuses_to_load_tampered_rollback_fragment_backup`
     - `gateway_reconciliation_reports_compound_restored_readiness_failure`
     - `previous_runtime_readiness_keeps_desired_admin_socket` (`gateway.rs`)
     - `previous_fragment_snapshot_ignores_non_caddy_files` (`gateway_config.rs`)
   - Rewrite tests so they expect the kept config (same PID, one load, desired root on disk, marker set):
     - `worker_reconciliation_restores_previous_service_port_after_readiness_failure`
     - `gateway_reconciliation_does_not_reload_after_runtime_exits_after_load`
     - `gateway_reconciliation_rolls_back_config_when_runtime_readiness_fails`
   - Rewrite tests so they expect a plain error or no fingerprint check:
     - `targeted_reconciliation_reports_alive_unready_worker` (plain readiness error)
     - `preserved_worker_sites_use_current_binds_and_rollback_exact_bytes` (no fingerprint check)
     - `promotion_reports_restore_failure_when_fragment_promotion_rollback_fails` (renamed error)
     - `transaction_errors_preserve_ownership_and_both_rollback_failures` (renamed error)
   - Remove the pv-fake `admin_statuses` control.
   - Where the rewritten tests only need "accepted, but the new port never opens", replace `retain_previous_listeners` with `apply_load: false`, then delete `retain_previous_listeners` once nothing uses it.
   - Rewrite DESIGN.md paragraphs 395 (rollback wording), 401 (backup verification), 405 and 824. Update the "Gateway test controls" table in the pv-fake spec.
3. **Ownership once per change.**
   - Remove the per-request verifier and the peer process-group check.
   - Delete `verifier_failure_sends_no_http_request_bytes`, `readiness_verifier_failure_is_not_masked_by_poll_timeout` and `mismatched_peer_process_group_receives_no_http_request_bytes` (`crates/daemon/tests/caddy_admin.rs`).
   - Rewrite DESIGN.md line 389.
4. **Worker health route.**
   - First record real FrankenPHP 1.12.4's answer, as the pv-fake spec requires.
   - Add the root site and the readiness check, and switch all four worker probes.
   - Teach the fake FrankenPHP persona to open the root site's port and answer its `respond` line.
   - Update DESIGN.md's readiness list (line 261) and reload contract.

## Verification

For every commit:
- `cargo nextest run --workspace` passes.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo fmt --all` are clean.

For the branch:
- `taskpolicy -b` with `--stress-count 40 -j 32` on the short-budget tests that remain. On main, 3 of 40 rounds failed on the original 18. On the branch, the original flakes are gone: 2 runs of 40 rounds failed 6 and 1 times, all in `gateway_reconciliation_rolls_back_after_fresh_service_readiness_failure`, whose 500 ms budget also had to cover a new process's admin readiness. That test now gets 2 s.
- The whole `gateway_reconciliation` binary under the same throttling is far beyond what CI sees. 10 rounds had 219 test failures on main and 67 on the branch, mostly 1-second admin waits and timing assertions that fail on main too.
- The Real Artifact E2E workflow (`real-artifact-e2e.yml`) runs against real Caddy and FrankenPHP.
- The worker contract in `gateway_runtime_contracts.rs` checks the worker health route against both the fake and the real binaries. Real FrankenPHP 1.12.4's answer was recorded first (pv-fake spec, Recorded Behavior).

## Risks And Mitigations

- **Worker moves.** Removing restore code could break the staged proof. `gateway_runtime_move_retains_source_until_gateway_commit` must keep passing; only its triggers change.
- **Restart churn.** A readiness failure after a reload that keeps happening now restarts that runtime on each pass. The existing 1 s, 5 s, 15 s recovery backoff bounds it. The Gateway with broken pf is excluded.
- **A 5 s reload budget turns out short.** `/load` returns only after FrankenPHP has restarted PHP, so readiness afterwards is one round trip. Real Artifact E2E covers it.
- **One reload per worker after the upgrade.** Accepted; FrankenPHP finishes in-flight requests first.

## Acceptance Criteria

- No admin request is limited by a readiness budget.
- No code path sends a previous config back to Caddy after Caddy accepted a new one.
- pv-fake no longer has `late_accept`, `late_apply_delay_ms` or `admin_statuses`, and `retain_previous_listeners` is gone or has a documented use.
- No throttled stress failure on the remaining short-budget tests comes from a `/load` or a restore sharing a readiness budget.
- Worker readiness (start, reload and the fast path) and the health tick use the worker health route; the foreign-listener probe keeps a TCP check.
- DESIGN.md describes the new contract, and nothing in it still mentions restored loads, backup verification or per-request peer checks.

## Rejected Alternatives

- **Raise the short test budgets to 1 s.** It cut the throttled failures from 6 to 1, but it kept testing a rollback that real Caddy never needs, and it added about 11 s of summed test time.
- **Commit an accepted config as applied when readiness fails** (D5), as `PreserveRuntime` does. Caddy ignores an identical `/load`, so a stuck runtime would stay stuck until something else changed. A Project could also be routed to a worker that never became ready.
- **A worker health route that runs PHP** (D3). It would also catch PHP stopping behind a running Caddy. But it needs a PV-owned PHP file, and anything in the user's editable `php.ini` can break it.

## Out Of Scope, Noticed

- DESIGN.md line 477 says explicit install and update commands wait for reconciliation. `pv php:install` and the resource install commands submit the job and return without waiting.
