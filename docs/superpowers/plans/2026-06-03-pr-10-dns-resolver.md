# PR 10 DNS Resolver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement PR 10's internal `.test` DNS resolver plus non-privileged `pv dns:*` prepared-config/status commands.

**Architecture:** Keep DNS runtime behavior inside the daemon and resolver-file behavior inside the `macos` crate. Use `state` for persisted DNS port assignment, `hickory-proto` for DNS wire parsing/encoding, and CLI commands for prepared config plus read-only system status. Do not run `sudo`, mutate `/etc/resolver/test`, or start/register the LaunchAgent in this PR.

**Tech Stack:** Rust 2024, Tokio UDP/TCP, `hickory-proto = "0.26.1"`, `thiserror`, `camino`, `cargo nextest`, `cargo insta`.

---

## File Structure

- Modify `Cargo.toml`: add workspace dependency `hickory-proto = "0.26.1"`.
- Modify `crates/daemon/Cargo.toml`: add `hickory-proto`.
- Modify `crates/macos/Cargo.toml`: add `camino` and `state`.
- Modify `crates/cli/Cargo.toml`: add `macos`.
- Modify `crates/state/src/paths.rs`: add `PvPaths::resolver_config()`.
- Modify `crates/state/src/database.rs`: add `PortRequest::pv_dns()` and DNS port constants.
- Modify `crates/state/src/lib.rs`: export `DNS_PREFERRED_PORT`, `RUNTIME_PORT_FALLBACK_START`, and `RUNTIME_PORT_FALLBACK_END`.
- Modify `crates/state/tests/state_foundation.rs`: add DNS port allocator test.
- Replace `crates/macos/src/lib.rs`: implement resolver config rendering/parsing and read-only resolver-file inspection.
- Create `crates/macos/tests/resolver_config.rs`: test resolver config and file status behavior with injected paths.
- Modify `crates/cli/src/environment.rs`: add a default `resolver_test_path()` method for injected tests.
- Modify `crates/cli/src/args.rs`: add `dns:status`, `dns:install`, `dns:uninstall`.
- Modify `crates/cli/src/commands/mod.rs`: route DNS commands.
- Create `crates/cli/src/commands/dns.rs`: implement command behavior.
- Create `crates/cli/tests/dns.rs`: command-level integration tests with injected home and resolver path.
- Create `crates/daemon/src/dns.rs`: implement DNS protocol response builder, UDP/TCP listeners, and port availability helper.
- Modify `crates/daemon/src/lib.rs`: start and shut down the DNS task with the daemon.
- Modify `crates/daemon/src/error.rs`: add typed DNS protocol/startup errors.
- Modify `crates/daemon/tests/daemon_foundation.rs`: add UDP/TCP DNS integration tests.
- Update generated snapshots under `crates/cli/tests/snapshots/`, `crates/macos/tests/snapshots/`, `crates/state/tests/snapshots/`, and `crates/daemon/tests/snapshots/` only through `cargo insta`.

## Task 1: Dependency, State Path, And DNS Port Helpers

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/daemon/Cargo.toml`
- Modify: `crates/macos/Cargo.toml`
- Modify: `crates/cli/Cargo.toml`
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/state/src/database.rs`
- Modify: `crates/state/src/lib.rs`
- Test: `crates/state/tests/state_foundation.rs`

- [ ] **Step 1: Add the failing DNS port allocator test**

Add this test near the existing port allocator tests in `crates/state/tests/state_foundation.rs`:

```rust
#[test]
fn dns_port_allocator_persists_and_reuses_preferred_assignment() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned_dns = database.assign_port(PortRequest::pv_dns(), |port| port == 35353)?;
    let reused_dns = database.assign_port(PortRequest::pv_dns(), |port| {
        port == assigned_dns.port
    })?;
    let fallback_dns = {
        database.release_port(PortOwner::Dns)?;
        database.assign_port(PortRequest::pv_dns(), |port| port != 35353)?
    };

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            assigned_dns,
            reused_dns,
            fallback_dns,
            database.assigned_ports()?,
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
```

- [ ] **Step 2: Run the focused state test and verify it fails**

Run:

```bash
cargo nextest run -p state -E 'test(dns_port_allocator_persists_and_reuses_preferred_assignment)'
```

Expected: FAIL because `PortRequest::pv_dns()` does not exist.

- [ ] **Step 3: Add dependency declarations**

In root `Cargo.toml`, add under `[workspace.dependencies]`:

```toml
hickory-proto = "0.26.1"
```

In `crates/daemon/Cargo.toml`, add under `[dependencies]`:

```toml
hickory-proto = { workspace = true }
```

In `crates/macos/Cargo.toml`, change dependencies to:

```toml
[dependencies]
camino = { workspace = true }
state = { path = "../state" }
thiserror = { workspace = true }

[dev-dependencies]
anyhow = { workspace = true }
camino-tempfile = { workspace = true }
insta = { workspace = true }
```

In `crates/cli/Cargo.toml`, add:

```toml
macos = { path = "../macos" }
```

- [ ] **Step 4: Add DNS constants and `PortRequest::pv_dns()`**

