# Test Clock Design

Status: proposed 2026-10-25. D1–D3 below are open.

## Summary

Some tests check what PV does when something it waits on freezes: a config validator that never exits, a login shell that never finishes, a daemon that won't stop. Today those tests wait out PV's real limit (10 s, 10 s, 10 s, 5 s, 3 s), or they pass a shortened budget through a `*_for_test` entry point. This design gives PV one clock that tests can control. Every limit and schedule PV owns goes through it. A test that needs a limit to expire moves the clock forward instead of waiting, so the test takes about as long as its real work, whatever the limit is.

Failures that aren't freezes don't need the clock: pv-fake fails at once instead.

## Motivation

- **The waits are wasted time.** The tests that wait out a production limit spend about 65 s summed per CI run (minimum across runners, 2026-10-21 and 2026-10-22 recordings). The table under Inventory lists them.
- **Shortening limits doesn't scale.** Every new limit brings its own wait, and short test budgets are the flaky ones. The 2026-10-15 runtime-reload spec traced several `gateway_reconciliation` failures on slow runners to 150–250 ms budgets.
- **Tokio's paused clock can't do this alone.**
  - Tokio 1.52 auto-advances a paused clock whenever the runtime has no work (`tokio/src/time/clock.rs`). Only a running `spawn_blocking` task holds it still.
  - The daemon's tests talk to real processes over sockets, such as a fake Caddy. Waiting on a socket counts as "no work", so a paused clock jumps past every real wait and real readiness checks time out at once.
  - That is why `daemon_foundation` and `project_env_reconciliation` pause, advance 30 s, resume, then `sleep` 1.5 s of real time (`daemon_foundation.rs:1117`).
- **PV mixes clocks.** No fake can cover all of them as things stand:
  - `gateway.rs` computes readiness deadlines with `std::time::Instant` but sleeps with `tokio::time::sleep`.
  - `runtime_stop.rs` uses `std::time::Instant` and `std::thread::sleep`.
  - `health.rs`, `server.rs`, `jobs.rs` and `client.rs` use `tokio::time`.

## Goals

- A test never waits out, and never shortens, a limit PV owns. It advances PV's clock past it.
- A test never makes something freeze when a crash or an error would test the same thing.
- PV's limits and schedules all read one clock, so a new limit is fakeable without extra test plumbing.
- Production behaviour and limits are unchanged.

## Non-Goals

