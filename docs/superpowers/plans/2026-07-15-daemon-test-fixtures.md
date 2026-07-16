# Daemon Test Fixture Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace substantial daemon-test Python and shell raw strings with standalone fixture assets while preserving their observable CLI, protocol, lifecycle, and supervision contracts.

**Architecture:** Fixture source lives under `crates/daemon/test-fixtures/` and is embedded into test binaries with compile-time `include_str!`. Existing test helpers continue writing private temporary copies and setting executable mode; only RustFS reject mode and the blocked FrankenPHP port use exact-once sentinel rendering. Direct Python fixtures replace inert parsing/forwarding shells, while gateway fixtures retain their behaviorally relevant shell parents and materialize sibling Python server files.

**Tech Stack:** Rust 2024 workspace, `include_str!`, `anyhow`, `camino`, `camino-tempfile`, `insta`, Python 3 standard library, POSIX `sh`, `shellcheck`, Cargo nextest.

## Global Constraints

- Consult `DESIGN.md` before any implementation decision not already settled by the approved [design spec](../specs/2026-07-15-daemon-test-fixtures-design.md); ask the user if neither document answers it.
- Scope is daemon-only. Do not change `crates/pv-release`, release recipes, release workflows, CI scheduling, nextest configuration, fixture timing, or production daemon/supervisor code.
- Python fixtures must use only Python 3's standard library. Do not add Node, containers, real third-party daemons, Python packages, a requirements file, or a virtual environment.
- Direct Python is limited to the approved parsing/forwarding-only wrappers. Preserve the gateway shell parent, child wait loop, signal traps, and exit propagation.
- Keep short scenario-local scripts inline, including `fake_sql_script`, validator failure injectors, and dynamic environment/descendant-PID observers.
- Load fixture text with crate-local compile-time `include_str!`; do not add a registry, generic fixture loader, template engine, process harness, or cross-crate abstraction.
- Use only the exact `__PV_REJECT_S3__` and `__PV_BLOCKED_PORT__` sentinels. Require exactly one occurrence and verify none remains after typed `bool`/`u16` substitution.
- Checked-in fixture files remain mode `100644`. Materialized executable copies keep the existing explicit executable mode.
- Directly executed files start with their shebang at byte zero and use LF line endings. Shell fixtures use `#!/bin/sh` and POSIX syntax; no Bash feature is allowed.
- Preserve arguments, validation, stable exit codes/messages, filesystem effects, ports, protocol responses, signal outcomes, process-group ownership, and supervisor-visible executable paths described by the spec.
- Manual Python parsers must preserve permissive behavior as well as rejection behavior; do not use `argparse`.
- Preserve Redis/RustFS helper-thread shutdown and fast-exit Mailpit's flushed HTTP 200 followed by `os._exit(0)` exactly.
- Existing snapshots and assertions must not change. Only the new fixture-contract snapshots may be added.
- Prefer integration coverage and nearby `insta` patterns. Avoid `panic!`, `unreachable!`, `.unwrap()`, `.expect()`, unsafe code, Clippy ignores, and shortened variable names.
- Use Conventional Commit messages exactly as listed in each task.

### Post-Review Amendment