In `crates/state/src/database.rs`, add near the existing port request code:

```rust
pub const DNS_PREFERRED_PORT: u16 = 35353;
pub const RUNTIME_PORT_FALLBACK_START: u16 = 45000;
pub const RUNTIME_PORT_FALLBACK_END: u16 = 48999;
```

Add this method in `impl PortRequest`:

```rust
pub fn pv_dns() -> Self {
    Self::dns(
        DNS_PREFERRED_PORT,
        RUNTIME_PORT_FALLBACK_START,
        RUNTIME_PORT_FALLBACK_END,
    )
}
```

In `crates/state/src/lib.rs`, export the constants with the existing database exports:

```rust
Database, DatabaseInspection, DNS_PREFERRED_PORT, EnvContextValues, JobRecord, JobStatus,
LinkProjectInput, LinkProjectResult, LinkProjectStatus, ManagedResourceDesiredState,
ManagedResourceTrackRecord, PortAssignment, PortOwner, PortRequest, ProjectConfigWatch,
```

- [ ] **Step 5: Add the prepared resolver config path**

In `crates/state/src/paths.rs`, add this method in `impl PvPaths` near `config()`:

```rust
pub fn resolver_config(&self) -> Utf8PathBuf {
    self.config().join("resolver/test")
}
```

- [ ] **Step 6: Accept the state snapshot and rerun the focused state test**

Run:

```bash
cargo insta test --accept --test-runner nextest -p state -- dns_port_allocator_persists_and_reuses_preferred_assignment
cargo nextest run -p state -E 'test(dns_port_allocator_persists_and_reuses_preferred_assignment)'
```

Expected: PASS. The snapshot should show `PortOwner::Dns`, preferred port `35353`, fallback port `45000`, and persisted assignments with normalized timestamps.

- [ ] **Step 7: Update the lockfile narrowly**

Run:

```bash
cargo update -p hickory-proto --precise 0.26.1
```

Expected: `Cargo.lock` includes `hickory-proto` and its required transitive dependencies. Do not run a broad dependency update.

- [ ] **Step 8: Commit Task 1**

Run:

```bash
git add Cargo.toml Cargo.lock crates/daemon/Cargo.toml crates/macos/Cargo.toml crates/cli/Cargo.toml crates/state/src/paths.rs crates/state/src/database.rs crates/state/src/lib.rs crates/state/tests/state_foundation.rs crates/state/tests/snapshots
git commit -m "feat(state): add DNS resolver port helpers"
```

## Task 2: macOS Resolver Config Model And Read-Only Inspection

**Files:**
- Modify: `crates/macos/src/lib.rs`
- Test: `crates/macos/tests/resolver_config.rs`

- [ ] **Step 1: Write resolver config integration tests**

Create `crates/macos/tests/resolver_config.rs`:

```rust
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use macos::{ResolverConfig, inspect_resolver_file};
use state::fs;

#[test]
fn resolver_config_renders_pv_owned_test_resolver_file() {
    let config = ResolverConfig::new(35353);

    assert_debug_snapshot!(config.render());
}

#[test]
fn resolver_file_inspection_reports_missing_current_stale_conflict_and_unreadable() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current");
    let stale_path = tempdir.path().join("stale");
    let conflict_path = tempdir.path().join("conflict");
    let unreadable_path = tempdir.path().join("directory");
    let expected = ResolverConfig::new(35353);

    fs::write_sensitive_file(&current_path, &expected.render())?;
    fs::write_sensitive_file(&stale_path, &ResolverConfig::new(45000).render())?;
    fs::write_sensitive_file(&conflict_path, "nameserver 127.0.0.1\nport 35353\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_resolver_file(&tempdir.path().join("missing"), Some(&expected)),
        inspect_resolver_file(&current_path, Some(&expected)),
        inspect_resolver_file(&stale_path, Some(&expected)),
        inspect_resolver_file(&conflict_path, Some(&expected)),
        inspect_resolver_file(&unreadable_path, Some(&expected)),
    ];

    assert_debug_snapshot!(states);

    Ok(())
}
```

- [ ] **Step 2: Run the macOS tests and verify they fail**

Run:

```bash
cargo nextest run -p macos -E 'test(resolver_)'
```

Expected: FAIL because `ResolverConfig` and `inspect_resolver_file` do not exist.

- [ ] **Step 3: Implement resolver config and inspection**

Replace `crates/macos/src/lib.rs` with:

```rust
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

pub const SYSTEM_RESOLVER_TEST_PATH: &str = "/etc/resolver/test";
const PV_MARKER: &str = "# Managed by PV";
const PREPARED_MARKER: &str = "# Source: PV prepared resolver config for /etc/resolver/test";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolverConfig {
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolverFileState {
    Missing { path: Utf8PathBuf },
    Current { path: Utf8PathBuf, port: u16 },
    Stale {
        path: Utf8PathBuf,
        expected_port: u16,
        actual_port: Option<u16>,
    },
    Conflict { path: Utf8PathBuf },
    Unreadable { path: Utf8PathBuf, message: String },
}

#[derive(Debug, Error)]
#[error("macOS integration error: {message}")]
pub struct MacosError {
    message: String,
}

impl ResolverConfig {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn render(&self) -> String {
        format!(
            "{PV_MARKER}\n{PREPARED_MARKER}\nnameserver 127.0.0.1\nport {}\n",
            self.port
        )
    }

    pub fn parse(content: &str) -> Option<Self> {
        let mut port = None;
        for line in content.lines().map(str::trim) {
            let Some(value) = line.strip_prefix("port ") else {
                continue;
            };
            port = value.parse::<u16>().ok();
        }

        port.map(Self::new)
    }
}

pub fn inspect_resolver_file(
    path: &Utf8Path,
    expected: Option<&ResolverConfig>,
) -> ResolverFileState {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return ResolverFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return ResolverFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) {
        return ResolverFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = ResolverConfig::parse(&content);
    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => ResolverFileState::Current {
            path: path.to_path_buf(),
            port: actual.port,
        },
        (Some(expected), actual) => ResolverFileState::Stale {
            path: path.to_path_buf(),
            expected_port: expected.port,
            actual_port: actual.map(|config| config.port),
        },
        (None, Some(actual)) => ResolverFileState::Current {
            path: path.to_path_buf(),
            port: actual.port,
        },
        (None, None) => ResolverFileState::Stale {
            path: path.to_path_buf(),
            expected_port: 0,
            actual_port: None,
        },
    }
}
```

- [ ] **Step 4: Accept snapshots and rerun macOS tests**

Run:

```bash
cargo insta test --accept --test-runner nextest -p macos -- resolver_
cargo nextest run -p macos -E 'test(resolver_)'
```

Expected: PASS. Snapshots should show rendered resolver file content and each resolver file state.

- [ ] **Step 5: Commit Task 2**

Run:

```bash
git add crates/macos/src/lib.rs crates/macos/tests/resolver_config.rs crates/macos/tests/snapshots
git commit -m "feat(macos): inspect PV resolver config"
```

## Task 3: CLI DNS Command Tests And Routing

**Files:**
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Create: `crates/cli/src/commands/dns.rs`
- Test: `crates/cli/tests/dns.rs`

- [ ] **Step 1: Add CLI DNS integration tests**

Create `crates/cli/tests/dns.rs`:

```rust
use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use macos::ResolverConfig;
use state::fs;

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    resolver_test_path: PathBuf,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, resolver_test_path: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            resolver_test_path: resolver_test_path.as_std_path().to_path_buf(),
        }
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.borrow().clone())
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn resolver_test_path(&self) -> PathBuf {
        self.resolver_test_path.clone()
    }
}

#[derive(Debug)]
struct CommandOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<CommandOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = run_with_environment(["pv"].into_iter().chain(args.iter().copied()), environment, &mut stdout, &mut stderr)?;

    Ok(CommandOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

#[test]
fn dns_install_prepares_config_and_reports_deferred_privileged_install() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let resolver_path = tempdir.path().join("etc").join("resolver").join("test");
    let environment = TestEnvironment::new(&home, tempdir.path(), &resolver_path);

    let output = run_pv(&["dns:install"], &environment)?;
    let prepared = state::testing::read_to_string(&state::PvPaths::for_home(&home).resolver_config())?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((output, prepared));
    });

    Ok(())
}

#[test]
fn dns_status_reports_prepared_and_system_resolver_states() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let resolver_path = tempdir.path().join("etc").join("resolver").join("test");
    let paths = state::PvPaths::for_home(&home);
    let environment = TestEnvironment::new(&home, tempdir.path(), &resolver_path);

    let missing = run_pv(&["dns:status"], &environment)?;
    fs::write_sensitive_file(&paths.resolver_config(), &ResolverConfig::new(35353).render())?;
    let prepared_only = run_pv(&["dns:status"], &environment)?;
    fs::write_sensitive_file(&resolver_path, &ResolverConfig::new(35353).render())?;
    let current = run_pv(&["dns:status"], &environment)?;
    fs::write_sensitive_file(&resolver_path, &ResolverConfig::new(45000).render())?;
    let stale = run_pv(&["dns:status"], &environment)?;
    fs::write_sensitive_file(&resolver_path, "nameserver 127.0.0.1\nport 35353\n")?;
    let conflict = run_pv(&["dns:status"], &environment)?;

    assert_eq!(missing.exit_code, ExitCode::SUCCESS);
    assert_eq!(prepared_only.exit_code, ExitCode::SUCCESS);
    assert_eq!(current.exit_code, ExitCode::SUCCESS);
    assert_eq!(stale.exit_code, ExitCode::SUCCESS);
    assert_eq!(conflict.exit_code, ExitCode::SUCCESS);
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((missing, prepared_only, current, stale, conflict));
    });

    Ok(())
}

#[test]
fn dns_uninstall_removes_prepared_config_without_touching_system_resolver() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let resolver_path = tempdir.path().join("etc").join("resolver").join("test");
    let paths = state::PvPaths::for_home(&home);
    let environment = TestEnvironment::new(&home, tempdir.path(), &resolver_path);
    fs::write_sensitive_file(&paths.resolver_config(), &ResolverConfig::new(35353).render())?;
    fs::write_sensitive_file(&resolver_path, &ResolverConfig::new(35353).render())?;

    let output = run_pv(&["dns:uninstall"], &environment)?;
    let prepared_after = state::testing::read_to_string(&paths.resolver_config()).err().map(|error| error.to_string());
    let system_after = state::testing::read_to_string(&resolver_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((output, prepared_after, system_after));
    });

    Ok(())
}
```

