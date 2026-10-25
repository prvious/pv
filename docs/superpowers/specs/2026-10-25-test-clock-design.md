# Test Clock Design

Status: accepted 2026-10-25; the owner decided D1–D3. PR 1 adds the clock and revises this spec with the #460 review points: named limits, retried failures, and failing a blocked test.

## Summary

Some tests check what PV does when something it waits on freezes: a config validator that never exits, a login shell that never finishes, a daemon that won't stop. Today those tests wait out PV's real limit (10 s, 10 s, 10 s, 5 s, 3 s), or they pass a shortened budget through a `*_for_test` entry point.

This design adds one small PV clock that every limit and schedule goes through:
- **In production** it is tokio's and the standard library's time, with no new dependency.
- **In tests** it can be swapped for a fake that moves only when the test says so. A test that needs a limit to expire moves the clock past it once PV is waiting on it, so the test takes about as long as its real work, whatever the limit is.

Failures that aren't freezes don't need the clock: pv-fake fails at once instead.

## Motivation

- **The waits are wasted time.** The tests that wait out a production limit spend about 65 s summed per CI run (minimum across runners, 2026-10-21 and 2026-10-22 recordings). Inventory lists them.
- **Shortening limits doesn't scale.**
  - Every new limit brings its own wait.
  - Short budgets are the tests that flaked on slow runners: the 2026-10-15 runtime-reload spec traced several `gateway_reconciliation` failures to 150–250 ms budgets.
  - A shortened limit also can't control *when* it expires. The frozen-validator test has to wait for the validator's children to exist before the limit fires, or it proves less.
- **Tokio's paused clock can't do this.**
  - It auto-advances whenever the runtime has no work (tokio 1.52, `tokio/src/time/clock.rs`). Waiting on a real process over a socket counts as "no work", so real readiness checks time out at once. Only a running `spawn_blocking` task holds it still.
  - It applies to one runtime only. PV builds private runtimes in the CLI (`cli/src/commands/mod.rs:400`) and the blocking daemon client (`daemon/src/client.rs:62`), and some limits are plain synchronous code with no runtime. Together these cover 38 of the 61 s in the inventory.
  - The existing workaround in `daemon_foundation` and `project_env_reconciliation` (pause, advance 30 s, resume, then `sleep` 1.5 s of real time, `daemon_foundation.rs:1117`) shows the limit.
- **PV mixes clocks.** No fake can cover all of them as things stand:
  - `gateway.rs` computes readiness deadlines with `std::time::Instant` and sleeps with `tokio::time::sleep`.
  - `runtime_stop.rs` uses `std::time::Instant` and `std::thread::sleep`.
  - `health.rs`, `server.rs`, `jobs.rs` and `client.rs` use `tokio::time`.

## Goals

- A test never waits out, and never shortens, a limit PV owns. It moves PV's clock past it.
- A test never makes something freeze when a crash or an error would test the same thing.
- Every PV limit and schedule reads one clock, so a new limit is fakeable without extra test plumbing.
- Production behaviour, limits and dependencies are unchanged.

## Non-Goals