The approved [post-review corrections](../specs/2026-07-15-daemon-test-fixtures-design.md#post-review-corrections) supersede this plan only where it previously prohibited fixture timing and shutdown corrections. Follow the [review-corrections implementation plan](2026-07-16-daemon-fixture-review-corrections.md) for the bounded fixture-contract runner, custom `ThreadingMixIn` normalization, their regression tests, and their separate commits. Every other constraint and completed task in this plan remains unchanged.

## File Map

**Create managed-resource fixtures:**

- `crates/daemon/test-fixtures/managed-resources/mysql.py` — fake `mysqld` initialization and TCP server.
- `crates/daemon/test-fixtures/managed-resources/fake-mailpit.py` — positional two-port fake Mailpit runtime.
- `crates/daemon/test-fixtures/managed-resources/mailpit.py` — adapter-compatible Mailpit CLI and SMTP/dashboard servers.
- `crates/daemon/test-fixtures/managed-resources/mailpit-fast-exit.py` — readiness response followed by immediate process exit.
- `crates/daemon/test-fixtures/managed-resources/mailpit-unready.sh` — TERM-aware non-ready runtime.
- `crates/daemon/test-fixtures/managed-resources/postgres.py` — Postgres CLI validation and wire-protocol server.
- `crates/daemon/test-fixtures/managed-resources/postgres-initdb.sh` — fake `initdb` filesystem CLI.
- `crates/daemon/test-fixtures/managed-resources/postgres-unready.sh` — validated but non-ready Postgres runtime.
- `crates/daemon/test-fixtures/managed-resources/redis-server.py` — fake Redis config parser, RESP responses, and safe shutdown.
- `crates/daemon/test-fixtures/managed-resources/rustfs.py.in` — rendered fake RustFS API/console server.

**Create gateway and supervisor fixtures:**

- `crates/daemon/test-fixtures/gateway/fake-frankenphp.sh` — supervised shell parent for normal fake gateway/worker processes.
- `crates/daemon/test-fixtures/gateway/fake-frankenphp-server.py` — HTTP/TLS server body consumed through Python stdin.
- `crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port.sh.in` — blocked-port wrapper template.
- `crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port-server.py` — non-TLS HTTP child for the rollback scenario.
- `crates/daemon/test-fixtures/supervisor/owned-python-runtime.py` — direct shebang ownership fixture.

**Create integration coverage:**

- `crates/daemon/tests/fixture_contracts.rs` — executable-level compatibility coverage for shell parsers translated to Python.
- `crates/daemon/tests/snapshots/fixture_contracts__mysql_fixture_cli_preserves_shell_contract.snap`
- `crates/daemon/tests/snapshots/fixture_contracts__fake_mailpit_fixture_cli_ignores_extra_arguments.snap`
- `crates/daemon/tests/snapshots/fixture_contracts__postgres_fixture_cli_preserves_shell_contract.snap`
- `crates/daemon/tests/snapshots/fixture_contracts__mailpit_fixture_cli_preserves_shell_contract.snap`
- `crates/daemon/tests/snapshots/fixture_contracts__rustfs_fixture_cli_preserves_shell_contract.snap`

**Modify Rust/tests/docs:**

- `crates/daemon/src/managed_resources/mysql_tests.rs:1-16,302-349,440-509` — include and use `mysql.py`; delete the raw string.
- `crates/daemon/src/managed_resources/tests.rs:1-67,3154-3776,4168-5108` — include and use managed-resource fixtures; retain `fake_sql_script`; add exact RustFS rendering.
- `crates/daemon/tests/gateway_reconciliation.rs:1-31,1759-1885` — materialize wrapper/companion pairs and render the blocked-port template.
- `crates/daemon/tests/supervisor_foundation.rs:1-28,475-500` — include and materialize the ownership fixture.
- `CONTRIBUTING.md:19-39` — document `python3` for daemon tests.

---

### Task 1: Lock down translated fixture CLI contracts

**Files:**

- Create: `crates/daemon/tests/fixture_contracts.rs`
- Create: `crates/daemon/test-fixtures/managed-resources/mysql.py`
- Create: `crates/daemon/test-fixtures/managed-resources/fake-mailpit.py`
- Create: `crates/daemon/test-fixtures/managed-resources/postgres.py`
- Create: `crates/daemon/test-fixtures/managed-resources/mailpit.py`
- Create: `crates/daemon/test-fixtures/managed-resources/rustfs.py.in`
- Create: `crates/daemon/tests/snapshots/fixture_contracts__*.snap` (five snapshots generated by `insta`)

**Interfaces:**

- Consumes: Existing shell parser contracts from `mysql_fixture_script`, `fake_mailpit_script`, `fake_postgres_script`, `mailpit_script`, and `rustfs_script_source`.
- Produces: Standalone fixture sources consumed by Tasks 2 and 3; integration helpers `materialize_fixture`, `run_fixture`, and `render_rustfs_fixture` remain private to `fixture_contracts.rs`.

- [ ] **Step 1: Add the failing executable-contract integration test**

Create `crates/daemon/tests/fixture_contracts.rs` with this complete test harness:

```rust
use std::io::ErrorKind;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};

const MYSQL_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mysql.py"
));
const FAKE_MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/fake-mailpit.py"
));
const POSTGRES_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres.py"
));
const MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit.py"
));
const RUSTFS_FIXTURE_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/rustfs.py.in"
));
const RUSTFS_REJECT_S3_SENTINEL: &str = "__PV_REJECT_S3__";

#[expect(
    clippy::disallowed_types,
    reason = "daemon fixture contract tests execute materialized test programs"
)]
type FixtureCommand = std::process::Command;

#[derive(Debug)]
struct FixtureOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[test]
fn mysql_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mysqld");
    let probe_dir = tempdir.path().join("probe");
    let probe_path = tempdir.path().join("mkdir-target");
    let rejected_data_dir = tempdir.path().join("rejected-data");
    let first_data_dir = tempdir.path().join("first-data");
    let selected_data_dir = tempdir.path().join("selected-data");

    materialize_fixture(&fixture, MYSQL_FIXTURE)?;
    state::fs::write_sensitive_file(
        &probe_dir.join("sitecustomize.py"),
        r#"import os


def record_makedirs(path, mode=0o777, exist_ok=False):
    with open(os.environ["PV_MYSQL_MKDIR_PROBE"], "w", encoding="utf-8") as probe:
        probe.write(os.fspath(path))


os.makedirs = record_makedirs
"#,
    )?;

    let first_argument_failure = run_fixture(
        &fixture,
        &[
            "--initialize-insecure",
            "--no-defaults",
            "--datadir",
            rejected_data_dir.as_str(),
        ],
        tempdir.path(),
    )?;
    let successful_initialization = run_fixture(
        &fixture,
        &[
            "--no-defaults",
            "--bind-address=127.0.0.1",
            "--future-option",
            "--initialize-insecure",
            "--datadir",
            first_data_dir.as_str(),
            "--datadir",
            selected_data_dir.as_str(),
            "--basedir",
            tempdir.path().as_str(),
        ],
        tempdir.path(),
    )?;
    let empty_data_dir_initialization = FixtureCommand::new(fixture.as_std_path())
        .args(["--no-defaults", "--initialize-insecure"])
        .current_dir(tempdir.path())
        .env("PYTHONPATH", &probe_dir)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PV_MYSQL_MKDIR_PROBE", &probe_path)
        .output()?;
    let empty_data_dir_initialization = FixtureOutput {
        code: empty_data_dir_initialization.status.code(),
        stdout: String::from_utf8(empty_data_dir_initialization.stdout)?,
        stderr: String::from_utf8(empty_data_dir_initialization.stderr)?,
    };

    assert_fixture_snapshot(
        tempdir.path(),
        "mysql_fixture_cli_preserves_shell_contract",
        (
            first_argument_failure,
            successful_initialization,
            path_exists(&rejected_data_dir.join("mysql"))?,
            path_exists(&first_data_dir.join("mysql"))?,
            path_exists(&selected_data_dir.join("mysql"))?,
            empty_data_dir_initialization,
            state::fs::read_to_string(&probe_path)?,
        ),
    )
}

#[test]
fn fake_mailpit_fixture_cli_ignores_extra_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("fake-mailpit");
    let smtp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let smtp_port = smtp_listener.local_addr()?.port();
    let dashboard_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let dashboard_port = dashboard_listener.local_addr()?.port();

    materialize_fixture(&fixture, FAKE_MAILPIT_FIXTURE)?;
    drop(smtp_listener);
    drop(dashboard_listener);

    let mut child = FixtureCommand::new(fixture.as_std_path())
        .args([
            smtp_port.to_string(),
            dashboard_port.to_string(),
            "ignored-extra".to_owned(),
        ])
        .current_dir(tempdir.path())
        .spawn()?;
    let lifecycle = (|| {
        let readiness =
            wait_for_loopback_ports([smtp_port, dashboard_port], Duration::from_secs(3))?;
        let running_after_readiness = child.try_wait()?.is_none();

        Ok::<_, anyhow::Error>((readiness, running_after_readiness))
    })();
    let kill_result = child.kill();
    let wait_result = child.wait();

    let lifecycle = lifecycle?;
    if let Err(error) = kill_result
        && error.kind() != ErrorKind::InvalidInput
    {
        return Err(error.into());
    }
    wait_result?;

    assert_fixture_snapshot(
        tempdir.path(),
        "fake_mailpit_fixture_cli_ignores_extra_arguments",
        lifecycle,
    )
}

#[test]
fn postgres_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("postgres");
    let initialized_data_dir = tempdir.path().join("initialized-postgres");
    let selected_missing_data_dir = tempdir.path().join("selected-missing-postgres");

    materialize_fixture(&fixture, POSTGRES_FIXTURE)?;
    state::fs::write_sensitive_file(&initialized_data_dir.join("PG_VERSION"), "16\n")?;

    let unknown_argument = run_fixture(&fixture, &["--unexpected"], tempdir.path())?;
    let last_data_dir_wins = run_fixture(
        &fixture,
        &[
            "-D",
            initialized_data_dir.as_str(),
            "-D",
            selected_missing_data_dir.as_str(),
            "-h",
            "127.0.0.1",
            "-p",
            "5432",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "postgres_fixture_cli_preserves_shell_contract",
        (unknown_argument, last_data_dir_wins),
    )
}

#[test]
fn mailpit_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mailpit");
    let missing_database = tempdir.path().join("missing/mailpit.db");

    materialize_fixture(&fixture, MAILPIT_FIXTURE)?;

    let unknown_argument = run_fixture(&fixture, &["--unexpected"], tempdir.path())?;
    let missing_required_arguments =
        run_fixture(&fixture, &["--disable-version-check"], tempdir.path())?;
    let missing_version_check = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            missing_database.as_str(),
        ],
        tempdir.path(),
    )?;
    let invalid_database_path = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            "mailpit.db",
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;
    let missing_database_directory = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            missing_database.as_str(),
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;
    let duplicate_database_last_wins = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            "mailpit.db",
            "--database",
            missing_database.as_str(),
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "mailpit_fixture_cli_preserves_shell_contract",
        (
            unknown_argument,
            missing_required_arguments,
            missing_version_check,
            invalid_database_path,
            missing_database_directory,
            duplicate_database_last_wins,
        ),
    )
}

#[test]
fn rustfs_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("rustfs");
    let first_data_dir = tempdir.path().join("first-rustfs-data");
    let selected_data_dir = tempdir.path().join("selected-rustfs-data");
    let rendered = render_rustfs_fixture(false)?;

    materialize_fixture(&fixture, &rendered)?;
    let output = run_fixture(
        &fixture,
        &[
            first_data_dir.as_str(),
            "--future-option",
            selected_data_dir.as_str(),
            "--address",
            "invalid-api-address",
            "--console-address",
            "invalid-console-address",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "rustfs_fixture_cli_preserves_shell_contract",
        (
            output.code,
            output.stdout,
            output.stderr.contains("ValueError"),
            path_exists(&first_data_dir)?,
            path_exists(&selected_data_dir.join("buckets"))?,
            path_exists(&selected_data_dir.join("process-env"))?,
            path_exists(&tempdir.path().join("invalid-api-address"))?,
            path_exists(&tempdir.path().join("invalid-console-address"))?,
            rendered.contains(RUSTFS_REJECT_S3_SENTINEL),
        ),
    )
}

fn render_rustfs_fixture(reject_s3: bool) -> Result<String> {
    let occurrence_count = RUSTFS_FIXTURE_TEMPLATE
        .matches(RUSTFS_REJECT_S3_SENTINEL)
        .count();
    if occurrence_count != 1 {
        bail!(
            "RustFS fixture must contain exactly one {RUSTFS_REJECT_S3_SENTINEL} sentinel; found {occurrence_count}"
        );
    }

    let replacement = if reject_s3 { "True" } else { "False" };
    let rendered = RUSTFS_FIXTURE_TEMPLATE.replacen(RUSTFS_REJECT_S3_SENTINEL, replacement, 1);
    if rendered.contains(RUSTFS_REJECT_S3_SENTINEL) {
        bail!("RustFS fixture still contains {RUSTFS_REJECT_S3_SENTINEL} after rendering");
    }

    Ok(rendered)
}

fn materialize_fixture(path: &Utf8Path, source: &str) -> Result<()> {
    state::fs::write_sensitive_file(path, source)?;
    set_executable(path)
}

fn run_fixture(
    path: &Utf8Path,
    arguments: &[&str],
    current_dir: &Utf8Path,
) -> Result<FixtureOutput> {
    let output = FixtureCommand::new(path.as_std_path())
        .args(arguments)
        .current_dir(current_dir)
        .output()?;

    Ok(FixtureOutput {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn assert_fixture_snapshot(
    tempdir: &Utf8Path,
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(&regex_literal(tempdir.as_str()), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon fixture contract tests inspect fixture filesystem effects directly"
)]
fn path_exists(path: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon fixture contract tests set materialized fixture executable bits directly"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

fn regex_literal(value: &str) -> String {
    let mut literal = String::new();

    for character in value.chars() {
        if matches!(
            character,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            literal.push('\\');
        }
        literal.push(character);
    }

    literal
}
```

- [ ] **Step 2: Run the new integration target and confirm the fixture files are the missing implementation**

Run:

```shell
cargo nextest run -p daemon --test fixture_contracts --locked
```

Expected: compilation fails at the first `include_str!` with a missing file under `crates/daemon/test-fixtures/managed-resources/`. Do not weaken the test or replace compile-time inclusion.

- [ ] **Step 3: Add the direct MySQL and fake Mailpit fixture implementations**

Create `crates/daemon/test-fixtures/managed-resources/mysql.py`:

```python
#!/usr/bin/env python3
import os
import signal
import socketserver
import sys


arguments = list(sys.argv[1:])
first_argument = arguments[0] if arguments else ""
data_dir = ""
port = ""
initialize = False

while arguments:
    argument = arguments.pop(0)
    if argument == "--initialize-insecure":
        initialize = True
    elif argument == "--datadir":
        data_dir = arguments.pop(0)
    elif argument == "--basedir":
        arguments.pop(0)
    elif argument == "--port":
        port = arguments.pop(0)
    elif argument in {"--bind-address", "--socket"}:
        arguments.pop(0)
    elif argument == "--no-defaults":
        pass

if initialize:
    if first_argument != "--no-defaults":
        print("mysqld initialization must start with --no-defaults", file=sys.stderr)
        sys.exit(64)
    os.makedirs(f"{data_dir}/mysql", exist_ok=True)
    sys.exit(0)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.recv(1024)


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True


def stop(_signum, _frame):
    sys.exit(0)


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

server = TcpServer(("127.0.0.1", int(port)), Handler)
server.serve_forever()
```

Create `crates/daemon/test-fixtures/managed-resources/fake-mailpit.py`:

```python
#!/usr/bin/env python3
import http.server
import signal
import socketserver
import sys
import threading


smtp_port = sys.argv[1]
dashboard_port = sys.argv[2]


class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 fake mailpit\r\n")


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True


def stop(_signum, _frame):
    sys.exit(0)


smtp = TcpServer(("127.0.0.1", int(smtp_port)), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(
    ("127.0.0.1", int(dashboard_port)),
    http.server.SimpleHTTPRequestHandler,
)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=smtp.serve_forever, daemon=True).start()
dashboard.serve_forever()
```

These parsers intentionally ignore unknown/extra arguments exactly as their shell wrappers do. Do not add `else` rejection to MySQL and do not validate `sys.argv[3:]` in fake Mailpit. The MySQL empty-datadir probe must record `/mysql` without creating it, and fake Mailpit must bind both reserved loopback ports, remain alive after readiness, and be cleaned up after the probe.

- [ ] **Step 4: Add the direct Postgres fixture implementation**

Create `crates/daemon/test-fixtures/managed-resources/postgres.py`:

```python
#!/usr/bin/env python3
import os
import signal
import socketserver
import struct
import sys
import threading


arguments = list(sys.argv[1:])
data_dir = ""
argument_host = ""
argument_port = ""

while arguments:
    argument = arguments.pop(0)
    if argument == "-D":
        data_dir = arguments.pop(0)
    elif argument == "-h":
        argument_host = arguments.pop(0)
    elif argument == "-p":
        argument_port = arguments.pop(0)
    else:
        print(f"unexpected postgres argument: {argument}", file=sys.stderr)
        sys.exit(64)

if (
    not data_dir
    or not argument_host
    or not argument_port
    or not os.path.isfile(os.path.join(data_dir, "PG_VERSION"))
):
    print("postgres data dir is not initialized", file=sys.stderr)
    sys.exit(64)

argument_port = int(argument_port)
config_path = os.path.join(data_dir, "postgresql.conf")
database_dir = os.path.join(data_dir, "databases")

host = "127.0.0.1"
port = None

with open(config_path, "r", encoding="utf-8") as config:
    for line in config:
        line = line.strip()
        if line.startswith("listen_addresses"):
            host = line.split("=", 1)[1].strip().strip("'\"")
        if line.startswith("port"):
            port = int(line.split("=", 1)[1].strip())

if host != "127.0.0.1" or port is None:
    raise SystemExit("postgresql.conf did not set loopback host and port")
if argument_host != host or argument_port != port:
    raise SystemExit("postgres arguments did not match generated config")

os.makedirs(database_dir, exist_ok=True)
with open(
    os.path.join(data_dir, "postgres.started"), "w", encoding="utf-8"
) as started:
    started.write(f"{host}:{port}\n")


def packet(message_type, payload=b""):
    return message_type + struct.pack("!I", len(payload) + 4) + payload


def auth_ok():
    return packet(b"R", struct.pack("!I", 0))


def parameter_status(key, value):
    return packet(b"S", key.encode() + b"\0" + value.encode() + b"\0")


def backend_key_data():
    return packet(b"K", struct.pack("!II", os.getpid() & 0x7FFFFFFF, 1))


def ready():
    return packet(b"Z", b"I")


def parameter_description(query):
    if "$1" in query:
        return packet(b"t", struct.pack("!H", 1) + struct.pack("!I", 25))
    return packet(b"t", struct.pack("!H", 0))


def command_complete(tag):
    return packet(b"C", tag.encode() + b"\0")


def parse_complete():
    return packet(b"1")


def bind_complete():
    return packet(b"2")


def close_complete():
    return packet(b"3")


def no_data():
    return packet(b"n")


def row_description():
    field = b"?column?\0" + struct.pack("!IhIhih", 0, 0, 23, 4, -1, 0)
    return packet(b"T", struct.pack("!H", 1) + field)


def data_row(value):
    data = str(value).encode()
    return packet(b"D", struct.pack("!H", 1) + struct.pack("!I", len(data)) + data)


def error_response(message):
    return packet(b"E", b"SERROR\0CXX000\0M" + message.encode() + b"\0\0")


def cstring(payload, start):
    end = payload.index(b"\0", start)
    return payload[start:end].decode(), end + 1


def read_exact(stream, length):
    data = b""
    while len(data) < length:
        chunk = stream.recv(length - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def read_startup(stream):
    length = struct.unpack("!I", read_exact(stream, 4))[0]
    payload = read_exact(stream, length - 4)
    code = struct.unpack("!I", payload[:4])[0]
    if code == 80877103:
        stream.sendall(b"N")
        return read_startup(stream)
    return payload


def startup_response():
    return b"".join(
        [
            auth_ok(),
            parameter_status("server_version", "16.0"),
            parameter_status("server_encoding", "UTF8"),
            parameter_status("client_encoding", "UTF8"),
            parameter_status("DateStyle", "ISO, MDY"),
            parameter_status("integer_datetimes", "on"),
            parameter_status("standard_conforming_strings", "on"),
            backend_key_data(),
            ready(),
        ]
    )


def database_file(database):
    safe = "".join(character for character in database if character.isalnum() or character == "_")
    if safe != database:
        raise ValueError("unsafe database name")
    return os.path.join(database_dir, database)


def database_exists(database):
    return os.path.exists(database_file(database))


def create_database(database):
    with open(database_file(database), "w", encoding="utf-8") as marker:
        marker.write(database + "\n")


def database_from_create(query):
    quoted = query.split("CREATE DATABASE", 1)[1].strip()
    if quoted.startswith('"') and quoted.endswith('"'):
        return quoted[1:-1]
    return quoted


def query_response(query, params):
    normalized = " ".join(query.strip().split())
    if normalized.upper() in {"SELECT 1", "SELECT $1"}:
        return row_description() + data_row(1) + command_complete("SELECT 1")
    if "FROM pg_database WHERE datname" in normalized:
        database = params[0] if params else ""
        if database_exists(database):
            return row_description() + data_row(1) + command_complete("SELECT 1")
        return row_description() + command_complete("SELECT 0")
    if normalized.upper().startswith("CREATE DATABASE"):
        create_database(database_from_create(normalized))
        return command_complete("CREATE DATABASE")
    if normalized.upper().startswith("SET "):
        return command_complete("SET")
    return error_response("unsupported fixture query: " + normalized)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        statements = {}
        portals = {}
        try:
            read_startup(self.request)
            self.request.sendall(startup_response())
            while True:
                message_type = read_exact(self.request, 1)
                length = struct.unpack("!I", read_exact(self.request, 4))[0]
                payload = read_exact(self.request, length - 4)
                if message_type == b"X":
                    return
                if message_type == b"Q":
                    query = payload[:-1].decode()
                    self.request.sendall(query_response(query, []) + ready())
                    continue
                if message_type == b"P":
                    statement, offset = cstring(payload, 0)
                    query, _offset = cstring(payload, offset)
                    statements[statement] = query
                    self.request.sendall(parse_complete())
                    continue
                if message_type == b"B":
                    portal, offset = cstring(payload, 0)
                    statement, offset = cstring(payload, offset)
                    format_count = struct.unpack("!H", payload[offset : offset + 2])[0]
                    offset += 2 + (format_count * 2)
                    param_count = struct.unpack("!H", payload[offset : offset + 2])[0]
                    offset += 2
                    params = []
                    for _index in range(param_count):
                        size = struct.unpack("!i", payload[offset : offset + 4])[0]
                        offset += 4
                        if size == -1:
                            params.append(None)
                        else:
                            params.append(payload[offset : offset + size].decode())
                            offset += size
                    portals[portal] = (statements.get(statement, ""), params)
                    self.request.sendall(bind_complete())
                    continue
                if message_type == b"D":
                    describe_kind = payload[:1]
                    name = payload[1:-1].decode()
                    query, _params = portals.get(name, (statements.get(name, ""), []))
                    response = b""
                    if describe_kind == b"S":
                        response += parameter_description(query)
                    if query.strip().upper().startswith("CREATE DATABASE"):
                        response += no_data()
                    else:
                        response += row_description()
                    self.request.sendall(response)
                    continue
                if message_type == b"E":
                    portal, offset = cstring(payload, 0)
                    _max_rows = struct.unpack("!I", payload[offset : offset + 4])[0]
                    query, params = portals.get(portal, ("", []))
                    self.request.sendall(query_response(query, params))
                    continue
                if message_type == b"S":
                    self.request.sendall(ready())
                    continue
                if message_type == b"H":
                    continue
                if message_type == b"C":
                    self.request.sendall(close_complete())
                    continue
                self.request.sendall(error_response("unsupported message type"))
        except (EOFError, ConnectionResetError, BrokenPipeError):
            return


class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True


server = Server((host, port), Handler)


def stop(_signum, _frame):
    server.shutdown()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=server.serve_forever, daemon=True).start()
signal.pause()
```

The protocol implementation is an extraction of the current body. The only intentional rewrite is the manual option loop at the top. Exact `-D`, `-h`, and `-p` options reject anything else with exit code 64, and repeated options remain last-wins.

- [ ] **Step 5: Add the direct Mailpit and rendered RustFS fixture implementations**

Create `crates/daemon/test-fixtures/managed-resources/mailpit.py`:

```python
#!/usr/bin/env python3
import http.server
import os
import signal
import socketserver
import sys
import threading


arguments = list(sys.argv[1:])
smtp = ""
listen = ""
database = ""
disable_version_check = False

while arguments:
    argument = arguments.pop(0)
    if argument == "--smtp":
        smtp = arguments.pop(0)
    elif argument == "--listen":
        listen = arguments.pop(0)
    elif argument == "--database":
        database = arguments.pop(0)
    elif argument == "--disable-version-check":
        disable_version_check = True
    else:
        print(f"unexpected argument: {argument}", file=sys.stderr)
        sys.exit(2)

if not smtp or not listen or not database:
    print("missing required mailpit argument", file=sys.stderr)
    sys.exit(2)

if not disable_version_check:
    print("missing --disable-version-check", file=sys.stderr)
    sys.exit(2)

if not database.endswith("/mailpit.db"):
    print(f"unexpected database path: {database}", file=sys.stderr)
    sys.exit(2)

database_dir = os.path.dirname(database)
if not os.path.isdir(database_dir):
    print(f"database directory does not exist: {database_dir}", file=sys.stderr)
    sys.exit(2)


def host_port(value):
    host, port = value.rsplit(":", 1)
    return host, int(port)


class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 mailpit fixture\r\n")


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True


smtp_server = TcpServer(host_port(smtp), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(
    host_port(listen),
    http.server.SimpleHTTPRequestHandler,
)


def stop(_signum, _frame):
    sys.exit(0)


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=smtp_server.serve_forever, daemon=True).start()
dashboard.serve_forever()
```

Create `crates/daemon/test-fixtures/managed-resources/rustfs.py.in`:

```python
#!/usr/bin/env python3
import hashlib
import http.server
import os
import posixpath
import signal
import sys
import threading
import urllib.parse


arguments = list(sys.argv[1:])
api_address = ""
console_address = ""
data_dir = ""

while arguments:
    argument = arguments.pop(0)
    if argument == "--address":
        api_address = arguments.pop(0)
    elif argument == "--console-address":
        console_address = arguments.pop(0)
    else:
        data_dir = argument

reject_s3 = __PV_REJECT_S3__
buckets_dir = os.path.join(data_dir, "buckets")
os.makedirs(buckets_dir, exist_ok=True)
with open(os.path.join(data_dir, "process-env"), "w", encoding="utf-8") as file:
    file.write(f"RUSTFS_ACCESS_KEY={os.environ.get('RUSTFS_ACCESS_KEY', '')}\n")
    file.write(f"RUSTFS_SECRET_KEY={os.environ.get('RUSTFS_SECRET_KEY', '')}\n")


def split_address(value):
    host, port = value.rsplit(":", 1)
    return host, int(port)


def bucket_path(bucket):
    return os.path.join(buckets_dir, bucket)


def object_path(bucket, key):
    clean_key = posixpath.normpath("/" + key).lstrip("/")
    return os.path.join(bucket_path(bucket), clean_key)


class RustfsHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path in {"/", "/health"}:
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"rustfs")
            return

        self.send_response(404)
        self.end_headers()

    def do_PUT(self):
        if reject_s3:
            self.send_response(403)
            self.end_headers()
            return

        path = urllib.parse.urlparse(self.path).path.strip("/")
        parts = path.split("/", 1)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        if not parts or not parts[0]:
            self.send_response(400)
            self.end_headers()
            return

        bucket = parts[0]
        if len(parts) == 1:
            os.makedirs(bucket_path(bucket), exist_ok=True)
            self.send_response(200)
            self.end_headers()
            return

        target = object_path(bucket, parts[1])
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "wb") as file:
            file.write(body)
        self.send_response(200)
        self.send_header("ETag", hashlib.md5(body).hexdigest())
        self.end_headers()

    def do_HEAD(self):
        if reject_s3:
            self.send_response(403)
            self.end_headers()
            return

        path = urllib.parse.urlparse(self.path).path.strip("/")
        parts = path.split("/", 1)
        if len(parts) != 2:
            exists = bool(parts and parts[0] and os.path.isdir(bucket_path(parts[0])))
            self.send_response(200 if exists else 404)
            self.end_headers()
            return

        target = object_path(parts[0], parts[1])
        if not os.path.exists(target):
            self.send_response(404)
            self.end_headers()
            return

        size = os.path.getsize(target)
        with open(target, "rb") as file:
            digest = hashlib.md5(file.read()).hexdigest()
        self.send_response(200)
        self.send_header("Content-Length", str(size))
        self.send_header("ETag", digest)
        self.end_headers()

    def log_message(self, _format, *_args):
        return


class ConsoleHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"rustfs console")

    def log_message(self, _format, *_args):
        return


class Server(http.server.ThreadingHTTPServer):
    allow_reuse_address = True


api = Server(split_address(api_address), RustfsHandler)
console = Server(split_address(console_address), ConsoleHandler)

shutdown_requested = threading.Event()


def stop(_signum, _frame):
    if shutdown_requested.is_set():
        return
    shutdown_requested.set()
    threading.Thread(target=api.shutdown, daemon=True).start()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=console.serve_forever, daemon=True).start()
api.serve_forever()
console.shutdown()
```

Do not use `argparse`: Mailpit's duplicate values must remain last-wins, and every unknown RustFS token must continue becoming the data directory, with the last token winning.

- [ ] **Step 6: Generate and review the five new snapshots**

Run:

```shell
INSTA_UPDATE=always cargo nextest run -p daemon --test fixture_contracts --locked
```

Expected: all five tests pass and create five `.snap` files. Inspect them before continuing. The stable outcomes must show:

```text
MySQL: rejected code 64 with "mysqld initialization must start with --no-defaults"; accepted code 0; only selected-data/mysql exists; the empty-datadir initialization is code 0 and the sitecustomize probe records `/mysql` without creating it.
Fake Mailpit: both reserved loopback ports become ready and the child remains alive after readiness before cleanup.
Postgres: both cases code 64; one "unexpected postgres argument" and one "postgres data dir is not initialized".
Mailpit: all six cases code 2 with the five explicit validation messages; the duplicate-database case repeats the missing-directory result to prove last-wins parsing, and the path is normalized to <tempdir>.
RustFS: code 1 after invalid address parsing; ValueError=true; only selected-rustfs-data contains buckets and process-env; neither consumed address value becomes a directory; sentinel-present=false.
```

Then run without snapshot updates:

```shell
cargo nextest run -p daemon --test fixture_contracts --locked
```

Expected: `5 passed` with no `.snap.new` files.

- [ ] **Step 7: Run formatting and commit the contract boundary**

Run:

```shell
cargo fmt --all
cargo fmt --all --check
git diff --check
```

Expected: all three commands exit 0.

Commit:

```shell
git add crates/daemon/test-fixtures/managed-resources/mysql.py \
  crates/daemon/test-fixtures/managed-resources/fake-mailpit.py \
  crates/daemon/test-fixtures/managed-resources/postgres.py \
  crates/daemon/test-fixtures/managed-resources/mailpit.py \
  crates/daemon/test-fixtures/managed-resources/rustfs.py.in \
  crates/daemon/tests/fixture_contracts.rs \
  crates/daemon/tests/snapshots/fixture_contracts__*.snap
git commit -m "test(daemon): cover managed fixture CLI contracts"
```

---

### Task 2: Extract MySQL, Postgres, and Mailpit fixture assets

**Files:**

- Create: `crates/daemon/test-fixtures/managed-resources/postgres-initdb.sh`
- Create: `crates/daemon/test-fixtures/managed-resources/postgres-unready.sh`
- Create: `crates/daemon/test-fixtures/managed-resources/mailpit-unready.sh`
- Create: `crates/daemon/test-fixtures/managed-resources/mailpit-fast-exit.py`
- Modify: `crates/daemon/src/managed_resources/mysql_tests.rs:1-16,302-349,440-509`
- Modify: `crates/daemon/src/managed_resources/tests.rs:1-67,3154-3776,4168-4861`
- Test: `crates/daemon/tests/fixture_contracts.rs`

**Interfaces:**

- Consumes: The direct Python fixtures and contract snapshots from Task 1; existing `state::fs::write_sensitive_file` and file-local `set_executable` helpers.
- Produces: Compile-time `&'static str` fixture constants used by all MySQL, Postgres, and Mailpit artifact/archive helpers. `fake_sql_script` deliberately remains inline.

- [ ] **Step 1: Record a green focused baseline before moving source**

Run:

```shell
cargo nextest run -p daemon --lib --all-features --locked
cargo nextest run -p daemon --test fixture_contracts --all-features --locked
```

Expected: the daemon library target and all five contract tests pass. This is a behavior-preserving extraction, so the same commands must pass afterward without snapshot changes.

- [ ] **Step 2: Add the standalone POSIX lifecycle/CLI fixtures**

Create `crates/daemon/test-fixtures/managed-resources/postgres-initdb.sh`:

```sh
#!/bin/sh
set -eu

data_dir=""
username=""
password_file=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -U)
      username="$2"
      shift 2
      ;;
    --username)
      username="$2"
      shift 2
      ;;
    --pwfile)
      password_file="$2"
      shift 2
      ;;
    --auth-host|--auth-local)
      shift 2
      ;;
    *)
      echo "unexpected initdb argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ -z "$username" ] || [ -z "$password_file" ]; then
  echo "missing initdb inputs" >&2
  exit 64
fi

if [ -d "$data_dir" ] && [ "$(find "$data_dir" -mindepth 1 -maxdepth 1 | wc -l)" -gt 0 ]; then
  echo "PGDATA is not empty before initdb" >&2
  exit 65
fi

mkdir -p "$data_dir/databases"
printf '16\n' > "$data_dir/PG_VERSION"
printf '%s\n' "$username" > "$data_dir/initdb.username"
cat "$password_file" > "$data_dir/initdb.password"
```

Create `crates/daemon/test-fixtures/managed-resources/postgres-unready.sh`:

```sh
#!/bin/sh
set -eu

data_dir=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -h|-p)
      shift 2
      ;;
    *)
      echo "unexpected postgres argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ ! -f "$data_dir/PG_VERSION" ]; then
  echo "postgres data dir is not initialized" >&2
  exit 64
fi

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  sleep 1
done
```

Create `crates/daemon/test-fixtures/managed-resources/mailpit-unready.sh`:

```sh
#!/bin/sh
set -eu

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  sleep 1
done
```

Keep these as POSIX shell because their filesystem or signal/wait behavior is the fixture. Do not translate them to Python.

- [ ] **Step 3: Add the direct fast-exit Mailpit fixture**

Create `crates/daemon/test-fixtures/managed-resources/mailpit-fast-exit.py`:

```python
#!/usr/bin/env python3
import http.server
import os
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ready")
        self.wfile.flush()
        os._exit(0)

    def log_message(self, _format, *_args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), Handler)
server.serve_forever()
```

Do not replace `os._exit(0)` with normal shutdown or move it outside the request handler. The response body must be written and flushed first.

- [ ] **Step 4: Add compile-time fixture constants to the two managed-resource test modules**

In `crates/daemon/src/managed_resources/mysql_tests.rs`, add this next to the existing test constants:

```rust
const MYSQL_FIXTURE_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mysql.py"
));
```

In `crates/daemon/src/managed_resources/tests.rs`, add these next to the other test constants:

```rust
const FAKE_MAILPIT_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/fake-mailpit.py"
));
const MAILPIT_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit.py"
));
const MAILPIT_FAST_EXIT_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit-fast-exit.py"
));
const MAILPIT_UNREADY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit-unready.sh"
));
const POSTGRES_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres.py"
));
const POSTGRES_INITDB_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres-initdb.sh"
));
const POSTGRES_UNREADY_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres-unready.sh"
));
```

- [ ] **Step 5: Replace the raw-string accessors with the fixture constants**

In `mysql_tests.rs`, make both archive/artifact writes use the constant:

```rust
state::fs::write_sensitive_file(&executable, MYSQL_FIXTURE_SCRIPT)?;
```

Delete `mysql_fixture_script` entirely.

In `managed_resources/tests.rs`, apply these exact expression replacements everywhere in artifact seeding, archive creation, and Postgres binary materialization:

```text
fake_mailpit_script()                -> FAKE_MAILPIT_SCRIPT
mailpit_script()                     -> MAILPIT_SCRIPT
unready_fake_mailpit_script()        -> MAILPIT_UNREADY_SCRIPT
fast_exit_fake_mailpit_script()      -> MAILPIT_FAST_EXIT_SCRIPT
fake_postgres_initdb_script()        -> POSTGRES_INITDB_SCRIPT
fake_postgres_script()               -> POSTGRES_SCRIPT
unready_fake_postgres_script()       -> POSTGRES_UNREADY_SCRIPT
```

The resulting seed helpers must have this shape:

```rust
fn seed_fake_mailpit_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_fake_mailpit_artifact_with_script(paths, track, FAKE_MAILPIT_SCRIPT)
}

fn seed_mailpit_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    let release_path = paths
        .resources()
        .join("mailpit")
        .join(track)
        .join(format!("releases/{FAKE_MAILPIT_ARTIFACT_VERSION}"));
    let executable = release_path.join("bin/mailpit");

    state::fs::write_sensitive_file(&executable, MAILPIT_SCRIPT)?;
    set_executable(&executable)?;
    let mut database = Database::open(paths)?;
    database.record_managed_resource_track_installed(
        "mailpit",
        track,
        FAKE_MAILPIT_ARTIFACT_VERSION,
        &release_path,
    )?;

    Ok(())
}

fn seed_unready_fake_mailpit_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_fake_mailpit_artifact_with_script(paths, track, MAILPIT_UNREADY_SCRIPT)
}

fn seed_fast_exit_fake_mailpit_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_fake_mailpit_artifact_with_script(paths, track, MAILPIT_FAST_EXIT_SCRIPT)
}

fn seed_postgres_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_postgres_fixture_artifact_with_script(paths, track, POSTGRES_SCRIPT)
}

fn seed_unready_postgres_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    seed_postgres_fixture_artifact_with_script(paths, track, POSTGRES_UNREADY_SCRIPT)
}
```

`write_postgres_fixture_binaries_without_support_files` must write the two constants directly:

```rust
state::fs::write_sensitive_file(&initdb, POSTGRES_INITDB_SCRIPT)?;
state::fs::write_sensitive_file(&postgres, POSTGRES_SCRIPT)?;
```

Delete only these seven obsolete raw-string functions. Keep `fake_sql_script` inline and do not change the shared `*_with_script` helpers because failure scenarios still inject alternate fixture source.

- [ ] **Step 6: Lint the extracted scripts and rerun the behavioral targets**

Run:

```shell
shellcheck \
  crates/daemon/test-fixtures/managed-resources/postgres-initdb.sh \
  crates/daemon/test-fixtures/managed-resources/postgres-unready.sh \
  crates/daemon/test-fixtures/managed-resources/mailpit-unready.sh
cargo fmt --all
cargo nextest run -p daemon --lib --all-features --locked
cargo nextest run -p daemon --test fixture_contracts --all-features --locked
git diff --check
```

Expected: shellcheck exits 0, the daemon library target passes, all five fixture-contract snapshots remain unchanged, and the diff check is clean.

- [ ] **Step 7: Commit the SQL and Mailpit extraction**

```shell
git add crates/daemon/test-fixtures/managed-resources/postgres-initdb.sh \
  crates/daemon/test-fixtures/managed-resources/postgres-unready.sh \
  crates/daemon/test-fixtures/managed-resources/mailpit-unready.sh \
  crates/daemon/test-fixtures/managed-resources/mailpit-fast-exit.py \
  crates/daemon/src/managed_resources/mysql_tests.rs \
  crates/daemon/src/managed_resources/tests.rs
git commit -m "refactor(daemon): extract SQL and Mailpit test fixtures"
```

---

### Task 3: Extract Redis and RustFS server fixtures

**Files:**

- Create: `crates/daemon/test-fixtures/managed-resources/redis-server.py`
- Modify: `crates/daemon/src/managed_resources/tests.rs:1-67,3471-3746,4862-5108`
- Test: `crates/daemon/tests/fixture_contracts.rs`

**Interfaces:**

- Consumes: `rustfs.py.in` and its executable contract from Task 1; existing Redis and RustFS archive/artifact helpers.
- Produces: `REDIS_SERVER_SCRIPT`, `RUSTFS_SCRIPT_TEMPLATE`, and fallible `rustfs_script_source(bool) -> Result<String>` for all daemon managed-resource tests.

- [ ] **Step 1: Add the direct Redis server fixture**

Create `crates/daemon/test-fixtures/managed-resources/redis-server.py` by removing only the forwarding shell from the current fixture:

```python
#!/usr/bin/env python3
import os
import signal
import shlex
import socketserver
import sys
import threading


def redis_config(argv):
    port = None
    data_dir = None
    arguments = list(argv)
    while arguments:
        argument = arguments.pop(0)
        if argument == "--port" and arguments:
            port = int(arguments.pop(0))
        elif argument == "--dir" and arguments:
            data_dir = arguments.pop(0)
        elif os.path.isfile(argument):
            with open(argument, "r", encoding="utf-8") as config:
                for line in config:
                    parts = shlex.split(line)
                    if len(parts) == 2 and parts[0] == "port":
                        port = int(parts[1])
                    elif len(parts) == 2 and parts[0] == "dir":
                        data_dir = parts[1]
    if port is None:
        raise RuntimeError("missing Redis port")
    return port, data_dir


class RedisPingHandler(socketserver.BaseRequestHandler):
    def handle(self):
        while True:
            data = self.request.recv(4096)
            if not data:
                return
            upper = data.upper()
            responses = []
            for _index in range(upper.count(b"CLIENT")):
                responses.append(b"+OK\r\n")
            for _index in range(upper.count(b"PING")):
                responses.append(b"+PONG\r\n")
            if not responses:
                responses.append(b"+OK\r\n")
            self.request.sendall(b"".join(responses))


class RedisServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


shutdown_requested = threading.Event()


def stop(_signum, _frame):
    if shutdown_requested.is_set():
        return
    shutdown_requested.set()
    threading.Thread(target=server.shutdown, daemon=True).start()


port, data_dir = redis_config(sys.argv[1:])
if data_dir:
    os.makedirs(data_dir, exist_ok=True)

server = RedisServer(("127.0.0.1", port), RedisPingHandler)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
server.serve_forever()
```

Do not call `server.shutdown()` directly from the signal handler. The event and helper thread prevent Python's same-thread `serve_forever()` deadlock.

- [ ] **Step 2: Add the Redis and RustFS compile-time constants**

Add to `crates/daemon/src/managed_resources/tests.rs`:

```rust
const REDIS_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/redis-server.py"
));
const RUSTFS_SCRIPT_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/rustfs.py.in"
));
const RUSTFS_REJECT_S3_SENTINEL: &str = "__PV_REJECT_S3__";
```

Replace both `redis_server_script()` call sites with `REDIS_SERVER_SCRIPT`, then delete `redis_server_script`.

- [ ] **Step 3: Replace the inline RustFS source generator with exact-once typed rendering**

Replace the current three RustFS source functions with:

```rust
fn rustfs_script() -> Result<String> {
    rustfs_script_source(false)
}

fn auth_rejecting_rustfs_script() -> Result<String> {
    rustfs_script_source(true)
}

fn rustfs_script_source(reject_s3: bool) -> Result<String> {
    let occurrence_count = RUSTFS_SCRIPT_TEMPLATE
        .matches(RUSTFS_REJECT_S3_SENTINEL)
        .count();
    if occurrence_count != 1 {
        bail!(
            "RustFS fixture must contain exactly one {RUSTFS_REJECT_S3_SENTINEL} sentinel; found {occurrence_count}"
        );
    }

    let replacement = if reject_s3 { "True" } else { "False" };
    let script = RUSTFS_SCRIPT_TEMPLATE.replacen(RUSTFS_REJECT_S3_SENTINEL, replacement, 1);
    if script.contains(RUSTFS_REJECT_S3_SENTINEL) {
        bail!("RustFS fixture still contains {RUSTFS_REJECT_S3_SENTINEL} after rendering");
    }

    Ok(script)
}
```

Propagate the new `Result<String>` explicitly at every generated call site:

```rust
fn seed_rustfs_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    let script = rustfs_script()?;
    seed_rustfs_fixture_artifact_with_script(paths, track, &script)
}

fn seed_auth_rejecting_rustfs_fixture_artifact(paths: &PvPaths, track: &str) -> Result<()> {
    let script = auth_rejecting_rustfs_script()?;
    seed_rustfs_fixture_artifact_with_script(paths, track, &script)
}
```

In `create_rustfs_archive`, render before writing:

```rust
let script = rustfs_script()?;
state::fs::write_sensitive_file(&executable, &script)?;
```

Delete the old raw RustFS shell/Python heredoc. Do not alter `seed_rustfs_fixture_artifact_with_script`, because the auth-rejection scenario still passes rendered source into it.

- [ ] **Step 4: Parse the direct Python and both rendered template variants in memory**

Run:

```shell
python3 -c 'import ast, pathlib; paths = [pathlib.Path("crates/daemon/test-fixtures/managed-resources/redis-server.py")]; [ast.parse(path.read_text(), filename=str(path)) for path in paths]'
python3 -c 'import ast, pathlib; path = pathlib.Path("crates/daemon/test-fixtures/managed-resources/rustfs.py.in"); source = path.read_text(); sentinel = "__PV_REJECT_S3__"; assert source.count(sentinel) == 1; [ast.parse(source.replace(sentinel, value), filename=f"{path}:{value}") for value in ("False", "True")]'
```

Expected: both commands exit 0 and no `__pycache__` directory is created.

- [ ] **Step 5: Run Redis/RustFS lifecycle coverage and ensure snapshots are unchanged**

Run:

```shell
cargo fmt --all
cargo nextest run -p daemon --lib --all-features --locked \
  -E 'test(redis_reconciliation_marks_prefix_allocation_ready_and_renders_env) | test(redis_project_demand_installs_missing_fixture_track_before_start) | test(rustfs_reconciliation_creates_bucket_and_renders_env) | test(rustfs_project_demand_installs_missing_fixture_track_before_start) | test(rustfs_allocation_failure_preserves_project_env_and_records_failed_runtime) | test(rustfs_runtime_receives_private_credentials_without_persisting_them)'
cargo nextest run -p daemon --test fixture_contracts --all-features --locked
git diff --check
```

Expected: every selected lifecycle test and all five fixture contracts pass. No existing `.snap` file changes.

- [ ] **Step 6: Commit the Redis and RustFS extraction**

```shell
git add crates/daemon/test-fixtures/managed-resources/redis-server.py \
  crates/daemon/src/managed_resources/tests.rs
git commit -m "refactor(daemon): extract Redis and RustFS test fixtures"
```

---

### Task 4: Extract gateway shell parents and Python server companions

**Files:**

- Create: `crates/daemon/test-fixtures/gateway/fake-frankenphp.sh`
- Create: `crates/daemon/test-fixtures/gateway/fake-frankenphp-server.py`
- Create: `crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port.sh.in`
- Create: `crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port-server.py`
- Modify: `crates/daemon/tests/gateway_reconciliation.rs:1-31,1759-1885`

**Interfaces:**

- Consumes: Existing absolute `bin/frankenphp` paths beneath each temporary release and `set_executable`; existing gateway reconciliation tests.
- Produces: `write_fake_frankenphp(&Utf8Path) -> Result<()>`, `write_fake_frankenphp_that_hangs_on_port(&Utf8Path, u16) -> Result<()>`, and a private exact-once blocked-port renderer with unchanged call sites.

- [ ] **Step 1: Add the normal FrankenPHP shell parent and Python server body**

Create `crates/daemon/test-fixtures/gateway/fake-frankenphp.sh`:

```sh
#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  python3 - "$3" < "$0.server.py" &
  child="$!"
  trap ':' USR1
  trap 'kill "$child"; wait "$child"; exit 0' TERM INT
  while true; do
    wait "$child" && exit 0
    status="$?"
    if kill -0 "$child" 2>/dev/null; then
      continue
    fi
    exit "$status"
  done
fi

exit 2
```

Create `crates/daemon/test-fixtures/gateway/fake-frankenphp-server.py`:

```python
#!/usr/bin/env python3
import http.server
import re
import signal
import ssl
import sys
import threading


signal.signal(signal.SIGUSR1, signal.SIG_IGN)

config = open(sys.argv[1], encoding="utf-8").read()


def required(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        raise SystemExit(f"missing fake runtime setting: {pattern}")
    return match.group(1)


def optional(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        return None
    return match.group(1)


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):
        pass


http_port = int(required(r"^# PV_FAKE_PORT (\d+)$"))
https_port = optional(r"^\s*https_port (\d+)$")
cert_path = optional(r'^\s*cert "([^"]+)"$')
key_path = optional(r'^\s*key "([^"]+)"$')
servers = [http.server.ThreadingHTTPServer(("127.0.0.1", http_port), Handler)]

if https_port is not None and cert_path is not None and key_path is not None:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    https_server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", int(https_port)), Handler
    )
    https_server.socket = context.wrap_socket(https_server.socket, server_side=True)
    servers.append(https_server)

for server in servers[1:]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

with servers[0] as server:
    server.serve_forever()
```

The shell invocation must stay exactly `python3 - "$3" < "$0.server.py" &`. It preserves the existing Python `sys.argv[0] == "-"`, config argument position, shell PID, process group, signal forwarding, and child-status loop.

- [ ] **Step 2: Add the blocked-port shell template and server companion**

Create `crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port.sh.in`:

```sh
#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  port="$(awk '/^# PV_FAKE_PORT / { print $3; exit }' "$3")"
  if [ "$port" = "__PV_BLOCKED_PORT__" ]; then
    sleep 30
    exit 0
  fi
  python3 "$0.server.py" "$port" &
  child="$!"
  trap ':' USR1
  trap 'kill "$child"; wait "$child"; exit 0' TERM INT
  while true; do
    wait "$child" && exit 0
    status="$?"
    if kill -0 "$child" 2>/dev/null; then
      continue
    fi
    exit "$status"
  done
fi

exit 2
```

Create `crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port-server.py`:

```python
#!/usr/bin/env python3
import http.server
import signal
import sys


signal.signal(signal.SIGUSR1, signal.SIG_IGN)
port = int(sys.argv[1])
with http.server.ThreadingHTTPServer(
    ("127.0.0.1", port), http.server.SimpleHTTPRequestHandler
) as server:
    server.serve_forever()
```

The blocked child intentionally changes only its interpreter source argument from `-c` to the sibling file path. Keep every shell-parent behavior unchanged. The `awk` program uses literal single braces now that it is no longer inside Rust `format!`.

- [ ] **Step 3: Add compile-time sources and exact-once materialization helpers**

At the top of `crates/daemon/tests/gateway_reconciliation.rs`:

```rust
use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
```

Replace the existing single-name imports; do not duplicate them. Add:

```rust
const FAKE_FRANKENPHP_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp.sh"
));
const FAKE_FRANKENPHP_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp-server.py"
));
const FAKE_FRANKENPHP_HANGS_ON_PORT_SCRIPT_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp-hangs-on-port.sh.in"
));
const FAKE_FRANKENPHP_HANGS_ON_PORT_SERVER_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/gateway/fake-frankenphp-hangs-on-port-server.py"
));
const FAKE_FRANKENPHP_BLOCKED_PORT_SENTINEL: &str = "__PV_BLOCKED_PORT__";
```

Replace the two raw-string writers with:

```rust
fn write_fake_frankenphp(path: &Utf8Path) -> Result<()> {
    write_fake_frankenphp_fixture(
        path,
        FAKE_FRANKENPHP_SCRIPT,
        FAKE_FRANKENPHP_SERVER_SCRIPT,
    )
}

fn write_fake_frankenphp_that_hangs_on_port(
    path: &Utf8Path,
    blocked_port: u16,
) -> Result<()> {
    let script = render_fake_frankenphp_hangs_on_port_script(blocked_port)?;

    write_fake_frankenphp_fixture(
        path,
        &script,
        FAKE_FRANKENPHP_HANGS_ON_PORT_SERVER_SCRIPT,
    )
}

fn write_fake_frankenphp_fixture(
    path: &Utf8Path,
    shell_script: &str,
    server_script: &str,
) -> Result<()> {
    let server_path = Utf8PathBuf::from(format!("{path}.server.py"));

    fs::write_sensitive_file(&server_path, server_script)?;
    fs::write_sensitive_file(path, shell_script)?;
    set_executable(path)?;

    Ok(())
}

fn render_fake_frankenphp_hangs_on_port_script(blocked_port: u16) -> Result<String> {
    let occurrence_count = FAKE_FRANKENPHP_HANGS_ON_PORT_SCRIPT_TEMPLATE
        .matches(FAKE_FRANKENPHP_BLOCKED_PORT_SENTINEL)
        .count();
    if occurrence_count != 1 {
        bail!(
            "fake FrankenPHP blocked-port template must contain exactly one {FAKE_FRANKENPHP_BLOCKED_PORT_SENTINEL} sentinel; found {occurrence_count}"
        );
    }

    let replacement = blocked_port.to_string();
    let script = FAKE_FRANKENPHP_HANGS_ON_PORT_SCRIPT_TEMPLATE.replacen(
        FAKE_FRANKENPHP_BLOCKED_PORT_SENTINEL,
        &replacement,
        1,
    );
    if script.contains(FAKE_FRANKENPHP_BLOCKED_PORT_SENTINEL) {
        bail!("fake FrankenPHP blocked-port sentinel remained after rendering");
    }

    Ok(script)
}
```

Writing the non-executable companion first prevents a runnable wrapper from existing without its server input. Append `.server.py` literally; do not use `with_extension`, canonicalize the path, or embed a checkout path. All current callers already pass absolute temporary paths. Keep validators and `shell_single_quoted` inline.

- [ ] **Step 4: Lint rendered shell and Python sources**

Run:

```shell
shellcheck crates/daemon/test-fixtures/gateway/fake-frankenphp.sh
sed 's/__PV_BLOCKED_PORT__/12345/' \
  crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port.sh.in \
  | shellcheck -s sh -
python3 -c 'import ast, pathlib; paths = [pathlib.Path("crates/daemon/test-fixtures/gateway/fake-frankenphp-server.py"), pathlib.Path("crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port-server.py")]; [ast.parse(path.read_text(), filename=str(path)) for path in paths]'
```

Expected: all commands exit 0. The rendered shell contains no sentinel, and Python parsing creates no bytecode.

- [ ] **Step 5: Run the full gateway target**

Run:

```shell
cargo fmt --all
cargo nextest run -p daemon --test gateway_reconciliation --all-features --locked
git diff --check
```

Expected: every gateway reconciliation test passes, including `gateway_reconciliation_starts_gateway_and_one_worker_per_php_track` and `gateway_reconciliation_rolls_back_config_when_runtime_readiness_fails`. The existing gateway snapshots remain byte-for-byte unchanged.

- [ ] **Step 6: Commit the paired gateway fixtures**

```shell
git add crates/daemon/test-fixtures/gateway \
  crates/daemon/tests/gateway_reconciliation.rs
git commit -m "refactor(gateway): extract FrankenPHP test fixtures"
```

---

### Task 5: Extract the supervisor ownership fixture and document Python 3

**Files:**

- Create: `crates/daemon/test-fixtures/supervisor/owned-python-runtime.py`
- Modify: `crates/daemon/tests/supervisor_foundation.rs:1-28,475-500`
- Modify: `CONTRIBUTING.md:19-39`

**Interfaces:**

- Consumes: Existing macOS `supervisor_verifies_owned_python_shebang_script` behavior and materialization helper.
- Produces: A compile-time macOS-only ownership fixture constant and an explicit contributor prerequisite; no production interface changes.

- [ ] **Step 1: Add the direct Python ownership fixture**

Create `crates/daemon/test-fixtures/supervisor/owned-python-runtime.py`:

```python
#!/usr/bin/env python3
import signal
import sys


def stop(_signum, _frame):
    sys.exit(0)


if sys.argv[1:] != ["1025", "8025"]:
    sys.exit(2)

signal.signal(signal.SIGTERM, stop)
signal.pause()
```

Keep the shebang at byte zero and preserve `signal.pause()`, argument validation, and SIGTERM exit exactly.

- [ ] **Step 2: Replace the supervisor test's raw string with compile-time inclusion**

Add near the existing test-only type alias in `crates/daemon/tests/supervisor_foundation.rs`:

```rust
#[cfg(target_os = "macos")]
const OWNED_PYTHON_RUNTIME_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/supervisor/owned-python-runtime.py"
));
```

The `cfg` prevents an unused private constant on non-macOS all-target Clippy runs. In `supervisor_verifies_owned_python_shebang_script`, replace only the raw-string write:

```rust
state::fs::write_sensitive_file(&runtime, OWNED_PYTHON_RUNTIME_SCRIPT)?;
```

Keep the materialized runtime path without a `.py` extension, executable mode, process name, arguments, ownership polling, PID assertion, and stop behavior unchanged.

- [ ] **Step 3: Document the existing Python runtime prerequisite**

Add this paragraph to `CONTRIBUTING.md` immediately after the opening sentence of `## Testing`:

```markdown
Daemon tests that exercise Managed Resource, gateway, and supervisor fixtures require `python3` on `PATH`. These fixtures use only Python's standard library; no Python packages or virtual environment are required.
```

Do not add setup-python, a version pin, dependencies, or CI workflow changes.

- [ ] **Step 4: Verify ownership and source syntax**

Run:

```shell
python3 -c 'import ast, pathlib; path = pathlib.Path("crates/daemon/test-fixtures/supervisor/owned-python-runtime.py"); ast.parse(path.read_text(), filename=str(path))'
cargo fmt --all
cargo nextest run -p daemon --test supervisor_foundation --all-features --locked \
  -E 'test(supervisor_verifies_owned_python_shebang_script)'
git diff --check
```

Expected on macOS: the ownership test passes and the materialized direct shebang process stops cleanly. No new test or snapshot is required because the existing test directly exercises the relocated program.

- [ ] **Step 5: Commit the supervisor fixture and prerequisite documentation**

```shell
git add crates/daemon/test-fixtures/supervisor/owned-python-runtime.py \
  crates/daemon/tests/supervisor_foundation.rs \
  CONTRIBUTING.md
git commit -m "refactor(supervisor): extract Python ownership fixture"
```

---

### Task 6: Run complete fixture lint and workspace verification

**Files:**

- Verify only: all files changed by Tasks 1-5
- Verify only: no changes under `crates/pv-release`, `.github/workflows`, `.config`, or production daemon modules

**Interfaces:**

- Consumes: All five implementation commits and the approved design constraints.
- Produces: Evidence that extracted assets are syntactically valid, behaviorally compatible, warning-free, and isolated to the approved scope.

- [ ] **Step 1: Shellcheck every extracted shell source, rendering the template first**

Run:

```shell
shellcheck \
  crates/daemon/test-fixtures/managed-resources/postgres-initdb.sh \
  crates/daemon/test-fixtures/managed-resources/postgres-unready.sh \
  crates/daemon/test-fixtures/managed-resources/mailpit-unready.sh \
  crates/daemon/test-fixtures/gateway/fake-frankenphp.sh
sed 's/__PV_BLOCKED_PORT__/12345/' \
  crates/daemon/test-fixtures/gateway/fake-frankenphp-hangs-on-port.sh.in \
  | shellcheck -s sh -
```

Expected: both shellcheck invocations exit 0.

- [ ] **Step 2: Parse every Python source and both rendered RustFS variants without bytecode**

Run:

```shell
python3 -c 'import ast, pathlib; root = pathlib.Path("crates/daemon/test-fixtures"); paths = sorted(root.rglob("*.py")); [ast.parse(path.read_text(), filename=str(path)) for path in paths]; print(f"parsed {len(paths)} Python fixtures")'
python3 -c 'import ast, pathlib; path = pathlib.Path("crates/daemon/test-fixtures/managed-resources/rustfs.py.in"); source = path.read_text(); sentinel = "__PV_REJECT_S3__"; assert source.count(sentinel) == 1; [ast.parse(source.replace(sentinel, value), filename=f"{path}:{value}") for value in ("False", "True")]'
```

Expected: the first command reports nine `.py` fixtures, both RustFS variants parse, no assertion fails, and no `__pycache__` is created.

- [ ] **Step 3: Confirm the intended raw strings are gone and inline exceptions remain**

Run:

```shell
if rg -n "fn (mysql_fixture_script|fake_mailpit_script|fake_postgres_initdb_script|fake_postgres_script|unready_fake_postgres_script|unready_fake_mailpit_script|fast_exit_fake_mailpit_script|mailpit_script|redis_server_script)" crates/daemon; then
  exit 1
fi
if rg -n "python3 .*<<'PY'|python3 -c" crates/daemon --glob '*.rs'; then
  exit 1
fi
if rg -n "<<'PY'|python3 -c" crates/daemon/test-fixtures --glob '*.sh' --glob '*.sh.in'; then
  exit 1
fi
rg -n "fn fake_sql_script|fn write_failing_validator|fn write_hanging_frankenphp_validator|const HANGING_FIXTURE" crates/daemon --glob '*.rs'
```

Expected: all three negative searches print nothing; the final search finds the intentionally inline scenario-local fixtures, including the approved `HANGING_FIXTURE` scenario-local Rust string.

- [ ] **Step 4: Run formatting, Clippy, and the complete locked workspace suite**

Run:

```shell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --all-features --locked
```

Expected: formatting and Clippy exit 0, and every non-skipped workspace test passes. The five new fixture-contract tests are included in the total; existing snapshots remain unchanged.

- [ ] **Step 5: Audit scope, modes, sentinels, and final repository state**

Run:

```shell
merge_base="$(git merge-base main HEAD)"
git diff "$merge_base" --check
git diff --stat "$merge_base"
git diff --name-only "$merge_base"
git diff --name-status "$merge_base" -- 'crates/daemon/**/snapshots/*.snap'
find crates/daemon/test-fixtures -type f -exec stat -f '%Sp %N' {} \; | sort
rg -n "__PV_REJECT_S3__|__PV_BLOCKED_PORT__" crates/daemon/test-fixtures crates/daemon/src crates/daemon/tests
git status --short
```

Expected:

- `git diff --check` is clean.
- Changed paths are limited to the approved design/plan documents, `crates/daemon/test-fixtures/`, the four named daemon test files, the new fixture-contract target/snapshots, and `CONTRIBUTING.md`.
- Snapshot status lists exactly five added fixture-contract snapshots and no modified existing snapshot.
- Every checked-in fixture is non-executable (`-rw-r--r--`, mode `100644`).
- Each sentinel appears once in its template and only in the corresponding Rust renderer/contract test where expected.
- `git status --short` prints nothing. If any generated or formatted file remains, inspect and commit it with the task that owns it before reporting completion.