- [ ] **Step 2: Run the CLI DNS tests and verify they fail**

Run:

```bash
cargo nextest run -p cli -E 'test(dns_)'
```

Expected: FAIL because the DNS command routing and `Environment::resolver_test_path()` do not exist.

- [ ] **Step 3: Add resolver path injection to the CLI environment**

In `crates/cli/src/environment.rs`, add a default method to the `Environment` trait:

```rust
fn resolver_test_path(&self) -> PathBuf {
    PathBuf::from(macos::SYSTEM_RESOLVER_TEST_PATH)
}
```

No change is required in `ProcessEnvironment` because the trait default returns `/etc/resolver/test`.

- [ ] **Step 4: Add DNS command routing**

In `crates/cli/src/args.rs`, add variants to `Command`:

```rust
#[command(name = "dns:status", about = "Show PV .test resolver status")]
DnsStatus,

#[command(name = "dns:install", about = "Prepare PV .test resolver configuration")]
DnsInstall,

#[command(name = "dns:uninstall", about = "Remove prepared PV .test resolver configuration")]
DnsUninstall,
```

In `crates/cli/src/commands/mod.rs`, add:

```rust
mod dns;
```

and route the variants:

```rust
Command::DnsStatus => dns::status(environment, stdout),
Command::DnsInstall => dns::install(environment, stdout),
Command::DnsUninstall => dns::uninstall(environment, stdout),
```

- [ ] **Step 5: Add a compiling DNS command module stub**

Create `crates/cli/src/commands/dns.rs`:

```rust
use std::io::Write;
use std::process::ExitCode;

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

pub(crate) fn status(
    _environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("PV DNS resolver status is not implemented")?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    _environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("PV DNS resolver installation is not implemented")?;

    Ok(ExitCode::FAILURE)
}

pub(crate) fn uninstall(
    _environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("PV DNS resolver uninstall is not implemented")?;

    Ok(ExitCode::FAILURE)
}
```

- [ ] **Step 6: Rerun CLI DNS tests and verify they still fail for behavior**

Run:

```bash
cargo nextest run -p cli -E 'test(dns_)'
```

Expected: FAIL because snapshots/behavior expect real prepared config and status output, while the module still returns stubs.

## Task 4: CLI DNS Command Implementation

**Files:**
- Modify: `crates/cli/src/commands/dns.rs`
- Test: `crates/cli/tests/dns.rs`

- [ ] **Step 1: Implement CLI DNS behavior**

Replace `crates/cli/src/commands/dns.rs` with:

```rust
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use macos::{ResolverConfig, ResolverFileState};
use state::{Database, PortRequest, PvPaths, StateError};

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn status(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_path = paths.resolver_config();
    let prepared_config = read_prepared_config(&prepared_path)?;
    let prepared_state = macos::inspect_resolver_file(&prepared_path, prepared_config.as_ref());
    let system_path = resolver_test_path(environment)?;
    let system_state = macos::inspect_resolver_file(&system_path, prepared_config.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("PV DNS resolver")?;
    output.line(&format!("prepared: {}", describe_state(&prepared_state)))?;
    output.line(&format!("system: {}", describe_state(&system_state)))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut database = Database::open(&paths)?;
    let assignment = database.assign_port(PortRequest::pv_dns(), daemon::dns_port_available)?;
    let config = ResolverConfig::new(assignment.port);

    state::fs::write_sensitive_file(&paths.resolver_config(), &config.render())?;

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!(
        "Prepared PV .test resolver config at {}",
        paths.resolver_config()
    ))?;
    output.line(&format!("DNS port: {}", assignment.port))?;
    output.line(
        "Privileged installation into /etc/resolver/test is deferred to pv setup/system integration work.",
    )?;

    Ok(ExitCode::FAILURE)
}

pub(crate) fn uninstall(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    remove_prepared_config(&paths.resolver_config())?;
    let system_path = resolver_test_path(environment)?;
    let system_state = macos::inspect_resolver_file(&system_path, None);
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Removed prepared PV .test resolver config at {}",
        paths.resolver_config()
    ))?;

    match system_state {
        ResolverFileState::Current { .. } | ResolverFileState::Stale { .. } => {
            output.line(
                "A PV-owned /etc/resolver/test file still requires privileged removal, deferred to setup/system integration work.",
            )?;
            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Conflict { .. } => {
            output.line("A non-PV-owned /etc/resolver/test file exists; PV left it unchanged.")?;
            Ok(ExitCode::FAILURE)
        }
        ResolverFileState::Missing { .. } | ResolverFileState::Unreadable { .. } => {
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn read_prepared_config(path: &Utf8Path) -> Result<Option<ResolverConfig>, ExecuteError> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(ResolverConfig::parse(&content)),
        Err(error) => {
            if let StateError::Filesystem { source, .. } = &error
                && source.kind() == io::ErrorKind::NotFound
            {
                return Ok(None);
            }

            Err(error.into())
        }
    }
}

fn remove_prepared_config(path: &Utf8Path) -> Result<(), ExecuteError> {
    match state::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) => {
            if let StateError::Filesystem { source, .. } = &error
                && source.kind() == io::ErrorKind::NotFound
            {
                return Ok(());
            }

            Err(error.into())
        }
    }
}

fn describe_state(state: &ResolverFileState) -> String {
    match state {
        ResolverFileState::Missing { path } => format!("missing ({path})"),
        ResolverFileState::Current { path, port } => {
            format!("PV-owned current ({path}, port {port})")
        }
        ResolverFileState::Stale {
            path,
            expected_port,
            actual_port,
        } => format!(
            "PV-owned stale ({path}, expected port {expected_port}, found {})",
            actual_port
                .map(|port| port.to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
        ResolverFileState::Conflict { path } => format!("non-PV-owned conflict ({path})"),
        ResolverFileState::Unreadable { path, message } => {
            format!("unreadable ({path}: {message})")
        }
    }
}

fn resolver_test_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.resolver_test_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
```

