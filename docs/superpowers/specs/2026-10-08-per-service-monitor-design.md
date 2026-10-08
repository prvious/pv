# Per-Service Monitor Design

Status: proposed 2026-10-08, not implemented. Three decisions are still open, each marked **Decision needed**. Until this is accepted and built, DESIGN.md's Daemon Lifecycle section describes how PV supervises runtimes. Once it is built, DESIGN.md's supervision paragraphs (pid files, adoption, and runtime logs) are rewritten to match.

## Why

The daemon is the parent of the runtimes it starts only until it restarts. After a restart or self-update, it finds them again through pid files and runtime metadata, and checks their identity (start time, executable, and arguments) before it signals them. That works, but:

- An adopted runtime isn't the new daemon's child, so a crash shows up only at the next health tick, up to 30 seconds later, and its exit status is lost.
- Identity checking is the most bug-prone part of supervision: reused PIDs, records that change during cleanup, and macOS answering a process-group signal with `EPERM` while the process is still exiting (issue #394).
- Runtimes write their output straight to their log files, so nothing can rotate those files without the runtime's help. Only Caddy's own access and error logs roll today, despite the size-based rotation that DESIGN.md's Filesystem Layout section requires.
- Process containment and identity checks exist only for macOS. Supporting Linux would mean porting them.

## Shape

PV runs one monitor process for each supervised runtime: the Gateway, each Project-serving worker, and each Managed Resource track. The monitor is the `pv` binary in a hidden mode. It starts its runtime as its own child, stays its parent for the runtime's whole life, and talks to the daemon over a Unix socket. The daemon talks to monitors and never signals a runtime PID itself. The DNS resolver runs inside the daemon and short-lived processes such as config validation are not runtimes, so neither gets a monitor.

The division of work follows containerd's shims:

- **The monitor does the mechanics; the daemon decides policy.** The monitor can start its runtime, send a signal to the runtime or to its whole process group, report state, and report the exit. Stop signals (for example SIGINT for Postgres), grace periods, escalation to SIGKILL, readiness checks, and restart backoff stay in the daemon. A monitor keeps running the code it started with until its runtime restarts, while the daemon is replaced on every self-update, so keeping policy in the daemon means an update changes stop behavior at once.
- **The monitor never waits on the daemon.** It keeps reading its runtime's output and keeps its runtime running whether or not the daemon is connected, so a daemon crash or update never stalls a runtime.
- **The monitor stays small.** Monitor mode loads no `pv.db`, manifests, or reconciliation state, so each monitor costs a few MB, not a daemon's worth.

## Lifecycle

1. **Start.** The daemon runs `pv monitor start` with the runtime's command, arguments, private environment, log path, and stop policy. The command starts the long-lived monitor in a new session, so it survives the daemon's LaunchAgent restarts as runtimes do today. It returns only once the monitor's socket is listening, and prints `{version, address}` or an error, so the daemon gets a clear success or failure without polling for a socket.
2. **Run.** The monitor starts the runtime in its own process group, with the runtime's stdout and stderr connected to the monitor through pipes.
3. **Exit.** As the parent, the monitor learns the runtime's exact exit status the moment it exits. It reports the runtime stopped only once the runtime's whole process group is gone. Because the monitor reaps the runtime itself, a group that still answers `EPERM` can only mean another member is still exiting, which removes the ambiguity behind #394. The monitor pushes an `exited` event to a connected daemon and keeps the exit status until the daemon acknowledges it, then removes its state directory and exits. A crash during a daemon restart or update is therefore never lost.
4. **Stop.** The daemon asks the monitor to send the runtime's stop signal to the process group, waits up to the grace period for the `exited` event, asks for SIGKILL to the group if needed, and then acknowledges the exit.

## Finding monitors

Each monitor has a state directory, `~/.pv/run/m/<id>/`, which holds its socket and a small state file: the runtime's PID and start identity, the protocol version, and which runtime it serves. `<id>` is a short, fixed-length hash of the runtime's identity, as worker admin socket names are, so socket paths stay under macOS's limit of about 104 bytes even under long test home directories. The daemon finds monitors only by listing these directories, never by scanning processes.

When the daemon starts, including after a self-update or a crash, it connects to each monitor, asks for its `version`, and then asks for its `state`. Pushed events only make things faster: events sent while the daemon is down are lost, so asking for state on every connection is what makes recovery reliable.

## Protocol

Newline-delimited JSON over the monitor's socket, like the daemon socket. The socket lives in the owner-only `~/.pv/run/`, and the monitor checks the connecting peer's UID before answering.

**A frozen core never changes.** From the first release on, these two requests, together with the socket location and the message framing, stay exactly as they are, so any future daemon can stop any older monitor:

```
→ {"op":"version"}
← {"protocol":1,"pv":"0.3.0","runtime":"postgres:18"}
→ {"op":"stop"}
← {"stopped":true,"exit_code":0}
```

`stop` stops the runtime with the stop policy the monitor was started with (its stop signal, a 10-second grace period, then SIGKILL), and then the monitor exits. It is an emergency exit. Normal stops use the versioned requests, so the daemon's current policy applies.

**Everything else is versioned.** The daemon uses the other requests only when the monitor reports the daemon's own protocol version: `state`, `signal` (a signal sent to the runtime or to its process group), the `exited` event, acknowledging an exit, and log settings. These can change freely between versions.

**A version mismatch restarts that runtime once.** If a monitor reports a different protocol version, the daemon sends the frozen `stop`, then starts a new monitor and runtime with its own version. Self-updates that don't change the protocol keep runtimes running, as today.

**An unreachable monitor is cleaned up from its state file.** If a state directory exists but its socket doesn't answer because the monitor crashed, the daemon runs `pv monitor delete <id>`. It reads the state file, checks the runtime's identity (PID plus start identity), stops the runtime's process group, and removes the directory. On macOS this is the only place identity checks remain. On Linux it kills the runtime's cgroup instead.

## Logs

The monitor writes its runtime's output to the runtime's log file and rotates it by size, as DESIGN.md's Filesystem Layout section requires, using the same limits as Caddy's logs: 10 MiB per file and 5 rotated files, named as `pv logs` already expects. Rotation is safe because the monitor holds the file, not the runtime. If writing fails, for example because the disk is full, the monitor drops output instead of blocking its runtime, and counts what it dropped.

## When the daemon is gone

**Decision needed: how long a runtime outlives its daemon.** Each monitor has an owner that stays connected to it: the daemon in production, and the test process in tests. The monitor notices when that connection closes, and can stop its runtime once the owner has been gone for a set time. A daemon that restarts reconnects within seconds, which resets the clock.

- **Recommended: in production, never.** Runtimes keep running until they are stopped or the Mac restarts, as today. Issue #349 asks not to change runtime lifetime to fix test teardown.
- **Alternative: after a long wait, such as several hours.** Runtimes left behind by a broken uninstall or a manually unloaded LaunchAgent would stop eventually, but a daemon crash loop longer than the wait would also take down databases.

Either way, tests start monitors with no wait: when the test process that owns a monitor dies, its connection closes and the monitor stops its runtime immediately. That replaces `pv-fake`'s test-only lifeline with the production code path, and it works for real artifacts too.

## Commands that stop every runtime

`pv daemon:disable` and `pv uninstall` stop every runtime by asking each monitor found under `~/.pv/run/m/` to stop it. This also closes a gap: DESIGN.md's Daemon Lifecycle section says `pv daemon:disable` stops PV-managed processes first, but today it only unloads the LaunchAgent, without asking the daemon to stop any runtime.

## What it replaces

Once monitors run every runtime, PV no longer writes pid files or runtime metadata, adopts runtimes by checking their identity, or keeps the supervisor's script-identity fallback. `pv monitor delete` keeps the only identity check on macOS. The 30-second health tick stays for readiness probes, while crash detection becomes immediate through `exited` events.

## Costs

- One extra process per runtime.
- The frozen core has to be right the first time.
- A monitor runs the code it started with until its runtime restarts. After two self-updates its release may have been pruned from `~/.pv/bin/releases/`. That is harmless because a monitor never re-runs its own binary, and cleanup uses the current `pv monitor delete`.
- If a monitor crashes, its runtime loses its parent. macOS has no signal that kills a process when its parent dies, so `pv monitor delete` is the recovery path.
- Daemon tests need a monitor binary to start.

**Decision needed: where the monitor code lives.** Recommended: a module in the daemon crate, entered through a hidden `pv monitor` subcommand in production and a daemon example binary in tests, the same way tests build `pv-fake`. Tests then need no separately built `pv` binary.

**Decision needed: the state directory name.** Recommended: `~/.pv/run/m/<id>/`, with `<id>` as described in Finding monitors. The short name keeps socket paths within the macOS limit.

## Cutover

There is no backward compatibility (decided 2026-09-29). The first release with monitors does not adopt runtimes started from pid files. Stop the old runtimes once by hand on each machine before updating, and the new daemon starts every runtime under a monitor. Roll it out on one machine first and on the second a few days later.

## Linux

On Linux, the monitor marks itself a child subreaper, so descendants that leave the runtime's process group, such as Postgres's backends, are re-parented to the monitor rather than to init. It also places the runtime in its own cgroup when one is available, so stopping or deleting a runtime kills the cgroup with no identity checks. The details are left for Linux support.