- **Processes other than the test process.** pv-monitor and pv-fake keep their own real clocks. A test passes their timings as data (for example the monitor's `grace_ms`) and picks the smallest value that proves its point.
- **Wall-clock timestamps** (`OffsetDateTime::now_utc` in `pv.db` and logs). Tests already redact them.
- **Waits on the operating system.**
  - The watcher's quiet periods wait for FSEvents.
  - The test-side `sleep`s wait for jobs or pv-fake events.
  - These need event waits, not a clock.
- **Injecting a clock into every component.** See D1.

## Two kinds of failure

1. **A crash or an error PV gives up on at once.** The process exits, or a request fails in a way PV doesn't retry.
   - The test makes pv-fake fail immediately: `validate_exit_code`, `rustfs_reject_credentials`, or a Gateway control response.
   - No limit is involved and the clock isn't used.
2. **A freeze, or a failure PV retries until a limit.** Only a limit expiring tells PV to stop.
   - **A freeze:** the process stays alive and silent. The test holds pv-fake with `validate_pause`/`run_pause` (`Pause { until }`), `TcpHolder`, or a fixture daemon that never exits.
   - **A retried failure:** each attempt fails at once, but PV keeps retrying until its budget runs out. Examples:
     - `GatewayListeners::Nothing` and `AdminOnly` make every readiness probe fail immediately;
     - admin readiness and supervisor readiness retry those probes until the budget expires.
   - Either way, once PV is waiting, the test moves PV's clock past the limit. Replacing a retried failure with a crash would test a different path.

## Design

### Production: tokio and std, unchanged

A new `platform::clock` module exposes a few functions plus a real poll tick.

- **Every limit is a named `Limit`,** for example `Limit::new("config validation", Duration::from_secs(10))`.
- **The name is how a test picks the limit to advance past.** Durations alone can't: the client's write and response limits are both 3 s, and a daemon always has health timers pending.

| Function | In production |
|---|---|
| `now() -> Instant` | `std::time::Instant::now()` |
| `sleep(&limit)` | `tokio::time::sleep(limit.duration())` (from PR 3, for schedules) |
| `timeout(&limit, future)` | `tokio::time::timeout(limit.duration(), future)` |
| `Deadline::after(&limit)` with `is_expired()` | `now()` arithmetic, for polling loops such as `DaemonProcess::wait_for_exit` |
| `poll(duration)` / `poll_blocking(duration)` | `tokio::time::sleep` / `std::thread::sleep`, always real. Only for short ticks (≤ 100 ms) between checks of a real process. `poll_blocking` arrives in PR 2. |

- **What goes through the clock:**
  - Every limit PV gives up on: config validation, hook environment capture, daemon exit, helper socket, client response, readiness budgets, cleanup grace.
  - Every schedule: the health interval and its retries, download retry backoff, debounce windows.
- **What stays real:** the poll ticks between "is it up yet?" checks, through `poll`.

Without the `test-support` feature the fake isn't compiled. Each function is the tokio or std call itself, with no extra branch and no runtime cost.

### Tests: an opt-in fake

The fake lives behind a new `test-support` feature on `platform`, which the dev-dependencies of `daemon`, `cli` and `platform`'s own tests enable. `state` already has a `test-support` feature used the same way.

- **Opt-in.** `platform::clock::install_fake_for_test()` installs the fake for the rest of the test process and returns a `FakeClock`. Tests that don't call it run on real time, as today.
- **Process-wide.** It covers every runtime and thread in the test process, including the CLI's and the blocking client's private runtimes and synchronous code. This is safe because nextest runs each test in its own process, which CONTRIBUTING.md already requires and the Gateway-probe switch already relies on.
- **Time moves only when the test advances it.**
  - Virtual time starts when the fake is installed.
  - `sleep`, `timeout` and `Deadline` register a pending limit with the fake. An advance fires every limit it passes; each sees the advanced `now()`.
  - Nothing advances on its own.
- **Advance when waiting.** Advancing before PV starts waiting would leave the limit unfired and the test hung. So the only way to advance is to wait until PV is waiting on that limit, which is what Kubernetes' fake clock provides with `HasWaiters`:
  - **`FakeClock::advance_when_waiting(&limit)`.** Waits until PV has a pending limit with **that name**. Other pending limits don't count, even with the same duration. It then moves time to the moment that limit passes, which also fires anything else due by then.
  - **`advance_when_waiting_in_background(&limit)`** (PR 2). Does the same from a helper thread, for synchronous code that blocks the test's own thread.
  - **If the named limit doesn't appear within 30 s of real time,** the call fails with the list of pending limits. A mistake becomes a clear failure instead of a hang.
- **A blocked test still fails.**
  - An async test isn't blocked while it awaits `advance_when_waiting`, so the error returns to it.
  - A synchronous test's own thread is stuck inside PV, so an error returned on the helper thread would never reach it. Instead, when the guard expires, the background helper writes the pending limits to stderr and exits the test process with a failure status.
  - nextest runs each test in its own process, so that fails exactly this test, at once, with the diagnostics in its output.
  - The helper is the one place tests start a thread directly; it carries a narrow `#[expect(clippy::disallowed_methods)]`.

```rust
// Freeze: a validator that never exits (PR 1).
let clock = platform::clock::install_fake_for_test();
let (result, advanced) = tokio::join!(
    validate_config(&command, &config_path, &environment),
    async {
        // the hung validator's child process must exist before the limit fires
        wait_for_existing_path_count(slice::from_ref(&validator_child_pid), 1).await?;
        clock.advance_when_waiting(&CONFIG_VALIDATION_TIMEOUT).await?;
        Ok::<_, Error>(())
    },
);
advanced?;
assert!(matches!(result, Err(DaemonError::ProtocolTimedOut { .. })));
```

```rust
// Synchronous: `pv daemon:disable` runs in-process and blocks the test thread (PR 2).
let clock = platform::clock::install_fake_for_test();
clock.advance_when_waiting_in_background(&DAEMON_EXIT_GRACE);
let output = run_pv(&["daemon:disable"], &environment)?;
```

The fake holds a mutex over the virtual time and the pending limits with their wakers. It gets its own tests, including one where an unrelated limit with the same duration is already pending.

### Guardrails

1. **Opt-in.** Only tests that install the fake get fake time.
2. **No production branching.** Production code never asks whether time is fake; there is no `is_fake()`.
3. **Poll ticks are explicit.** Real sleeps go through `poll`/`poll_blocking` and stay at 100 ms or less, so every real wait is deliberate.
4. **No early advances.** Every advance goes through advance-when-waiting, with a real-time guard that fails loudly.
5. **Test-only build.** The fake exists only with `test-support`; release builds can't contain it.
6. **One clock.** From PR 4, a Clippy rule bans direct timers outside `platform::clock` in every crate (D2):
   - `std::thread::sleep`
   - `std::time::Instant::now`
   - `tokio::time::{sleep, sleep_until, timeout, interval}`
   - `tokio::time::Instant::now`

## Risks

- **A hidden dependency.** Code doesn't show that it depends on time. It's acceptable because every wait goes through the clock (guardrail 6), so there's nothing special to spot.
- **One clock per process.** Plain `cargo test` runs tests as threads in one process, where they would share the fake. nextest is required.
- **The ordering trap.** An advance before PV waits would hang. Guardrail 4 removes the API that allows it.
- **Everything freezes.** With the fake installed, health scans, debounce windows and every other PV timer stop too. A test that quietly depends on one of them can't finish. The real-time guard turns that into a failure listing the pending limits.
- **A convention in production code.** PV calls `platform::clock::sleep` instead of `tokio::time::sleep`. It costs nothing at runtime, but contributors and agents must follow it. Guardrail 6 enforces it.
- **We own the fake.** About 150 lines of waking and ordering logic. It stays small, gets its own tests, and fires the limits one advance passes in deadline order.
- **A hung happy path** fails at nextest's 120 s termination instead of at PV's limit.

## Alternatives considered

- **`tick` (Microsoft Oxidizer).** Rejected (D3):
  - Its production clock runs its own background timer task instead of tokio's timer.
  - Its first release was December 2025, with eight breaking releases since.
  - `Clock::new_tokio` panics outside a tokio runtime, and some PV limits run with none.
- **Tokio's paused clock.** It auto-advances past real waits and reaches only the test's own runtime (see Motivation).
- **Shorter or zero limits passed in by tests.** These can't control when a limit expires, and every new limit needs its own plumbing. The owner rejected this on 2026-10-25.
- **A clock passed into each component**, Kubernetes style ([k8s.io/utils/clock](https://pkg.go.dev/k8s.io/utils/clock)). It's cleaner, but touches most signatures because the daemon has no context object. Revisit if the daemon gets one (D1).
- **Go's `testing/synctest`** ([docs](https://pkg.go.dev/testing/synctest)). The best semantics: time advances only when everything is blocked on something inside the test, never while waiting on real I/O. Rust has no equivalent short of writing our own async runtime.
- **Deterministic simulation** (turmoil, madsim). Everything must run inside the simulator, and PV runs real Caddy, PHP and Postgres binaries.

## Inventory

Waits on a production limit (CI minimum across runners):

| Test | Limit | Today |
|---|---|---|
| `frankenphp_config_validation_timeout_stops_validator_process_group` | `CONFIG_VALIDATION_TIMEOUT` 10 s, `daemon/src/gateway.rs:59` | 10.0 s |
| `hook_capture_times_out_shell_startup` | `CAPTURE_TIMEOUT` 10 s, `platform/src/hooks/unix.rs:22` | 10.1 s |
| `daemon_replacement_refuses_to_bootstrap_while_previous_daemon_is_alive` | `wait_for_exit(10 s)`, `cli/src/commands/daemon.rs:142` → `daemon/src/runtime_stop.rs:22` (twice per test) | 20.3 s |
| `daemon_disable_waits_for_process_death_after_its_socket_is_gone` | same | 10.4 s |
| `helper_bootstrap_wait_stops_when_the_socket_never_appears` | `HELPER_IO_TIMEOUT` 5 s, `platform/src/helper.rs:70` | 5.0 s |
| `blocking_client_times_out_when_daemon_withholds_response` | `DAEMON_RESPONSE_TIMEOUT` 3 s, `daemon/src/client.rs:15` | 3.0 s |
| 3 `supervisor_foundation` tests | 1 s probe timeouts, `daemon/src/supervisor.rs:24`, `gateway.rs:71` | ~1.0 s each |

Shortened budgets:
- About 16 `gateway_reconciliation` tests pass 100 ms–2 s readiness budgets to `*_for_test` entry points so readiness fails: 100 ms ×1, 150 ms ×3, 250 ms ×4, 1 s ×5, 2 s ×3.
- With the clock, they use the production budget and advance past it, and the budget parameter can go.

Pause, advance, resume, then sleep:
- 3 `daemon_foundation` health tests and 2 `project_env_reconciliation` tests.
- They become advances.

## PR plan

A stack, each PR verified with targeted runs, the full suite, and a comparison of CI recordings.

1. **The clock.**
   - Add `platform::clock`: `Limit`, `now`, `timeout`, `Deadline` and `poll`, plus the fake behind `test-support`, with its own tests.
   - Convert config validation and hook environment capture, including their cleanup limits, and their two tests. Each now finishes well under 1 s.
2. **Synchronous limits and private runtimes.**
   - Add `poll_blocking` and `advance_when_waiting_in_background`.
   - Convert `DaemonProcess::wait_for_exit`, the helper socket wait and the blocking client's response limit.
   - Name the CLI's inline 10 s grace as a constant.
   - Convert the CLI, platform and client tests.
3. **Readiness budgets and schedules.**
   - Add `sleep`.
   - Route Gateway, worker and supervisor readiness, health and retry schedules through the clock.
   - Replace the short budgets in `gateway_reconciliation` and the three 1 s `supervisor_foundation` tests with advances.
   - Turn the pause/advance/resume tests into advances.
   - If the pf removal lands first, rebase onto it, since that work deletes part of the readiness code.
4. **The rest and the lint.**
   - Convert the remaining timing constants (72 named, about 100 `timeout` calls).
   - Turn on the Clippy rule.
   - Document the two failure kinds and the clock in CONTRIBUTING.md.

## Decisions

- **D1 — process-wide fake, not an injected clock. Decided 2026-10-25.** A clock passed into each component stays the cleaner option if the daemon gets a context object; it isn't worth adding one for this.
- **D2 — the Clippy rule turns on at the end of the stack. Decided 2026-10-25.** PR 4 enables it for all crates, once every limit and schedule goes through the clock.
- **D3 — no new dependency. Decided 2026-10-25.** Production keeps tokio and std; PV owns the small fake.