- [ ] **Step 2: Expose `state::fs::remove_file` for the CLI command**

Change `crates/state/src/fs.rs` from:

```rust
pub(crate) fn remove_file(path: &Utf8Path) -> Result<(), StateError> {
```

to:

```rust
pub fn remove_file(path: &Utf8Path) -> Result<(), StateError> {
```

- [ ] **Step 3: Accept CLI snapshots and rerun CLI DNS tests**

Run:

```bash
cargo insta test --accept --test-runner nextest -p cli -- dns_
cargo nextest run -p cli -E 'test(dns_)'
```

Expected: PASS. Snapshots must show non-zero `dns:install`, read-only status states, and non-privileged uninstall behavior.

- [ ] **Step 4: Commit Tasks 3 and 4**

Run:

```bash
git add crates/cli/src/environment.rs crates/cli/src/args.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/dns.rs crates/cli/tests/dns.rs crates/cli/tests/snapshots crates/state/src/fs.rs
git commit -m "feat(cli): add DNS resolver commands"
```

## Task 5: Daemon DNS Protocol Module

**Files:**
- Create: `crates/daemon/src/dns.rs`
- Modify: `crates/daemon/src/error.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: module tests inside `crates/daemon/src/dns.rs`

- [ ] **Step 1: Create failing protocol tests inside `crates/daemon/src/dns.rs`**

Create `crates/daemon/src/dns.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use hickory_proto::op::{Message, MessageType, Query, ResponseCode};
    use hickory_proto::rr::{DNSClass, Name, RData, RecordType};

    use super::response_bytes;

    fn query(name: &str, record_type: RecordType) -> anyhow::Result<Vec<u8>> {
        let mut message = Message::new();
        message
            .set_id(42)
            .set_message_type(MessageType::Query)
            .add_query({
                let mut query = Query::query(Name::from_ascii(name)?, record_type);
                query.set_query_class(DNSClass::IN);
                query
            });

        Ok(message.to_bytes()?)
    }

    #[test]
    fn builds_a_and_aaaa_loopback_answers_for_test_names() -> anyhow::Result<()> {
        let a = Message::from_vec(&response_bytes(&query("acme.test.", RecordType::A)?)?)?;
        let aaaa = Message::from_vec(&response_bytes(&query("acme.test.", RecordType::AAAA)?)?)?;

        assert_eq!(a.id(), 42);
        assert_eq!(a.response_code(), ResponseCode::NoError);
        assert_eq!(a.answers().len(), 1);
        assert_eq!(a.answers()[0].ttl(), 5);
        assert!(matches!(a.answers()[0].data(), Some(RData::A(address)) if address.0.octets() == [127, 0, 0, 1]));
        assert_eq!(aaaa.answers().len(), 1);
        assert!(matches!(aaaa.answers()[0].data(), Some(RData::AAAA(address)) if address.0.is_loopback()));

        Ok(())
    }

    #[test]
    fn returns_nodata_for_unsupported_or_non_test_queries() -> anyhow::Result<()> {
        let mx = Message::from_vec(&response_bytes(&query("acme.test.", RecordType::MX)?)?)?;
        let external = Message::from_vec(&response_bytes(&query("example.com.", RecordType::A)?)?)?;

        assert_eq!(mx.response_code(), ResponseCode::NoError);
        assert!(mx.answers().is_empty());
        assert_eq!(external.response_code(), ResponseCode::NoError);
        assert!(external.answers().is_empty());

        Ok(())
    }
}
```

- [ ] **Step 2: Run daemon DNS module tests and verify they fail**

Run:

```bash
cargo nextest run -p daemon -E 'test(builds_a_and_aaaa_loopback_answers_for_test_names) or test(returns_nodata_for_unsupported_or_non_test_queries)'
```

Expected: FAIL because `response_bytes` and DNS implementation do not exist.

- [ ] **Step 3: Add DNS errors**

In `crates/daemon/src/error.rs`, add:

```rust
#[error("DNS protocol error: {0}")]
DnsProtocol(#[from] hickory_proto::error::ProtoError),

#[error("DNS listener failed to bind UDP and TCP on port {port}: {message}")]
DnsBind { port: u16, message: String },
```

- [ ] **Step 4: Implement protocol response and port availability**

In `crates/daemon/src/dns.rs`, add above the tests:

```rust
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use hickory_proto::op::{Message, MessageType, ResponseCode};
use hickory_proto::rr::{DNSClass, RData, Record, RecordType};
use state::{Database, PortAssignment, PortRequest, PvPaths};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::DaemonError;

pub const DNS_TTL_SECONDS: u32 = 5;

#[derive(Debug)]
pub(crate) struct RunningDnsResolver {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), DaemonError>>,
}