- **Processes other than the test process.** pv-monitor and pv-fake keep their own real clocks. A test passes their timings as data (for example the monitor's `grace_ms`) and picks the smallest value that proves its point.
- **Wall-clock timestamps** (`OffsetDateTime::now_utc` in `pv.db` and logs). Tests already redact them.
- **Waits on the operating system.** These need event waits, not a clock:
  - The watcher's quiet periods wait for FSEvents.
  - The test-side `sleep`s wait for jobs or pv-fake events.

## Two kinds of failure

1. **A crash or an error.** PV notices at once: the process exits, a request fails, or a listener refuses. The test makes pv-fake fail immediately, for example with:
   - `validate_exit_code`
   - `GatewayListeners`
   - `rustfs_reject_credentials`
   - a Gateway control response

   No limit is involved and the clock isn't used.
2. **A freeze.** The process stays alive and silent, and only a limit expiring tells PV something is wrong. The test holds pv-fake, using `validate_pause`/`run_pause` (`Pause { until }`), `TcpHolder`, or a fixture daemon that never exits. It then advances PV's clock past the limit.

## Design

### Clock

- Use the [`tick`](https://lib.rs/crates/tick) crate (Microsoft Oxidizer project, MIT, 0.8.0).
  - `tick::Clock` provides `delay`, a `timeout` through `tick::FutureExt`, and `instant`/`stopwatch` for reading time.
  - Its `test-util` `ClockControl` starts with auto-advance off. `advance(duration)` fires every timer that expires within the advanced period.
  - This is the behaviour tokio's paused clock lacks.
- `platform::clock` exposes the process clock. Without a test, it's tick's real clock, backed by tokio's timer where a runtime exists.
- A test installs a fake clock and gets its `ClockControl` back. The fake stays installed for the rest of the test process.
  - This mirrors `route_public_probes_to_gateway_for_test` and Laravel's facades.
  - It's safe because nextest runs each test in its own process, which CONTRIBUTING.md already requires.
- Faking is opt-in. Tests that don't install the fake run on real time, as today.

### What goes through the clock

- **Through the clock:**
  - limits PV gives up on: config validation, hook environment capture, daemon exit, helper socket, client response, readiness budgets, cleanup grace;
  - schedules: the health interval and its retries, download retry backoff, debounce windows.
- **Stays real:** short poll ticks between checks of a real process, such as 25 ms between readiness probes.
  - They go through a named helper, `platform::clock::poll(duration)`, so every real sleep is deliberate.
  - Under a fake clock, a limit never expires unless the test advances it, while polling keeps watching the real process at real speed.

### Tests

```rust
let time = platform::clock::install_fake_for_test();
let validation = tokio::spawn(validate_config(...));
time.wait_for_pending_timer().await; // PV is now waiting on its limit
time.advance(CONFIG_VALIDATION_TIMEOUT);
assert!(matches!(validation.await?, Err(DaemonError::ProtocolTimedOut { .. })));
```

- **Advance only after PV starts waiting.** The test must advance after PV has registered its limit. If tick has no "wait until a timer is pending" call, PV's wrapper counts pending timers and provides it. PR 1 verifies this.
- **Synchronous waits.** Some limits are checked by synchronous code running on the test's own thread. For example, the CLI runs in-process, and `pv daemon:disable` calls `DaemonProcess::wait_for_exit`. For those, a helper thread advances the clock once the limit is registered.
- **A hung happy path** fails at nextest's 120 s termination instead of at PV's limit.

### Lint

Once the conversion is complete, the root `clippy.toml` disallows these outside `platform::clock`:
- `std::thread::sleep`
- `std::time::Instant::now`
- `tokio::time::{sleep, sleep_until, timeout, interval}`
- `tokio::time::Instant::now`

A new limit then can't skip the fake. (D2)

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
- They become plain advances.

## PR plan

A stack, each PR verified with targeted runs, the full suite, and a comparison of CI recordings.

1. **The clock.** Add `tick` and `platform::clock` (real, fake, `poll`). Convert the async limits and their tests: config validation, hook capture, client response, supervisor probes. Each converted test should finish under 1 s.
2. **Synchronous limits.** Convert `DaemonProcess::wait_for_exit` and the helper socket wait, plus the CLI and platform tests.
3. **Readiness budgets and schedules.**
   - Route Gateway and worker readiness, health and retry schedules through the clock.
   - Replace the short budgets in `gateway_reconciliation` with advances.
   - Turn the pause/advance/resume tests into advances.
   - If the pf removal lands first, rebase this PR onto it, since that work deletes part of the readiness code.
4. **The rest and the lint.** Convert the remaining timing constants (72 named, about 100 `timeout` calls), turn on the Clippy rule, and document the two failure kinds in CONTRIBUTING.md.

## Risks

- **`tick` is pre-1.0.** Its API may change, so pin it the way CI pins nextest.
- **Code with no runtime.** tick's real `Clock` uses tokio's timer, so synchronous code with no runtime (the CLI's waits) has to read time through tick in a way that works without one. PR 2 checks this. If it fails, the synchronous path keeps `std::time` behind the same `platform::clock` API.
- **Plain `cargo test`.** A global fake is shared between tests that run as threads in one process. nextest is already required.
- **Hangs fail later.** A happy-path test that hangs fails at 120 s instead of earlier.

## Decisions

- **D1 — global clock or passed-in clock.**
  - A global `platform::clock` changes few signatures and works like the existing test switches.
  - Passing a `Clock` through the daemon and CLI contexts is more explicit, but touches most call sites.
  - Recommended: global.
- **D2 — when the Clippy rule turns on.**
  - At the end of the stack (PR 4), for all crates.
  - Or earlier, for the daemon only.
  - Recommended: at the end.
- **D3 — `tick` or our own fake.** About 150 lines of our own would avoid a pre-1.0 dependency. AGENTS.md prefers established libraries. Recommended: `tick`.
