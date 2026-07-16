# Daemon Test Fixture Extraction Design

## Summary

PV will move substantial fake executable programs used by daemon tests out of Rust raw-string literals and into standalone fixture files under `crates/daemon/test-fixtures/`.

Python remains an appropriate implementation language for the fake network services. The problem is not Python itself; it is embedding sizeable Python programs inside shell heredocs inside Rust strings. Standalone files make each fake executable readable, directly lintable, and easier to maintain while retaining the lightweight Python standard-library implementation.

Shell remains appropriate where the fixture's behavior is fundamentally shell process behavior, such as signal traps, waiting for a child, or intentionally staying alive without becoming ready. Shell wrappers that only parse or forward arguments before starting Python will be replaced by directly executable Python fixtures. Short, scenario-specific scripts will remain inline when extracting them would only move a few obvious lines away from the test that explains them.

This is a daemon-only structural refactor. It does not include `pv-release` scripts, CI scheduling, test parallelism, or production lifecycle changes. Post-review corrections may harden the test-only subprocess runner and normalize request-thread shutdown for the extracted fixtures as described below.

## Goals

- Make substantial daemon fake executables ordinary `.py` and `.sh` files.
- Keep fixture source next to the daemon crate and organized by test domain.
- Give Python and shell tooling direct access to complete source files.
- Preserve every fixture's current command-line contract, protocol responses, filesystem effects, signal outcomes, and process-group ownership. Preserve process topology wherever the shell parent is part of the scenario; remove it only for the explicitly identified parsing/forwarding-only wrappers.
- Keep temporary test isolation by materializing fixtures into each test's existing temporary runtime or artifact directory.
- Document Python 3 as a daemon-test prerequisite.
- Keep the extraction small and explicit rather than introducing a fixture framework.

## Non-Goals

- Do not replace Python fake servers with Rust, Node, containers, or real third-party daemons.
- Do not extract every inline shell snippet in the repository.
- Do not change scripts in `crates/pv-release`, release recipes, or release workflows.
- Do not change production daemon or supervisor behavior.
- Do not otherwise change fixture readiness, shutdown, retry, timeout, or failure behavior beyond the post-review corrections described below.
- Do not change nextest configuration, CI job structure, test scheduling, or suite performance policy.
- Do not introduce third-party Python dependencies.
- Do not add a generic fixture loader, templating engine, process harness, or cross-crate abstraction.

## Extraction Boundary

A daemon test executable belongs in `test-fixtures/` when it has at least one of these properties:

- implements a network protocol or HTTP service,
- contains meaningful command-line parsing or validation,
- owns signal, child-process, or long-running lifecycle behavior,
- is reused by multiple tests or represents a named fake product executable, or
- is large enough that the Rust test is easier to understand when the program is viewed separately.

An inline script stays with its test when it is short, used by one narrow scenario, and its values or behavior are primarily explained by that test. Examples include a validator that exits with a fixed status, a one-off child-PID observer, and a tiny TERM-aware loop used by a local test adapter.

The language boundary follows behavior:

- Use direct Python for fake servers and fake product CLIs whose shell layer only parses or forwards arguments.
- Use POSIX `sh` for fixtures whose behavior is specifically shell lifecycle behavior.
- Keep a shell parent plus a Python child when the shell parent is part of the process-supervision scenario under test.

Extracted shell fixtures use `#!/bin/sh` and portable POSIX syntax. No fixture requires Bash.

## Fixture Layout

The new source layout is:

```text
crates/daemon/test-fixtures/
├── gateway/
│   ├── fake-frankenphp.sh
│   ├── fake-frankenphp-server.py
│   ├── fake-frankenphp-hangs-on-port.sh.in
│   └── fake-frankenphp-hangs-on-port-server.py
├── managed-resources/
│   ├── fake-mailpit.py
│   ├── mailpit.py
│   ├── mailpit-fast-exit.py
│   ├── mailpit-unready.sh
│   ├── mysql.py
│   ├── postgres.py
│   ├── postgres-initdb.sh
│   ├── postgres-unready.sh
│   ├── redis-server.py
│   └── rustfs.py.in
└── supervisor/
    └── owned-python-runtime.py
```

The mapping from current Rust helpers to fixture files is:

| Current helper | New fixture | Treatment |
| --- | --- | --- |
| `mysql_fixture_script` | `managed-resources/mysql.py` | Port initialization and server-mode argument handling to Python; preserve the initialization directory and TCP readiness behavior. |
| `fake_mailpit_script` | `managed-resources/fake-mailpit.py` | Direct Python executable retaining the two positional port arguments. |
| `fake_postgres_initdb_script` | `managed-resources/postgres-initdb.sh` | Keep as POSIX shell because it is a filesystem-oriented fake CLI with no embedded Python. |
| `fake_postgres_script` | `managed-resources/postgres.py` | Port argument/config validation and the PostgreSQL wire-protocol server to one direct Python executable. |
| `unready_fake_postgres_script` | `managed-resources/postgres-unready.sh` | Keep its TERM/INT traps and non-ready wait loop in POSIX shell. |
| `unready_fake_mailpit_script` | `managed-resources/mailpit-unready.sh` | Keep its TERM/INT traps and non-ready wait loop in POSIX shell. |
| `fast_exit_fake_mailpit_script` | `managed-resources/mailpit-fast-exit.py` | Move the already-direct Python executable without changing its immediate-exit readiness behavior. |
| `mailpit_script` | `managed-resources/mailpit.py` | Port the real adapter-style option validation and both fake services to direct Python. |
| `redis_server_script` | `managed-resources/redis-server.py` | Remove the forwarding shell and retain all Redis parsing, protocol, and shutdown behavior in Python. |
| `rustfs_script_source` | `managed-resources/rustfs.py.in` | Port shell argument parsing to Python and retain one narrowly parameterized reject-mode template. |
| `write_fake_frankenphp` | `gateway/fake-frankenphp.sh` plus `gateway/fake-frankenphp-server.py` | Preserve the shell parent, child wait loop, signal traps, and validation branch; move only the embedded server body to a sibling Python fixture. |
| `write_fake_frankenphp_that_hangs_on_port` | `gateway/fake-frankenphp-hangs-on-port.sh.in` plus `gateway/fake-frankenphp-hangs-on-port-server.py` | Preserve the shell parent and blocked-port hang branch; move the HTTP server body to a sibling Python fixture. |
| `supervisor_verifies_owned_python_shebang_script` body | `supervisor/owned-python-runtime.py` | Reuse a standalone direct Python executable in the existing macOS ownership test. |

The following representative scripts remain inline:

- `fake_sql_script`, because it is a tiny test-local TERM-aware loop,
- failing and hanging validator scripts in `gateway_reconciliation.rs`, because their short behavior and dynamic paths belong to individual scenarios, and
- environment and descendant-PID observer scripts in supervisor tests, because they are small and constructed from test-specific paths or values.

## Compile-Time Loading

Rust loads fixture source with crate-local compile-time inclusion:

```rust
const FAKE_MAILPIT_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/fake-mailpit.py"
));
```

This makes missing fixture files a compile error and avoids dependence on the test process's current working directory. Each test module may define the constants it needs; there will be no global registry or loader abstraction.

Existing helpers such as `state::fs::write_sensitive_file` and the local `set_executable` functions continue materializing the source into the test's temporary directory. Tests execute those temporary copies, not files from the repository checkout. This preserves isolation, executable permissions, command paths, supervisor ownership checks, and cleanup behavior.

The checked-in fixture assets do not need executable Git modes. Their materialized copies receive the same explicit executable mode as the current inline scripts.

Every directly executed fixture starts with its interpreter shebang at byte zero and uses LF line endings.

## Gateway Parent And Child Materialization

The fake FrankenPHP shell process is behaviorally important: PV supervises that parent while it starts a Python server child, handles `USR1`, forwards termination, waits, and reports the child's exit status. The extraction must preserve that topology.

For each generated fake FrankenPHP executable, the Rust helper writes:

- the shell fixture to the requested executable path, and
- its Python server fixture to the same path with `.server.py` appended.

The shell fixture refers to the companion as `"$0.server.py"`, so no runtime checkout path or additional path template is needed. All current callers execute the exact absolute fixture path derived from their temporary directory, making `$0` a stable companion-path base. The regular fake FrankenPHP wrapper runs `python3 - "$3" < "$0.server.py" &`, preserving its current Python child argument shape and standard-input execution. The blocked-port variant invokes its companion by path; this intentionally changes only that child interpreter's source argument from `-c` to a file path, while its shell parent, signal handling, and supervised identity remain unchanged. Neither companion replaces the supervised parent process.

## Narrow Template Substitution

Among the extracted fixtures, only two values require source generation:

- `rustfs.py.in` contains one `__PV_REJECT_S3__` sentinel, replaced with Python `False` or `True`.
- `fake-frankenphp-hangs-on-port.sh.in` contains one `__PV_BLOCKED_PORT__` sentinel, replaced with the test's allocated port.