pub fn dns_port_available(port: u16) -> bool {
    let udp = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, port));
    let tcp = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port));

    udp.is_ok() && tcp.is_ok()
}

pub fn response_bytes(request: &[u8]) -> Result<Vec<u8>, DaemonError> {
    let request = Message::from_vec(request)?;
    let mut response = Message::new();
    response
        .set_id(request.id())
        .set_message_type(MessageType::Response)
        .set_op_code(request.op_code())
        .set_authoritative(true)
        .set_recursion_desired(request.recursion_desired())
        .set_recursion_available(false)
        .set_response_code(ResponseCode::NoError);

    for query in request.queries() {
        response.add_query(query.clone());
        if !is_test_name(&query.name().to_ascii()) {
            continue;
        }

        match query.query_type() {
            RecordType::A => {
                response.add_answer(
                    Record::from_rdata(
                        query.name().clone(),
                        DNS_TTL_SECONDS,
                        RData::A(Ipv4Addr::LOCALHOST.into()),
                    )
                    .set_dns_class(DNSClass::IN)
                    .clone(),
                );
            }
            RecordType::AAAA => {
                response.add_answer(
                    Record::from_rdata(
                        query.name().clone(),
                        DNS_TTL_SECONDS,
                        RData::AAAA(Ipv6Addr::LOCALHOST.into()),
                    )
                    .set_dns_class(DNSClass::IN)
                    .clone(),
                );
            }
            _ => {}
        }
    }

    Ok(response.to_bytes()?)
}

fn is_test_name(name: &str) -> bool {
    let normalized = name.trim_end_matches('.').to_ascii_lowercase();
    normalized == "test" || normalized.ends_with(".test")
}
```

- [ ] **Step 5: Rerun DNS module tests**

Run:

```bash
cargo nextest run -p daemon -E 'test(builds_a_and_aaaa_loopback_answers_for_test_names) or test(returns_nodata_for_unsupported_or_non_test_queries)'
```

Expected: PASS.

- [ ] **Step 6: Wire module export for CLI availability helper**

In `crates/daemon/src/lib.rs`, add:

```rust
mod dns;
pub use dns::dns_port_available;
```

- [ ] **Step 7: Commit Task 5**

Run:

```bash
git add crates/daemon/src/dns.rs crates/daemon/src/error.rs crates/daemon/src/lib.rs
git commit -m "feat(daemon): add DNS response protocol"
```

## Task 6: Daemon DNS Runtime Integration And Network Tests

**Files:**
- Modify: `crates/daemon/src/dns.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/tests/daemon_foundation.rs`

- [ ] **Step 1: Add daemon DNS network tests**

Add these helpers near the daemon foundation test helpers in `crates/daemon/tests/daemon_foundation.rs`:

```rust
use hickory_proto::op::{Message, MessageType, Query, ResponseCode};
use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
use state::PortOwner;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

fn dns_query(name: &str, record_type: RecordType) -> Result<Vec<u8>> {
    let mut message = Message::new();
    message
        .set_id(7)
        .set_message_type(MessageType::Query)
        .add_query({
            let mut query = Query::query(Name::from_ascii(name)?, record_type);
            query.set_query_class(DNSClass::IN);
            query
        });

    Ok(message.to_bytes()?)
}

fn dns_port(paths: &PvPaths) -> Result<u16> {
    let database = Database::open(paths)?;
    database
        .assigned_ports()?
        .into_iter()
        .find_map(|assignment| match assignment.owner {
            PortOwner::Dns => Some(assignment.port),
            _ => None,
        })
        .ok_or_else(|| anyhow!("missing DNS port assignment"))
}

async fn udp_dns_query(port: u16, query: &[u8]) -> Result<Message> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.send_to(query, ("127.0.0.1", port)).await?;
    let mut buffer = [0_u8; 512];
    let (length, _peer) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer)).await??;

    Ok(Message::from_vec(&buffer[..length])?)
}

