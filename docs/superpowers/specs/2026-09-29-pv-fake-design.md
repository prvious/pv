# pv-fake Test Service Design

Status: approved 2026-09-29. Steps 1 to 4 are implemented.

## Summary

PV will replace the daemon's shell and Python test fixtures with `pv-fake`: one native Rust executable that emulates each Managed Resource and Gateway runtime pv supervises. Tests install it at each real executable path, e.g. `bin/caddy` or `bin/mysqld`, as a copy. The fake reads its persona from a scenario file next to it, runs as a single process, and exits by itself when the test process that installed it dies.

Protocol layers come from established libraries wherever pv talks a real protocol. Each persona's answers are recorded from the real artifact first, and one contract suite runs against both the fake and, in the real-artifact lane, the real binary. That keeps the fakes from drifting.

This change is test-only. It does not change production daemon or supervisor behavior.

## Motivation

The chaos baseline (2026-09-29) measured the current fixtures:

- This laptop had 393 orphaned fixture processes plus 122 Python children left over from test runs between Sep 12 and Sep 26, using about 900 MB of memory and half a CPU core. All of them ran code from before the parent watchers (commit `91de9819`, merged in #356).
- Current code still leaks:
  - A normal full run of the daemon suite left 4 survivors.
  - Hard kills at 10 s, 25 s and 40 s left 6, 2 and 0.
  - The leaks come from fixtures outside #356's watcher convention: the `sleep "$1"` worker runtime (`crates/daemon/src/health.rs`) and the inline `FAKE_CADDY_SCRIPT` (`crates/daemon/tests/daemon_foundation.rs`). The `FAKE_CADDY_SCRIPT` leak is permanent.
  - The cooperative approach works where it was applied, but nothing enforces it.
- The full daemon suite is flaky under load. Each run has 1–3 failures in different tests, mostly "no PV-owned process could be verified", and every one passes in isolation. All current fakes are `#!` scripts, so they take the supervisor's script-identity path with its 250 ms stabilization; real binaries don't.
- Gateway fakes run as `sh → python` process pairs coordinated by `ps` polling, which is fragile under load. The Redis fake finds commands by substring rather than parsing RESP, so a TCP read split mid-command would break it.

## Goals

- Replace every long-running daemon test fixture with a `pv-fake` persona.
- Run each fake as one process that exits when its installing test process dies, including on SIGKILL.
- Take the direct executable-identity path that real binaries take, not the script fallback.
- Use a maintained library for every protocol and framing layer. Hand-written code only decides which answer to send.
- Derive each persona's behavior from recordings of the real artifact, and keep it honest with a contract suite that also runs against the real binary.
- Make fake failures legible: every fake writes an event log that tests can print on failure.
- Remove the `python3` test prerequisite once all fixtures are ported and the supervisor's script-identity tests are removed with the fallback they cover.

## Non-Goals

- Do not change production daemon, supervisor, or Gateway behavior. Removing the supervisor's script-identity fallback is a separate, later change.
- Do not emulate behavior pv never exercises. A persona covers the interactions pv performs plus the failure scenarios tests need.
- Do not replace the real-artifact lane. It becomes the second target of the contract suite.
- Do not add Linux support, the per-runtime monitor, or a simulated process backend.

## Architecture

### Personas and installation

`pv-fake` reads its persona from its scenario file, not from its executable name. Tests sometimes need one runtime's behavior under another's name, e.g. the health tests install a generic process as `bin/frankenphp`. Planned personas:

- `caddy`, `frankenphp`, `mysqld`, `postgres`, `initdb`, `redis-server`, `mailpit`, `rustfs`,
- the test-only variants tests use today, such as the positional fake Mailpit,
- `long_running`: a generic process that stays alive until SIGTERM or SIGINT. It replaces the ad-hoc `sleep` runtime and the inline fake SQL script, which is installed as `bin/pv-fake-sql`.

Tests install a fake with `pv_fake::install(executable, persona)`. It:

- sets up this test process's lifeline pipe (below) on first use,
- copies the `pv-fake` binary to `executable`, which on APFS is a clone that costs no disk space, and
- writes the scenario file next to it.

`pv_fake::install_with_settings(executable, persona, settings)` does the same with behavior a test chooses on top of the persona. The default `FakeSettings` is the recorded behavior:

- `gateway_listeners`: `All`, `AdminOnly` (configs apply, but no HTTP or HTTPS port opens), or `Nothing` (alive, serving nothing), for tests of Gateways that never become ready.
- `validate_pause` holds a Gateway persona's `validate` until a file exists, and `validate_exit_code` makes it exit with that code instead of checking the config.
- `run_pause` holds a Gateway persona's `run` after its HTTP and HTTPS ports open and before its admin socket does.
- `exit_after_first_http_response` makes a `pv_fake_mailpit` persona exit 0 once it has answered an HTTP request, for tests of runtimes that exit right after becoming ready.
- `rustfs_reject_credentials` makes a `rustfs` persona expect a different secret key than PV passed, so every signed request fails with `SignatureDoesNotMatch`, as a real key mismatch does.
- `descendant` starts one child process in the fake's process group, as runtimes start workers. The fake starts itself again with a descendant flag and the read end of a pipe whose write end only the parent holds. The descendant inherits the lifeline, does nothing else, and exits when its parent does. On a clean exit the parent closes the pipe and reaps the descendant first, so no zombie is left behind where nothing reaps orphans, as in some Linux containers.

A paused fake records `held` and still exits cleanly on SIGTERM or SIGINT: signals are watched through startup, not only once it serves.

The fake is a copy, not a symlink, because PV's artifact validation (`RuntimeArtifactAdapter::validate_installation`, via `symlink_metadata`) rejects symlinked executables. That's a production policy the fakes must not work around.

It isn't a hard link either. Hard links share one inode across every install, and Gatekeeper (`syspolicyd`) scans each new path on its first launch and records the result on the file. With hard links, fakes launched under parallel test load died with SIGKILL before running any code, in about half of parallel `pv-fake` suite runs. Copies stopped it. The mechanism is inferred from `syspolicyd`'s logs; the fix was measured.

A copy is a regular file, and the process executable is the path the fake was started from. macOS reports the path passed to exec, not a resolved one (verified with a `KERN_PROCARGS2` probe), so the supervisor's `executable_matches` succeeds directly.

Control files that tests change while a fake is running keep their current names and meaning: `<config>.readiness-gate`, `<config>.readiness-fail`, `fake-admin-control.json` and the marker files. The Gateway personas parse `fake-admin-control.json` into typed settings and answer with a 500 naming the problem when it has an unknown key or is malformed. `pv_fake::write_gateway_control` validates the settings when a test writes them.

### Binary location

Decided: the example target below.

`pv-fake` is a workspace crate containing a library (all logic) and a binary (used by its own contract tests through `CARGO_BIN_EXE_pv-fake`). The daemon lists it under `[dev-dependencies]`, so it never enters production builds, and adds a three-line example target, `crates/daemon/examples/pv-fake.rs`, that calls into the library.

Cargo builds a package's example targets whenever it builds the package's tests. It does so for `cargo nextest run -p daemon` with a test filter and for `cargo nextest run --workspace`. It does not for `cargo test --lib` (all three verified with a probe workspace). So the fake is rebuilt with the daemon's tests and can't silently go stale.

`pv_fake::binary()` locates it at `target/<profile>/examples/pv-fake`, relative to the running test executable, and returns a clear error when it's missing.

Cost: editing `pv-fake` rebuilds the daemon's test binaries, measured at about 5 seconds of incremental compile time.

Alternative considered: a standalone binary at `target/<profile>/pv-fake`. Cargo builds it for neither `-p daemon` nor `--workspace` (a package's binary is only built alongside its own integration tests), so local runs would silently use a stale or missing fake.

### Scenario file

Each installed fake has a scenario file next to it, `<executable>.pv-fake.json`, following today's `$0.server.py` sibling convention. The fake finds it from its absolute `argv[0]`. The install helper always writes one, so no environment variables are involved. A fake with no scenario file exits with an error naming the missing path.

It contains:

- the persona,
- the lifeline descriptor number,
- later, process-level settings: startup delay, readiness never or after N ms, crash after N ms with an exit code or signal, SIGTERM handling (exit, ignore, or delay), and spawning a descendant in the same or its own process group,
- later, persona settings where tests need them.

Settings are added only when a ported test needs them. Step 1 needs none beyond the persona and the lifeline.

### Event log

Each fake appends JSON lines to `<executable>.pv-fake.events.jsonl`. Events: started (persona, argv, whether the lifeline is armed), signal received, lifeline fired, held (the file a pause waits for), descendant spawned (its pid), parent exited (recorded by a descendant), and exit. Every event carries a timestamp, pid, process group, and parent pid. Tests wait on these events rather than on marker files: one atomic line replaces several files written one after another.

Tests read the log with `InstalledFake::events()`. Ported tests can add it to their failure context, so a failure shows whether pv or the fake misbehaved.

### Lifeline

On the first `pv_fake::install` call, each test process:

1. creates a pipe,
2. marks the write end close-on-exec right away. macOS has no `pipe2`, so this can't be atomic, and a process spawned in that instant would hold the pipe open,
3. keeps both ends in a process-lifetime static. The read end stays inheritable, and
4. records the read end's descriptor number in every scenario file it writes.

The number is whatever the pipe was given, never a fixed one, so nothing already open can be overwritten.

The fake `stat`s `/dev/fd/<n>` and checks for a FIFO. It then reads through that path on a blocking task, since opening `/dev/fd/<n>` duplicates the descriptor, so the fake never takes ownership of a raw descriptor number. The read returns end-of-file only when every write end is closed, which happens when the test process exits for any reason, SIGKILL included. It also returns at once if the test process died before the fake started. The fake then:

- signals its own process group with SIGKILL if it is the group leader, as supervised runtimes are. This mirrors the production validation anchor (`CONFIG_VALIDATION_GROUP_ANCHOR` in `crates/daemon/src/gateway.rs`).
- otherwise exits by itself, so a fake started directly by a contract test never signals the test's group.

Descendants are also `pv-fake` processes and inherit the descriptor, so a descendant in its own process group exits by itself too. If the descriptor isn't an open FIFO, for example when a fake is run by hand, no watcher starts.

This replaces every `ps`-polling parent watcher and the parent-capture test hooks.

### Process structure and signals

Each fake is one process with threads. Signal handling comes from the scenario file. Each persona's default SIGTERM behavior and exit status come from the recorded real artifact.

## Protocol Implementations

Rule: a library owns every protocol and framing layer. Persona code only decides what to answer.

| Persona | pv's interactions | Implementation |
|---|---|---|
| `caddy`, `frankenphp` | `validate`; `run --config … --adapter caddyfile`; HTTP and HTTPS using the configured PKI root certificate; admin Unix socket `GET /config/` and `POST /load` (`text/caddyfile`); `GET /__pv/health` over HTTP and TLS; the admin peer must be in the managed process group | `hyper`, `tokio-rustls` (workspace) |
| `postgres`, `initdb` | `initdb` with PV's arguments; `sqlx` startup with `sslmode=disable` and SCRAM-SHA-256 sign-in; `SELECT 1`, `SELECT 1 FROM pg_database WHERE datname = $1` and `CREATE DATABASE` as extended-protocol statements | `pgwire` 0.41 with only `server-api-ring` |
| `mysqld` | `--initialize-insecure`; a TCP port; signals | `tokio` only: no MySQL protocol. The daemon tests keep `RecordingMysqlAdmin`; PV's `sqlx` client meets real MySQL in the real-artifact matrix. `opensrv-mysql` was dropped (see Library compatibility); never hand-write the MySQL protocol |
| `redis-server` | `redis` crate multiplexed connection: `CLIENT SETINFO`, `PING` | `redis-protocol` for RESP framing, plus a command table |
| `rustfs` | `GET /health`; `aws-sdk-s3` `create_bucket` with path-style addressing and SigV4 using the configured credentials; `object_store` put and head of a probe; reject mode | `s3s` 0.17.0, the S3 layer the real RustFS is built on, over a small in-memory bucket store; `hyper` for `/health` |
| `mailpit`, `pv-fake-mailpit` | SMTP greeting; HTTP readiness (`/`, `/ready`) | `tokio`, `hyper` |

These crates are dependencies of `pv-fake` only. Versions of shared crates come from the workspace.

## Contract Fidelity

1. **Record before implementing.** Before each persona is written, run the real artifact (installed from the artifact manifest, as `real_artifact_resource_matrix.rs` does) through exactly the interactions pv performs. Save what comes back as `insta` snapshots: status codes, relevant headers, body shapes (e.g. Caddy's `/load` error `{"error": …}`), CLI exit codes and stderr, startup parameters and greetings, and exit status after SIGTERM.
2. **One contract suite, two targets.** Persona contracts live in the daemon crate, e.g. `crates/daemon/tests/gateway_runtime_contracts.rs`. Managed Resource contracts live in `crates/daemon/src/managed_resources/runtime_contracts.rs`, because the runtime adapters are private to the crate. That way they drive each binary with PV's own renderers, `ProcessSupervisor`, admin client, and readiness checks, so the contract is exactly what PV depends on. Each runs against the fake by default. An ignored twin runs it against the real artifact, installed from the manifest, when `PV_E2E_REAL_ARTIFACTS=1` and `PV_E2E_ARTIFACT_MANIFEST_URL` are set. `.github/workflows/real-artifact-e2e.yml` runs the real twins, so an artifact update that changes behavior PV relies on fails there. Test configs for real Caddy add `skip_install_trust` so a fresh test CA never triggers a trust-store prompt.
3. **Failure scenarios are fake-only.** Crash on start, never ready, ignore SIGTERM, slow shutdown, and escaping descendants have no real-binary equivalent. Their contract tests run against the fake alone.
4. **Plumbing contracts.** Contract tests also cover:
   - process identity at the install path
   - the lifeline killing a group-leading fake, and only the fake itself otherwise, when the write end closes
   - a non-pipe descriptor leaving the lifeline unarmed
   - a missing scenario failing with an actionable error

   A daemon test starts a fake through the real supervisor. It checks that the lifeline arrives armed, that ownership verifies, and that no script executable identity is recorded.

The matching cases in `crates/daemon/tests/fixture_contracts.rs` move here as their fixtures are deleted.

## Recorded Behavior

### Caddy 2.11.4 (2026-09-30)

These were recorded with PV's Gateway config shape and a real test CA.

| Interaction | Real Caddy | `caddy` persona |
|---|---|---|
| `validate`, valid config | exit 0; JSON logs on stderr | exit 0 if the file exists; appends its path to `fake-validator-spawns.log` |
| `validate`, missing file | exit 1; `Error: reading config from file: open <path>: no such file or directory` | same message and exit code |
| Root CA lifetime | refuses a root that expires before its 7-day intermediate | n/a |
| Admin socket | Unix socket, mode `0600` | same |
| `GET /config/` | `200 application/json`, full config | `200 application/json`, `{}` (PV checks only the status) |
| `/__pv/health` over HTTP | `200 text/plain` for `Host: pv-gateway.localhost`; `308` to HTTPS for other hosts | `200` for any host, body from the config's `respond` line; `404` without one |
| HTTPS | leaf per SNI (empty subject, critical SAN) signed by Caddy's own intermediate under PV's root | leaf per SNI signed directly by PV's root |
| `POST /load`, valid | `200`, JSON warnings array: PV indents with spaces, so its Caddyfile is never `caddy fmt` formatted | same warnings array |
| `POST /load`, invalid | `400 application/json` `{"error":"adapting config using caddyfile adapter: …"}` | `400` with the same shape for a config it can't read, e.g. a bad port; it doesn't emulate the Caddyfile grammar |
| `POST /load` with new HTTP and HTTPS ports | by the time `200` returns, the old ports refuse connections and the new ports serve the new identity; same process | same |
| `POST /load` onto a busy port | `200` with the warnings array followed by `{"error":"loading config: loading new config: http app module: start: listening on 127.0.0.1:<port>: listen tcp 127.0.0.1:<port>: bind: address already in use"}`; the previous config and ports keep serving | same |
| Rejected `POST /load` | the previous config keeps serving | same |
| Worker reload with an unchanged root config | imports are read on every load, so a fragment whose site moved to a new port moves the listener: the old port closes, the new one serves | same |
| `admin off` | HTTP and HTTPS serve; no admin socket or admin TCP listener | same |
| SIGTERM | graceful shutdown in milliseconds, exit 0 | same |

The reload rows were recorded 2026-09-30 by loading PV-shaped configs through the admin socket, and the dual-target contracts check them against real Caddy and FrankenPHP.

The persona issues leaves from the configured certificate, which must be a CA. Every Gateway test seeds one with `platform::generate_local_ca`, once per test home.

#### Gateway test controls

`fake-admin-control.json`, next to the runtime config, injects the failures tests need. With no controls, the persona behaves as recorded. A list is used up one entry per request, and a single value applies to every request:

- `GET /config/`: `admin_statuses`, and `admin_response_gate`, which holds the response until a path exists.
- `POST /load`:
  - `load_statuses`: anything but 2xx rejects without applying.
  - `apply_load`: `false` accepts without applying.
  - `retain_previous_listeners`: applies the config but keeps the old ports, so changed ports never open.
  - `load_response_gate`: holds the load, after recording it, until a path exists.
  - `load_delay_ms`, plus `late_accept` with `late_apply_delay_ms`, which applies the load that long after it arrived. Loads finish in their own task, so a late accept still applies after PV hangs up.
  - `load_response_body`, `load_accepted_marker`, and `exit_after_load`.
- `stop_service`: closes the HTTP listener on its next connection and keeps the process running.

Settings that only matter for an accepted load (`apply_load`, `retain_previous_listeners`, `load_accepted_marker`, `exit_after_load`) are used up only by accepted loads. A load the persona can't read is rejected with `400`, whatever `load_statuses` says.

The persona records every request in `fake-admin-requests.jsonl` before holding it, and every load body in `fake-admin-load-NNN.bin`, numbered from 0 in each process. It writes the served config to `fake-admin-current.bin` at startup and after each applied load.

### Redis 8.8.0 (2026-10-01)

These were recorded with PV's rendered `redis.conf` and the `redis` crate's handshake.

| Interaction | Real Redis | `redis-server` persona |
|---|---|---|
| `redis` 1.2.2 handshake | pipelined `CLIENT SETINFO LIB-NAME` and `LIB-VER` (the client ignores both replies), then `PING`: `+OK`, `+OK`, `+PONG` | same |
| `PING <message>` | the message as a bulk string | same |
| `PING` with more arguments, `CLIENT` alone | `-ERR wrong number of arguments for '<command>' command` | same |
| `CLIENT <unknown>` | `-ERR unknown subcommand '<subcommand>'. Try CLIENT HELP.` | same |
| Unknown command | `-ERR unknown command '<command>', with args beginning with: ` followed by `'<argument>' ` for each argument | same |
| Lowercase commands | accepted | same |
| `QUIT` | `+OK`, then closes the connection | same |
| `dir` doesn't exist | exit 1; stderr names the config line: `*** FATAL CONFIG FILE ERROR (Redis 8.8.0) ***`, `>>> 'dir "<path>"'`, `No such file or directory` | same message and exit code |
| Data directory | creates `appendonlydir` | writes nothing |
| SIGTERM, SIGINT | exit 0 in about 0.1 s | exit 0 |

The persona reads only `port` and `dir` from the config and binds `127.0.0.1`. It answers RESP arrays only; Redis's inline commands, which PV never sends, close the connection.

### Mailpit 1.30.1 (2026-10-01)

These were recorded with PV's command line: `--smtp 127.0.0.1:<port> --listen 127.0.0.1:<port> --database <data dir>/mailpit.db --disable-version-check`.

| Interaction | Real Mailpit | `mailpit` persona |
|---|---|---|
| SMTP connect | `220 <hostname> Mailpit ESMTP Service ready`, then waits for commands | same, with `localhost` as the hostname |
| `GET /` | `200 text/html; charset=utf-8`, the dashboard page | `200 text/html; charset=utf-8`, a stub page (PV checks only the status) |
| `GET /readyz`, `GET /livez` | `200`, empty | same |
| Any other path, including `/ready` | `404 text/plain; charset=utf-8`, `404 page not found` | same |
| Without `--disable-version-check` | starts and serves the same; only `/api/v1/info` differs | accepted and ignored |
| Unknown flag | exit 1; `Error: unknown flag: --<flag>` and the usage text | exit 1; the `Error:` line |
| Database folder missing | exit 1; `level=error msg="[db] open <path>: no such file or directory"` | same message without the timestamp |
| SMTP or dashboard port busy | exit 1; `level=error msg="listen tcp <address>: bind: address already in use"` | same message without the timestamp |
| Data directory | creates `mailpit.db`, `mailpit.db-shm` and `mailpit.db-wal` | writes nothing |
| SIGTERM, SIGINT | exit 0 in about 0.35 s | exit 0 |

The `pv_fake_mailpit` persona has no real counterpart: it is the program PV's test-only fake Mailpit adapter starts, `pv-fake-mailpit <smtp port> <dashboard port>`. It ignores further arguments, retries a busy port every 50 ms (tests release their port reservations just before PV starts the runtime), greets with `220 fake mailpit`, and answers `GET /ready` with `200` and anything else with `404`. Tests of a runtime that never becomes ready install the `long_running` persona as `bin/pv-fake-mailpit` instead.

### RustFS 1.0.0-beta.7 (2026-10-01)

These were recorded with PV's command line, `--address 127.0.0.1:<port> --console-address 127.0.0.1:<port> <data dir>`, and keys in `RUSTFS_ACCESS_KEY` and `RUSTFS_SECRET_KEY`. S3 requests were signed with SigV4 for `us-east-1`, path-style.

| Interaction | Real RustFS | `rustfs` persona |
|---|---|---|
| `GET /health` | `200 application/json`, a readiness report | `200 application/json`, `{"ready":true}` (PV checks only the status) |
| Unsigned request, API or console port | `403`, S3 `AccessDenied` | same, from `s3s` |
| Console port | PV never uses it; only the unsigned request above was recorded | knows no keys and has no `/health`, so a runtime started with the two addresses swapped never becomes ready or usable |
| Create a bucket | `200` | same |
| Create a bucket that exists | `200`, as S3 in `us-east-1`; PV's "already exists" branch never runs | same |
| Put an object | `200`, `ETag` is the quoted MD5 of the body | same |
| Head or get an object | `Content-Length`, `ETag`, `Last-Modified` | same |
| Missing object | `404 NoSuchKey` | same |
| Head a bucket | `200`; `404` when missing | same |
| Delete a non-empty bucket | `409 BucketNotEmpty` | same |
| Delete an object or an empty bucket | `204` | same |
| Wrong secret key | `403 SignatureDoesNotMatch` | same, from `s3s` |
| Data directory | buckets and `.rustfs.sys` | nothing: objects live in memory and are gone after a restart. PV creates and probes every allocation's bucket on each reconcile, so nothing it does depends on that. |
| SIGTERM, SIGINT | exit 0 in about 0.5 s | exit 0 |

The dual-target runtime contract checks, against the fake and real RustFS, the rows PV and its tests depend on: `/health` (as the adapter's readiness check), creating a bucket and creating it again, putting and heading the probe through PV's `object_store` code, getting it back, `BucketNotEmpty`, deleting the object and the bucket, a missing bucket's `404`, a wrong secret key, and SIGTERM. It uses PV's own `aws-sdk-s3` client. The `pv-fake` contract checks unsigned requests, the console port and SIGTERM against the fake only. The rest of the table is recorded but not checked. The daemon tests check buckets and probes through S3 too, instead of reading a data directory layout.

### PostgreSQL 18.4 (2026-10-02)

These were recorded from the manifest's `18.4-pv2` artifact, driven through PV's adapter: `initdb -D <dir> --username pv_root --pwfile <file> --auth-host scram-sha-256 --auth-local trust`, PV's `postgresql.conf`, then `postgres -D <dir> -h 127.0.0.1 -p <port>` and PV's `sqlx` calls.

| Interaction | Real PostgreSQL | `initdb` and `postgres` personas |
|---|---|---|
| `initdb` with PV's arguments | exit 0 in about 1 s; the full cluster layout, `PG_VERSION` is `18`, and TCP lines in `pg_hba.conf` use `scram-sha-256` | exit 0; writes `PG_VERSION`, the role in `initdb.username` and `initdb.password`, and a file per database under `databases/` |
| `initdb` into an existing empty directory | exit 0 | same |
| `initdb` into a directory that isn't empty | exit 1; `initdb: error: directory "<dir>" exists but is not empty` and a hint naming the directory | same message and exit code |
| Startup | `AuthenticationSASL` offering only `SCRAM-SHA-256` | same, from `pgwire` |
| Wrong password or unknown role | `FATAL 28P01 password authentication failed for user "<user>"` | same |
| `SELECT 1` | one `int4` row | same |
| `SELECT 1 FROM pg_database WHERE datname = $1` | a row once the database exists | same; `postgres`, `template0` and `template1` exist from the start |
| `CREATE DATABASE "<name>"` | `CREATE DATABASE`, then `42P04 database "<name>" already exists` | same |
| Restart | databases are still there | same |
| SIGTERM, no client | exit 0 in about 80 ms | exit 0 |
| SIGTERM, idle client | smart shutdown: keeps serving the client, and exits 0 about 10 ms after it disconnects. So PV stops Postgres with SIGINT instead (#391) | serves open clients until they disconnect, then exits 0. It closes its port at once, where PostgreSQL refuses new clients with `57P03` |
| SIGINT | fast shutdown: exit 0 in under 10 ms; idle clients get `FATAL 57P01` | exit 0; closes the connections without `57P01` |
| Processes | the postmaster plus workers, each in its own process group | one process |

The dual-target runtime contract checks, against the fake and real PostgreSQL: `initdb` through PV's adapter, the adapter's readiness check (a SCRAM sign-in and `SELECT 1`), PV's allocation step creating a database and then finding it, `42P04`, a wrong password's `28P01`, and PV's SIGINT stop finishing within the grace period while a client is still connected. The `pv-fake` contracts check `initdb`'s messages, the startup reply and both shutdowns against the fake only. Tests of a Postgres that never becomes ready install the `long_running` persona as `bin/postgres` next to the `initdb` persona. The two tests that check PV persisted the password it gave `initdb` read the persona's `initdb.password`.

### MySQL 8.4.9 (2026-10-03)

These were recorded from the manifest's `8.4.9-pv1` artifact, driven through PV's adapter: `mysqld --no-defaults --initialize-insecure --datadir <dir> --basedir <artifact>`, then `mysqld --no-defaults --datadir <dir> --bind-address=127.0.0.1 --port <port> --mysqlx=0 --socket <path> --init-file <path>` with PV's init file, and PV's `sqlx` calls.

| Interaction | Real MySQL | `mysqld` persona |
|---|---|---|
| `--initialize-insecure` | exit 0 in about 2.6 s; InnoDB files, TLS keys and certificates, and the `mysql/`, `performance_schema/` and `sys/` directories | exit 0; the three directories |
| `--initialize-insecure`, directory exists and is empty | exit 0 | same |
| `--initialize-insecure`, directory isn't empty | exit 1; `[ERROR] [MY-010457] … --initialize specified but the data directory has files in it. Aborting.`, then `MY-013236` naming `<dir>/` and `MY-010119 Aborting` | same lines, without the timestamp and thread |
| Start with PV's init file | ready in about 0.7 s; the init file creates `pv_root@127.0.0.1` with `caching_sha2_password`; creates the Unix socket and its `.lock` | listens at once; ignores the init file; no Unix socket |
| Greeting | protocol 10, version `8.4.9`, TLS offered, `caching_sha2_password` | none: accepts the connection and closes it |
| `sqlx` sign-in | over TLS (`TLS_AES_256_GCM_SHA384`), then `SET sql_mode=…,time_zone='+00:00',NAMES utf8mb4;` as text | not emulated |
| Wrong password or unknown user | `1045 (28000) Access denied for user '<user>'@'localhost' (using password: YES)` | not emulated |
| Prepared `SELECT 1` and `CREATE DATABASE IF NOT EXISTS` | accepted; one row affected, with note `1007` when the database exists | not emulated |
| Databases | one directory each in the data directory; still there after a restart | not emulated |
| Connecting to a missing database | `1049 (42000) Unknown database '<name>'` | not emulated |
| SIGTERM | exit 0 in about 0.6 s, or 2.7 s with an idle client, whose connection is closed. It then spends a few hundred milliseconds exiting, while macOS answers its process group with `EPERM`. PV's stop waits until the process is gone or a zombie (#394) | exit 0 at once |
| SIGINT | ignored: still running after 12 s, and clients stay usable | ignored; recorded as an event |
| Processes | one process | same |

The dual-target runtime contract checks, against the fake and real MySQL, what both can do: initialization through PV's adapter, a start with PV's arguments and init file, the port accepting connections (the readiness check of `RecordingMysqlAdmin`, which the daemon tests keep), and SIGTERM. The `pv-fake` contracts check the initialization messages and both signals against the fake only. The protocol rows above are recorded for reference: PV's `sqlx` client meets real MySQL in `tests/real_artifact_resource_matrix.rs`.

## Lints And Errors

`pv-fake` opts into workspace lints. It is an application boundary, so it uses `anyhow`. Raw process, filesystem, and executable-path primitives use narrow `#[expect(..., reason = "...")]` items, as elsewhere in the repository. It uses no `unwrap`, `expect`, or `panic!`.

## Implementation Sequence

Each step is one pull request. Each starts by recording the relevant real artifacts, and ends with the verification below.

1. **Foundation** (implemented).
   - Create the crate, the daemon example target and dev-dependency, `pv_fake::install`, the scenario file, the event log, the lifeline, and the plumbing contract tests.
   - Add the `long_running` persona, replacing the `health.rs` `sleep` runtime and the inline `fake_sql_script`.
   - Leave the supervisor tests that exercise script handling unchanged: `owned-python-runtime.py` and the `/bin/sh` descendant and argument scripts. They cover the supervisor's script-identity fallback and shell process groups, which still exist in production. They go when that fallback is removed.
2. **Gateway**, in two parts:
   - **2a** (implemented).
     - Record real Caddy (see Recorded Behavior).
     - Add the `caddy` and `frankenphp` personas and the dual-target Gateway and worker contracts.
     - Port the plain install sites in `gateway_reconciliation.rs` and `jobs.rs`, and the inline `FAKE_CADDY_SCRIPT` in `daemon_foundation.rs`.
   - **2b**, in three pull requests:
     - **2b-1** (implemented): record real reloads, make the personas apply loads and switch listeners as Caddy does, add the Gateway test controls, and extend the dual-target contracts to port changes, a busy port, and a worker fragment move.
     - **2b-2** (implemented): port the stateful control-file, no-admin, admin-only and legacy installs in `gateway_reconciliation.rs` and `jobs.rs`, seed one real CA per test home there, and delete those fixtures and the unreferenced `fake-frankenphp-hangs-on-port`. The six tests that relied on the old fixture never switching ports, and the `exit_after_load` test, set `retain_previous_listeners`. The legacy test runs the fake with `admin off` instead of supervising the Python server directly. `write_script_fake_frankenphp` and the `fake-runtime-reaped-<pid>` marker wait are gone.
     - **2b-3a** (implemented): pause settings for `validate` and `run`, a `validate` exit code, a descendant process, the `held`, `descendant_spawned` and `parent_exited` events, and signal handling while paused. A fake-only contract covers a lifeline that closed before the fake started.
     - **2b-3b** (implemented): port `daemon_foundation.rs`'s installs and barrier helpers, which patched fixture source text, to those settings and events; seed a real CA there and drop the TLS fallback and `pv-fake`'s `x509-parser` dependency; delete the shell-to-Python parent-loss variant and the last Gateway fixtures. The inline leader in `gateway_reconciliation.rs` stays a script, because its test needs a descendant that outlives the leader, which a pv-fake descendant never does; its wait is now bounded at 30 s like its descendant.
3. **Simple services**, in three pull requests:
   - **3a** (implemented): record real Redis, add the `redis-server` persona and the dual-target Managed Resource runtime contract, port the Redis installs and archive in `managed_resources/tests.rs`, and delete `redis-server.py` with its fixture contract.
   - **3b** (implemented): record real Mailpit, add the `mailpit` and `pv_fake_mailpit` personas and the `exit_after_first_http_response` setting, add the dual-target Mailpit runtime contract and a fake-only contract for PV's fake Mailpit adapter, port the Mailpit installs and archives in `managed_resources/tests.rs` and `jobs.rs` (the unready variant becomes `long_running`), and delete the four Mailpit fixtures, their fixture contracts, and the Mailpit parent-loss variant.
   - **3c** (implemented): record real RustFS, add the `rustfs` persona on `s3s` with an in-memory store and the `rustfs_reject_credentials` setting, add the dual-target RustFS runtime contract, port the RustFS installs and archive and move the tests' bucket and probe checks to S3, and delete `rustfs.py.in`, its fixture contract and the multi-server signal contract.
4. **SQL**, after a compatibility check (2026-10-01): `pgwire` and `opensrv-mysql` each answered PV's `sqlx` 0.9 calls in a scratch server.
   - **4a** (implemented): record real PostgreSQL, add the `initdb` and `postgres` personas on `pgwire` and the dual-target Postgres runtime contract, port the Postgres installs and archives in `managed_resources/tests.rs` (the unready variant becomes `long_running`), and delete `postgres.py`, `postgres-initdb.sh` and `postgres-unready.sh` with their fixture contracts, the Postgres half of the single-server signal contract, and the parent-loss contract.
   - **4b** (implemented): record real MySQL, add the `mysqld` persona (initialization, a TCP port, and MySQL's signals) and the dual-target MySQL runtime contract, port the MySQL installs and archive in `mysql_tests.rs`, and delete `mysql.py` with its fixture contracts and the single-server signal contract. `RecordingMysqlAdmin` stays: see Library compatibility.
5. **Cleanup** (implemented):
   - Delete `fixture_contracts.rs`. Its four Python scripts only tested the file's own helpers, which nothing else used once the Python runtime fixtures were gone.
   - Give every remaining shell script that stays alive a 30 s limit, and have the supervisor's descendant scripts write their PID files by rename.
   - Rewrite `CONTRIBUTING.md`'s Fixture Lifecycle section around `pv_fake::install` and the lifeline.
   - Keep the script-identity and process-group scripts (see step 1). The process-group tests need a child that outlives its parent or ignores SIGTERM, which a `pv-fake` descendant never does, and the planned per-service monitor will replace the supervisor code they test.
   - Keep `python3` as a prerequisite: the script-identity tests and `pv-release`'s installer and smoke tests use it.

## Verification

For every step:

1. Contract tests for new personas pass against the fake. Recorded snapshots are reviewed against the real artifact.
2. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo shear` pass.
3. The complete locked workspace nextest suite passes.
4. The chaos gauge reports zero survivors for ported fixtures after a normal run and after hard kills at several points in the run.
5. Several full daemon suite runs are compared against the baseline of 1–3 load-dependent failures per run.

## Risks And Mitigations

### Library compatibility

`pgwire` and `s3s` are actively released. `s3s` is what the real RustFS uses. `pgwire` adds 22 locked packages, including second versions of `base64`, `const-oid`, `fallible-iterator` and `rand_core`, and bumps no locked crate.

`opensrv-mysql` (last released February 2024) passed step 4's compatibility check: PV's `sqlx` 0.9 calls worked against it, with no TLS and the `caching_sha2_password` scramble checked by the persona. It was dropped anyway (decided 2026-10-04). It depends on `mysql_common` 0.32 with default features it can't turn off, and one of them is `flate2/zlib`. Cargo unifies features across a build, so every workspace test build, CI's included, switched every crate's `flate2` to the C zlib backend. That changed the archive bytes, digests and sizes in 10 `pv-release` and `resources` snapshot tests, which pass when those crates build alone. Turning the feature off needs a patched copy of `opensrv-mysql` plus a direct `flate2` dependency just to pick a backend. Switching the workspace to zlib would change production compression. `mysql_common` also added 43 locked packages, C builds of zstd and zlib, and `bindgen` and `cmake` as build dependencies. So the `mysqld` persona speaks no protocol and the daemon tests keep `RecordingMysqlAdmin`, as planned for a failed check.

`s3s` 0.15 and later require `async-trait` 0.1.92, which depends on `syn` 3, so `syn` 3 is compiled in every build, release builds included, through `object_store`. `s3s` also adds second versions of `nom`, `quick-xml` and `atoi`, and raised the locked versions of about 30 crates. `s3s-fs` was left out: it adds the `s3s-test` suite, `colored` 3 and `tracing-subscriber`, and PV needs only a handful of bucket and object operations.

### First-launch scans

Gatekeeper scans each new executable on its first launch, and every installed fake is a new copy. With `s3s` the debug binary grew from 16 MB to 30 MB, and 16 parallel first launches went from about 2.9 s to 4.1 s. Local runs from an app without the Developer Tools permission time out in `pv-fake`'s own contracts, which start their fakes in one burst. The fix is that permission (System Settings → Privacy & Security → Developer Tools) for the app running the tests; CI runners don't scan. `pgwire` grew the binary from 30.5 MB to 33.7 MB. If the SQL crates make first launches slower, the heavy personas can move to their own binary.

### Stale or missing fake binary

`--lib`-only and `--test <name>` runs don't build examples. `pv_fake::binary()` fails with an actionable message when the binary is missing.

A stale binary is caught too. `build.rs` hashes the crate's sources into a build ID, `install` writes that ID into the scenario file, and a fake from a different build refuses to start with instructions to rebuild the examples.

### Descriptor inheritance

The lifeline assumes the supervisor's spawn path passes non-close-on-exec descriptors to children. A daemon test starts a fake through the real supervisor and verifies that the lifeline arrives armed. A targeted chaos run then hard-killed a test mid-start: the fake recorded `started` after its parent was already gone, and its lifeline fired within 63 µs.

The per-runtime monitor planned after this migration must pass the lifeline descriptor through to the runtime. A monitor that closes inherited descriptors or daemonizes would cut it, and fakes would outlive their tests again.

### Symlink rejection

PV's artifact validation rejects symlinked executables. Fakes are copies, so they're regular files like real artifacts.

### Signaling the wrong process group

A fake only signals its own process group when it is the group leader. Otherwise it exits alone.

### Drift from real behavior

The dual-target contract suite runs in the real-artifact lane. Recorded snapshots are the reference.

### Compile time

Protocol crates are confined to `pv-fake`. The SQL crates arrive last. Editing the fake costs about 5 seconds of daemon test rebuild.

## Acceptance Criteria

The migration is complete when:

- every long-running daemon test fixture that stands in for a runtime is a `pv-fake` persona installed by `pv_fake::install`, and no `ps`-polling watcher remains,
- every persona's pv-facing behavior matches recorded real-artifact snapshots, and the contract suite runs against real binaries in the real-artifact lane,
- the chaos gauge reports zero survivors after normal runs and hard kills,
- `CONTRIBUTING.md` reflects the new fixture lifecycle, and
- formatting, Clippy, `cargo shear`, and the complete locked workspace nextest suite pass.

## Open Decisions

None. MySQL keeps `RecordingMysqlAdmin` (decided 2026-10-04; see Library compatibility).