The Rust substitution helper for each template must verify that the expected sentinel exists exactly once and is absent after replacement. Failure returns a test error rather than silently producing a malformed executable. No general-purpose template syntax or engine will be added.

All other dynamic values in extracted fixtures continue to arrive through the fixture's existing command-line arguments, environment, config file, or temporary path. Short inline scripts may still interpolate their scenario-specific paths and values as they do today.

## Behavioral Compatibility

This work is a source-layout refactor with explicitly identified inert-wrapper removals, so the Rust-visible executable interface is the compatibility boundary.

For every extracted fixture, preserve:

- argument order, accepted flags, rejected flags, validation rules, exit codes, and stderr messages used by tests,
- created directories, marker files, credentials, and other filesystem effects,
- bound addresses and ports,
- readiness responses and supported protocol messages,
- signal handlers and normal exit behavior,
- parent/child relationships where the shell parent is behaviorally relevant,
- the executable path, arguments, process group, and PID PV supervises after removing an inert shell wrapper, and
- paths visible in process command lines where ownership verification depends on them.

Porting shell parsing into Python must not relax validation. These small CLIs use explicit manual parsing rather than `argparse`, whose default validation and output would change the fixtures' behavior. Preserve repeated-option last-wins behavior, MySQL's unknown-option tolerance and raw first-argument rule, fake Mailpit's ignored extra arguments, Postgres and Mailpit's rejection codes, and RustFS's treatment of every unknown token as the last-wins data directory. The direct Python files report the same failures and codes as the current shell layer wherever those outcomes are observable.

The extraction also preserves the current lifecycle fixes exactly: Redis and RustFS use an idempotent `threading.Event` plus a helper thread to request server shutdown, while the fast-exit Mailpit handler sends and flushes its successful response before calling `os._exit(0)`.

No opportunistic cleanup or behavioral correction belongs in this change beyond the explicitly approved post-review corrections below.

## Post-Review Corrections

Review identified two narrow test-only hardening opportunities that are included in this branch:

- Every custom `socketserver.ThreadingMixIn` fixture server daemonizes its request threads. MySQL, PostgreSQL, fake Mailpit, and Mailpit set `daemon_threads = True`; Redis retains its existing setting. Python's `ThreadingHTTPServer` fixtures need no corresponding change because that class already daemonizes request threads.
- Every fixture-contract subprocess expected to exit uses one private standard-library runner with a 3-second deadline and a 10-millisecond poll interval. On timeout, the runner kills and reaps the direct child before returning `ErrorKind::TimedOut`. The MySQL environment probe uses the same runner while retaining its custom environment.

The subprocess runner preserves the current exit status, stdout, and stderr on normal completion. It closes stdin, captures stdout and stderr, and polls the direct child. At the deadline it requests a kill, tolerates the normal already-exited race, and always attempts to reap the child. A kill or reap failure is reported as the cleanup error; otherwise the runner returns the typed timeout error and does not present partial killed-process output as a normal fixture result. These fixture-contract commands do not intentionally create descendants, so test-only process-group management is unnecessary.

The lifecycle regression coverage holds an accepted idle client open against MySQL and PostgreSQL, sends `SIGTERM`, and requires prompt exit with bounded cleanup. These are the two fixtures whose handlers can remain blocked indefinitely. Fake Mailpit and Mailpit receive the same server configuration for consistency, but their current greeting handlers already return promptly and therefore have no distinct failing lifecycle case to protect.

These corrections do not change fixture command-line contracts, protocol responses, readiness behavior, process-group topology, production supervisor behavior, nextest configuration, or CI scheduling. They do not broaden ownership matching for non-`exec` interpreter wrappers, add metric-driven Python docstrings, or include unrelated fixture cleanup.

## Documentation And Dependencies

`CONTRIBUTING.md` will state that daemon integration tests require `python3` on `PATH`. All Python fixtures use only the standard library, so there is no virtual environment, requirements file, package installation, or version-management addition.

Shell remains a normal macOS system dependency. The extracted shell files are POSIX `sh`, not Bash scripts.

## Test And Lint Strategy

The existing daemon integration tests remain the primary behavioral coverage. They already execute the fake programs through the same artifact, adapter, gateway, and supervisor paths affected by the extraction.

Moving an unchanged program into a file does not by itself require a new test. Assertions that only prove `include_str!` returned known text would duplicate the compiler and would not protect behavior.

Porting command-line parsing from shell to Python is different: it rewrites observable fixture behavior even though the intended contract is unchanged. Add a daemon integration test for the translated fixture CLIs' currently uncovered compatibility cases. The test will materialize and invoke the final fixture executables, and use nearby `insta` patterns to snapshot exit status, stdout, and stderr where those outputs are stable. At minimum it covers:

- MySQL's initialization-first-argument rule and its existing tolerance of otherwise unknown arguments,
- fake Mailpit's two-port positional contract and ignored extra arguments,
- Postgres's unknown-argument and uninitialized-data-directory failures,
- Mailpit's unknown argument, required option, version-check, database-path, and database-directory validation, and
- RustFS's address-option consumption and last positional data-directory behavior.

Existing happy-path integration tests continue covering network protocols, readiness, filesystem effects, and lifecycle behavior. Existing snapshots and assertions must not change; only new fixture-contract snapshots may be added.

Implementation verification will include:

1. Run `shellcheck` on every extracted `.sh` source and on a rendered blocked-port `.sh.in` fixture.
2. Parse every extracted `.py` source plus both rendered `False` and `True` variants of `rustfs.py.in` with Python 3, verifying that no sentinel remains and without adding bytecode to the worktree.
3. Run the new fixture CLI compatibility integration test and review its snapshots.
4. Run focused daemon tests for MySQL, Postgres, Mailpit, Redis, RustFS, gateway reconciliation, and macOS supervisor ownership.
5. Run `cargo fmt --all --check`.
6. Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
7. Run `cargo nextest run --workspace --all-features --locked`.
8. Run `git diff --check` and inspect the final fixture permissions, shebangs, and source mapping.

The refactor will not add or change a CI workflow. The standalone files make direct linting possible, while this change's correctness gate remains the repository's normal Rust checks plus the explicit fixture lint commands above.

## Implementation Sequence

1. Add the fixture directory tree and copy each program into its target file without changing behavior.
2. Convert shell-to-Python wrappers into direct Python executables while preserving their CLI contracts.
3. Split the two gateway shell parents from their Python server children and retain the original parent lifecycle.
4. Replace Rust raw strings with compile-time fixture constants and the two narrow substitutions.
5. Add focused integration snapshots for command-line compatibility branches translated from shell to Python.
6. Remove only helper code, imports, and formatting logic made obsolete by this extraction.
7. Document the Python 3 prerequisite.
8. Run focused and full verification, then inspect the diff specifically for accidental behavior changes.

## Risks And Mitigations

### Process identity changes

Replacing a shell wrapper with Python intentionally removes that wrapper process and makes Python the tracked process. The Rust-visible executable path and arguments, process group, signal/exit outcomes, and cleanup contract remain the same. This is limited to wrappers whose shell did not contribute lifecycle behavior. The gateway shell parents remain in place because their identity and lifecycle are part of the tests. The existing supervisor ownership test protects direct Python shebang execution on macOS.

### Argument-parsing drift

Moving shell parsing into Python could accidentally accept an invalid flag or change an exit status. Each Python CLI mirrors the current branches with a manual parser, existing adapter integration tests exercise supported paths, and new compatibility snapshots cover representative failure and edge paths. Final review compares each old helper against its extracted counterpart.

### Broken companion paths

Gateway child scripts are materialized beside the shell executable and referenced relative to `$0`, avoiding dependence on the repository or current working directory. Gateway reconciliation tests cover startup, reload, validation, TLS, blocked-port, and shutdown paths.

### Template mistakes

Only two single-sentinel templates are permitted. Exact sentinel-count checks prevent partial or silent replacements.

### Hidden dependency expansion

Python fixtures remain standard-library-only, and shell fixtures remain POSIX-compatible. `CONTRIBUTING.md` makes the existing Python runtime dependency explicit.

## Acceptance Criteria

The extraction is complete when:

- all substantial daemon fake executable bodies listed above live under `crates/daemon/test-fixtures/`,
- the intentionally small or scenario-generated scripts remain inline,
- no Python heredoc remains inside an extracted daemon shell fixture,
- existing observable fixture contracts, assertions, snapshots, and production code remain unchanged apart from the documented inert-wrapper, blocked-port child-argument, and post-review test-lifecycle corrections,
- fixture-contract subprocesses that should exit are bounded, killed, and reaped on timeout,
- every custom `ThreadingMixIn` fixture server daemonizes request threads, with active-client shutdown regressions for MySQL and PostgreSQL,
- focused fixture-contract snapshots cover shell parsing that is reimplemented in Python,
- Python 3 is documented as a test prerequisite,
- extracted Python and shell sources pass their syntax and lint checks, and
- formatting, Clippy, the focused daemon tests, and the complete locked workspace nextest suite pass.