async fn tcp_dns_query(port: u16, query: &[u8]) -> Result<Message> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let length = u16::try_from(query.len())?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(query).await?;
    let mut prefix = [0_u8; 2];
    timeout(Duration::from_secs(2), stream.read_exact(&mut prefix)).await??;
    let response_length = u16::from_be_bytes(prefix) as usize;
    let mut response = vec![0_u8; response_length];
    timeout(Duration::from_secs(2), stream.read_exact(&mut response)).await??;

    Ok(Message::from_vec(&response)?)
}
```

Add tests:

```rust
#[tokio::test]
async fn dns_resolver_answers_udp_a_and_aaaa_for_test_hostnames() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    let a = udp_dns_query(port, &dns_query("acme.test.", RecordType::A)?).await?;
    let aaaa = udp_dns_query(port, &dns_query("acme.test.", RecordType::AAAA)?).await?;

    daemon.shutdown().await?;

    assert_eq!(a.response_code(), ResponseCode::NoError);
    assert!(matches!(a.answers()[0].data(), Some(RData::A(address)) if address.0.octets() == [127, 0, 0, 1]));
    assert_eq!(a.answers()[0].ttl(), 5);
    assert!(matches!(aaaa.answers()[0].data(), Some(RData::AAAA(address)) if address.0.is_loopback()));
    assert_eq!(aaaa.answers()[0].ttl(), 5);

    Ok(())
}

#[tokio::test]
async fn dns_resolver_returns_nodata_and_survives_malformed_udp() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;
    let socket = UdpSocket::bind("127.0.0.1:0").await?;

    socket.send_to(b"not dns", ("127.0.0.1", port)).await?;
    let mx = udp_dns_query(port, &dns_query("acme.test.", RecordType::MX)?).await?;
    let external = udp_dns_query(port, &dns_query("example.com.", RecordType::A)?).await?;

    daemon.shutdown().await?;

    assert_eq!(mx.response_code(), ResponseCode::NoError);
    assert!(mx.answers().is_empty());
    assert_eq!(external.response_code(), ResponseCode::NoError);
    assert!(external.answers().is_empty());

    Ok(())
}

#[tokio::test]
async fn dns_resolver_answers_tcp_queries() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    let response = tcp_dns_query(port, &dns_query("acme.test.", RecordType::A)?).await?;

    daemon.shutdown().await?;

    assert_eq!(response.response_code(), ResponseCode::NoError);
    assert!(matches!(response.answers()[0].data(), Some(RData::A(address)) if address.0.octets() == [127, 0, 0, 1]));

    Ok(())
}

#[tokio::test]
async fn dns_resolver_falls_back_when_preferred_port_is_unavailable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let _preferred_udp = std::net::UdpSocket::bind(("127.0.0.1", state::DNS_PREFERRED_PORT))?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let port = dns_port(&paths)?;

    daemon.shutdown().await?;

    assert_ne!(port, state::DNS_PREFERRED_PORT);
    assert!((45000..=48999).contains(&port));

    Ok(())
}
```

- [ ] **Step 2: Run daemon DNS network tests and verify they fail**

Run:

```bash
cargo nextest run -p daemon -E 'test(dns_resolver_)'
```

Expected: FAIL because the daemon has not started the DNS runtime listeners.

- [ ] **Step 3: Implement DNS runtime listener start and shutdown**

Extend `crates/daemon/src/dns.rs` with:

```rust
impl RunningDnsResolver {
    pub async fn start(paths: PvPaths) -> Result<Self, DaemonError> {
        let assignment = assign_dns_port(&paths)?;
        let udp = UdpSocket::bind(("127.0.0.1", assignment.port))
            .await
            .map_err(|source| DaemonError::DnsBind {
                port: assignment.port,
                message: source.to_string(),
            })?;
        let tcp = TcpListener::bind(("127.0.0.1", assignment.port))
            .await
            .map_err(|source| DaemonError::DnsBind {
                port: assignment.port,
                message: source.to_string(),
            })?;
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(run(udp, tcp, shutdown_receiver));

        Ok(Self { shutdown, task })
    }

    pub async fn shutdown(self) -> Result<(), DaemonError> {
        let _ = self.shutdown.send(());
        self.task.await?
    }
}

fn assign_dns_port(paths: &PvPaths) -> Result<PortAssignment, DaemonError> {
    let mut database = Database::open(paths)?;
    Ok(database.assign_port(PortRequest::pv_dns(), dns_port_available)?)
}

async fn run(
    udp: UdpSocket,
    tcp: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), DaemonError> {
    let mut tcp_connections = tokio::task::JoinSet::new();
    let mut udp_buffer = [0_u8; 512];

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tcp_connections.abort_all();
                while tcp_connections.join_next().await.is_some() {}
                return Ok(());
            }
            received = udp.recv_from(&mut udp_buffer) => {
                let (length, peer) = received?;
                handle_udp(&udp, peer, &udp_buffer[..length]).await?;
            }
            accepted = tcp.accept() => {
                let (stream, _peer) = accepted?;
                tcp_connections.spawn(handle_tcp(stream));
            }
            joined = tcp_connections.join_next(), if !tcp_connections.is_empty() => {
                match joined {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(_error))) => {}
                    Some(Err(error)) if error.is_panic() => return Err(error.into()),
                    Some(Err(_error)) => {}
                }
            }
        }
    }
}

async fn handle_udp(socket: &UdpSocket, peer: SocketAddr, request: &[u8]) -> Result<(), DaemonError> {
    let Ok(response) = response_bytes(request) else {
        return Ok(());
    };
    socket.send_to(&response, peer).await?;

    Ok(())
}

async fn handle_tcp(mut stream: tokio::net::TcpStream) -> Result<(), DaemonError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut prefix = [0_u8; 2];
    stream.read_exact(&mut prefix).await?;
    let length = u16::from_be_bytes(prefix) as usize;
    let mut request = vec![0_u8; length];
    stream.read_exact(&mut request).await?;
    let response = response_bytes(&request)?;
    let response_length = u16::try_from(response.len()).map_err(|error| {
        DaemonError::Io(io::Error::new(io::ErrorKind::InvalidData, error))
    })?;
    stream.write_all(&response_length.to_be_bytes()).await?;
    stream.write_all(&response).await?;

    Ok(())
}
```

- [ ] **Step 4: Start DNS with the daemon**

In `crates/daemon/src/lib.rs`, change the module declaration/export:

```rust
mod dns;
pub use dns::dns_port_available;
```

Extend `RunningDaemon`:

```rust
dns: dns::RunningDnsResolver,
```

In `RunningDaemon::start`, after binding the IPC listener and before returning:

```rust
let dns = dns::RunningDnsResolver::start(paths.clone()).await?;
```

Return it in `Self`.

In `RunningDaemon::shutdown`, before awaiting/removing the IPC task, shut DNS down:

```rust
let dns_result = self.dns.shutdown().await;
```

Then preserve existing socket cleanup and return `dns_result?` before `task_result?`.

In `wait_for_shutdown`, destructure `dns` and include a `dns.shutdown().await?` call in both shutdown branches before returning.

- [ ] **Step 5: Rerun daemon DNS tests**

Run:

```bash
cargo nextest run -p daemon -E 'test(dns_resolver_)'
```

Expected: PASS.

- [ ] **Step 6: Commit Task 6**

Run:

```bash
git add crates/daemon/src/dns.rs crates/daemon/src/lib.rs crates/daemon/tests/daemon_foundation.rs
git commit -m "feat(daemon): run internal DNS resolver"
```

## Task 7: Focused Verification And Snapshot Hygiene

**Files:**
- Modify only generated snapshots produced by focused tests.

- [ ] **Step 1: Run all focused PR 10 tests**

Run:

```bash
cargo nextest run -p state -E 'test(dns_port_allocator_persists_and_reuses_preferred_assignment)'
cargo nextest run -p macos -E 'test(resolver_)'
cargo nextest run -p cli -E 'test(dns_)'
cargo nextest run -p daemon -E 'test(dns_)'
```

Expected: all selected tests PASS.

- [ ] **Step 2: Check snapshots without hand-editing**

Run:

```bash
cargo insta test --test-runner nextest -p state -- dns_port_allocator_persists_and_reuses_preferred_assignment
cargo insta test --test-runner nextest -p macos -- resolver_
cargo insta test --test-runner nextest -p cli -- dns_
cargo insta test --test-runner nextest -p daemon -- dns_
```

Expected: no `.snap.new` files remain. If new snapshots appear, inspect them, then accept with the same focused `cargo insta test --accept --test-runner nextest -p <crate> -- <filter>` command.

- [ ] **Step 3: Run formatting checks for touched Rust crates**

Run:

```bash
cargo fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 4: Run targeted clippy**

Run:

```bash
cargo clippy -p state -p macos -p cli -p daemon --all-targets --all-features --locked -- -D warnings
```

Expected: PASS. If clippy reports a warning, fix it; do not assume it is pre-existing.

- [ ] **Step 5: Run diff hygiene**

Run:

```bash
git diff --check
```

Expected: PASS.

- [ ] **Step 6: Commit final verification-only changes if any**

If verification generated only snapshot updates or formatting changes after the last commit, run:

```bash
git add crates/state/tests/snapshots crates/macos/tests/snapshots crates/cli/tests/snapshots crates/daemon/tests/snapshots
git commit -m "test: update DNS resolver snapshots"
```

If no files changed, do not create an empty commit.

## Self-Review Checklist

- Spec coverage: Tasks 1 and 6 cover persisted preferred/fallback DNS port assignment. Tasks 2 and 4 cover prepared resolver config, read-only system inspection, no `sudo`, and no real `/etc/resolver/test` mutation. Tasks 5 and 6 cover UDP/TCP DNS, `.test` A/AAAA loopback answers, NODATA, malformed packets, TTL 5, and daemon startup/shutdown. Task 7 covers focused verification.
- Placeholder scan: no red-flag placeholder steps should remain in this plan.
- Type consistency: `ResolverConfig`, `ResolverFileState`, `PortRequest::pv_dns()`, `PvPaths::resolver_config()`, `daemon::dns_port_available`, and `Environment::resolver_test_path()` are introduced before later tasks use them.
