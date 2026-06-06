# PR 14 PHP/FrankenPHP Gateway Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build PR 14 by adding PHP/FrankenPHP runtime adapter validation, generated Gateway and PHP-track worker config, daemon-supervised runtime reconciliation, and an opt-in real-artifact Project-serving E2E.

**Architecture:** `resources` validates PHP and FrankenPHP runtime artifact layouts. `state` persists per-PHP-track worker ports and runtime observed status using existing tables. `daemon` renders Gateway/worker Caddyfiles, validates them with the managed FrankenPHP/Caddy binary, promotes configs atomically, supervises Gateway and one worker per PHP track, and keeps always-run tests on fake fixtures while real runtime serving is gated by explicit E2E environment variables.

**Tech Stack:** Rust, Tokio process supervision, rusqlite-backed state, FrankenPHP/Caddy Caddyfile config, `insta` snapshots, `cargo nextest`, local fake `.tar.gz` fixtures, and opt-in manifest-driven real artifacts.

---

## References

- Approved spec: `docs/superpowers/specs/2026-06-06-pr-14-php-frankenphp-gateway-design.md`
- Roadmap rows: `IMPLEMENTATION.md` PR 14, PV-070 through PV-074 and PV-078.
- FrankenPHP config docs: https://frankenphp.dev/docs/config/
- Caddy command docs for `reload`, `validate`, and signals: https://caddyserver.com/docs/command-line
- Caddy TLS and PKI docs for internal CA configuration: https://caddyserver.com/docs/caddyfile/directives/tls and https://caddyserver.com/docs/caddyfile/options

## File Structure

- Create `crates/resources/src/runtime.rs`: PHP and FrankenPHP runtime artifact adapters.
- Modify `crates/resources/src/lib.rs`: export runtime adapters.
- Create `crates/resources/tests/runtime_adapters.rs`: adapter layout validation tests.
- Modify `crates/state/src/paths.rs`: Gateway/worker config, pid, metadata, and log path helpers.
- Modify `crates/state/src/database.rs`: replace project-specific worker port ownership with per-PHP-track worker ownership and add runtime observed-state APIs.
- Modify `crates/state/src/lib.rs`: export new worker/runtime observed-state types.
- Modify `crates/state/tests/state_foundation.rs`: focused path, port, and runtime observed-state tests.
- Create `crates/daemon/src/gateway_config.rs`: pure Caddyfile config renderer and atomic promotion helpers.
- Create `crates/daemon/src/gateway.rs`: runtime plan building and supervised Gateway/worker reconciliation.
- Modify `crates/daemon/src/lib.rs`: wire new modules and exports for tests.
- Modify `crates/daemon/src/jobs.rs`: run Gateway/worker reconciliation for `system`, Project, and PHP/FrankenPHP resource scopes.
- Create `crates/daemon/tests/gateway_config.rs`: config rendering snapshots.
- Create `crates/daemon/tests/gateway_reconciliation.rs`: fake-artifact runtime reconciliation tests.
- Create `crates/daemon/tests/real_artifact_gateway_e2e.rs`: ignored/opt-in real-artifact E2E.
- Create `.github/workflows/real-artifact-e2e.yml`: manual workflow that runs the ignored E2E with a supplied manifest URL.
- Modify `IMPLEMENTATION.md`: after opening the PR, mark PR 14 with the actual GitHub PR number.

## Task 1: PHP And FrankenPHP Runtime Adapters

**Files:**
- Create: `crates/resources/src/runtime.rs`
- Modify: `crates/resources/src/lib.rs`
- Test: `crates/resources/tests/runtime_adapters.rs`

- [ ] **Step 1: Write failing adapter tests**

Create `crates/resources/tests/runtime_adapters.rs`:

```rust
use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use resources::{ResourcesError, frankenphp_adapter, php_adapter};

#[test]
fn php_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path().join("php-release");
    state::fs::write_sensitive_file(&release.join("bin/php"), "#!/bin/sh\n")?;

    let adapter = php_adapter()?;

    adapter.validate_installation(&release)?;
    assert_eq!(
        adapter.executable_path(&release).as_str(),
        release.join("bin/php").as_str()
    );

    Ok(())
}

#[test]
fn frankenphp_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path().join("frankenphp-release");
    state::fs::write_sensitive_file(&release.join("bin/frankenphp"), "#!/bin/sh\n")?;

    let adapter = frankenphp_adapter()?;

    adapter.validate_installation(&release)?;
    assert_eq!(
        adapter.executable_path(&release).as_str(),
        release.join("bin/frankenphp").as_str()
    );

    Ok(())
}

#[test]
fn runtime_adapters_reject_missing_executables() -> Result<()> {
    let tempdir = tempdir()?;
    let php_release = tempdir.path().join("php-release");
    let frankenphp_release = tempdir.path().join("frankenphp-release");
    state::fs::write_sensitive_file(&php_release.join("README.md"), "missing php")?;
    state::fs::write_sensitive_file(&frankenphp_release.join("README.md"), "missing frankenphp")?;

    let php_result = php_adapter()?.validate_installation(&php_release);
    let frankenphp_result = frankenphp_adapter()?.validate_installation(&frankenphp_release);

    assert!(matches!(
        php_result,
        Err(ResourcesError::InvalidArtifactLayout { resource, .. }) if resource == "php"
    ));
    assert!(matches!(
        frankenphp_result,
        Err(ResourcesError::InvalidArtifactLayout { resource, .. }) if resource == "frankenphp"
    ));

    Ok(())
}

fn _assert_utf8_path_trait_bound(_path: &Utf8Path) {}
```

- [ ] **Step 2: Run failing adapter tests**

Run:

```bash
cargo nextest run -E 'test(php_adapter_validates_expected_executable_layout) or test(frankenphp_adapter_validates_expected_executable_layout) or test(runtime_adapters_reject_missing_executables)'
```

Expected: FAIL to compile because `php_adapter`, `frankenphp_adapter`, and the runtime adapter type do not exist.

- [ ] **Step 3: Add runtime adapter implementation**

Create `crates/resources/src/runtime.rs`:

```rust
use camino::{Utf8Path, Utf8PathBuf};

use crate::fs;
use crate::{ResourceAdapter, ResourceName, ResourcesError, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeArtifactAdapter {
    resource_name: ResourceName,
    executable_relative_path: &'static str,
}

pub fn php_adapter() -> Result<RuntimeArtifactAdapter> {
    RuntimeArtifactAdapter::new("php", "bin/php")
}

pub fn frankenphp_adapter() -> Result<RuntimeArtifactAdapter> {
    RuntimeArtifactAdapter::new("frankenphp", "bin/frankenphp")
}

impl RuntimeArtifactAdapter {
    fn new(resource_name: &str, executable_relative_path: &'static str) -> Result<Self> {
        Ok(Self {
            resource_name: ResourceName::new(resource_name.to_string())?,
            executable_relative_path,
        })
    }

    pub fn executable_path(&self, release_path: &Utf8Path) -> Utf8PathBuf {
        release_path.join(self.executable_relative_path)
    }
}

impl ResourceAdapter for RuntimeArtifactAdapter {
    fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    fn validate_installation(&self, root: &Utf8Path) -> Result<()> {
        let executable = self.executable_path(root);
        if fs::path_entry_exists(&executable)? {
            return Ok(());
        }

        Err(ResourcesError::InvalidArtifactLayout {
            resource: self.resource_name.as_str().to_string(),
            reason: format!("missing executable `{}`", self.executable_relative_path),
        })
    }
}
```

Modify `crates/resources/src/lib.rs`:

```rust
pub mod runtime;
pub use runtime::{RuntimeArtifactAdapter, frankenphp_adapter, php_adapter};
```

- [ ] **Step 4: Run adapter tests**

Run:

```bash
cargo nextest run -E 'test(php_adapter_validates_expected_executable_layout) or test(frankenphp_adapter_validates_expected_executable_layout) or test(runtime_adapters_reject_missing_executables)'
```

Expected: PASS.

- [ ] **Step 5: Commit Task 1**

```bash
git add crates/resources/src/runtime.rs crates/resources/src/lib.rs crates/resources/tests/runtime_adapters.rs
git commit -m "feat(resources): add PHP runtime artifact adapters"
```

## Task 2: State Paths, PHP-Track Worker Ports, And Runtime Observations

**Files:**
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/state/src/database.rs`
- Modify: `crates/state/src/lib.rs`
- Test: `crates/state/tests/state_foundation.rs`

- [ ] **Step 1: Write failing state tests**

Append these tests near the existing port/path tests in `crates/state/tests/state_foundation.rs`:

```rust
#[test]
fn pv_paths_include_gateway_and_worker_runtime_artifacts() {
    let paths = PvPaths::for_home("/Users/alice");

    assert_eq!(
        paths.gateway_root_config().as_str(),
        "/Users/alice/.pv/config/gateway/Caddyfile"
    );
    assert_eq!(
        paths.worker_root_config("8.4").as_str(),
        "/Users/alice/.pv/config/workers/php-8.4/Caddyfile"
    );
    assert_eq!(
        paths.gateway_log().as_str(),
        "/Users/alice/.pv/logs/gateway/gateway.log"
    );
    assert_eq!(
        paths.worker_log("8.4").as_str(),
        "/Users/alice/.pv/logs/workers/php-8.4.log"
    );
}

#[test]
fn php_worker_port_allocator_persists_one_port_per_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let first = database.assign_port(
        PortRequest::php_worker("8.4", RUNTIME_PORT_FALLBACK_START, RUNTIME_PORT_FALLBACK_START, RUNTIME_PORT_FALLBACK_END),
        |_port| true,
    )?;
    let reused = database.assign_port(
        PortRequest::php_worker("8.4", RUNTIME_PORT_FALLBACK_START + 100, RUNTIME_PORT_FALLBACK_START, RUNTIME_PORT_FALLBACK_END),
        |_port| false,
    )?;

    assert_eq!(first.owner, PortOwner::PhpWorker { php_track: "8.4".to_string() });
    assert_eq!(reused.port, first.port);

    Ok(())
}

#[test]
fn runtime_observed_state_round_trips_through_observed_states() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Running,
        Some("Gateway ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::PhpWorker { php_track: "8.4".to_string() },
        RuntimeObservedStatus::Failed,
        Some("readiness timed out"),
    )?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!(database.runtime_observed_states()?);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
```

Add imports if missing:

```rust
use state::{RuntimeObservedStatus, RuntimeSubject};
```

- [ ] **Step 2: Run failing state tests**

Run:

```bash
cargo nextest run -E 'test(pv_paths_include_gateway_and_worker_runtime_artifacts) or test(php_worker_port_allocator_persists_one_port_per_track) or test(runtime_observed_state_round_trips_through_observed_states)'
```

Expected: FAIL to compile because the path helpers, `PortOwner::PhpWorker`, `PortRequest::php_worker`, and runtime observed-state APIs do not exist.

- [ ] **Step 3: Add Gateway and worker path helpers**

In `crates/state/src/paths.rs`, add these methods inside `impl PvPaths`:

```rust
pub fn gateway_root_config(&self) -> Utf8PathBuf {
    self.config().join("gateway/Caddyfile")
}

pub fn gateway_projects_config_dir(&self) -> Utf8PathBuf {
    self.config().join("gateway/projects")
}

pub fn worker_root_config(&self, php_track: &str) -> Utf8PathBuf {
    self.config()
        .join("workers")
        .join(format!("php-{php_track}"))
        .join("Caddyfile")
}

pub fn worker_projects_config_dir(&self, php_track: &str) -> Utf8PathBuf {
    self.config()
        .join("workers")
        .join(format!("php-{php_track}"))
        .join("projects")
}

pub fn gateway_log(&self) -> Utf8PathBuf {
    self.logs().join("gateway/gateway.log")
}

pub fn worker_log(&self, php_track: &str) -> Utf8PathBuf {
    self.logs().join(format!("workers/php-{php_track}.log"))
}

pub fn gateway_pid(&self) -> Utf8PathBuf {
    self.run().join("gateway.pid")
}

pub fn gateway_runtime_metadata(&self) -> Utf8PathBuf {
    self.run().join("gateway.runtime.json")
}

pub fn worker_pid(&self, php_track: &str) -> Utf8PathBuf {
    self.run().join(format!("worker-php-{php_track}.pid"))
}

pub fn worker_runtime_metadata(&self, php_track: &str) -> Utf8PathBuf {
    self.run().join(format!("worker-php-{php_track}.runtime.json"))
}
```

- [ ] **Step 4: Replace project-specific worker port ownership with PHP-track ownership**

In `crates/state/src/database.rs`, change `PortOwner`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortOwner {
    Dns,
    Gateway(GatewayPort),
    PhpWorker {
        php_track: String,
    },
    Resource {
        name: String,
        track: String,
    },
}
```

Replace `PortRequest::project_worker` with:

```rust
pub fn php_worker(
    php_track: impl Into<String>,
    preferred_port: u16,
    fallback_start: u16,
    fallback_end: u16,
) -> Self {
    Self::new(
        PortOwner::PhpWorker {
            php_track: php_track.into(),
        },
        preferred_port,
        fallback_start,
        fallback_end,
    )
}
```

Update `PortOwner::identity`, `PortOwner::from_database`, empty-component validation, and display names:

```rust
Self::PhpWorker { php_track } => {
    self.validate_component("php_track", php_track)?;

    Ok(PortIdentity {
        owner_kind: "php_worker",
        owner_id: php_track.clone(),
        owner_track: String::new(),
    })
}
```

```rust
"php_worker" if !owner_id.is_empty() && owner_track.is_empty() => {
    Ok(Self::PhpWorker { php_track: owner_id })
}
"php_worker" => Err(StateError::InvalidPortOwner {
    owner: describe_port_identity(&owner_kind, &owner_id, &owner_track),
    reason: "PHP worker ports must use the PHP track as owner id and an empty owner track",
}),
```

```rust
Self::PhpWorker { php_track } => format!("PHP worker {php_track:?}"),
```

- [ ] **Step 5: Add runtime observed-state APIs using the existing `observed_states` table**

In `crates/state/src/database.rs`, add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeSubject {
    Gateway,
    PhpWorker { php_track: String },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RuntimeObservedStatus {
    Running,
    Stopped,
    Degraded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeObservedStateRecord {
    pub subject: RuntimeSubject,
    pub status: RuntimeObservedStatus,
    pub message: Option<String>,
    pub observed_at: String,
}
```

Add `Database` methods:

```rust
pub fn record_runtime_observed_snapshot(
    &mut self,
    subject: RuntimeSubject,
    status: RuntimeObservedStatus,
    message: Option<&str>,
) -> Result<RuntimeObservedStateRecord, StateError> {
    let observed_at = timestamp()?;
    let subject_id = subject.subject_id();
    self.connection.execute(
        "INSERT INTO observed_states (
            subject_kind,
            subject_id,
            status,
            message,
            observed_at
        ) VALUES ('runtime', ?1, ?2, ?3, ?4)
        ON CONFLICT(subject_kind, subject_id) DO UPDATE SET
            status = excluded.status,
            message = excluded.message,
            observed_at = excluded.observed_at",
        params![subject_id, status.as_str(), message, observed_at],
    )?;

    Ok(RuntimeObservedStateRecord {
        subject,
        status,
        message: message.map(str::to_string),
        observed_at,
    })
}

pub fn runtime_observed_states(&self) -> Result<Vec<RuntimeObservedStateRecord>, StateError> {
    let mut statement = self.connection.prepare(
        "SELECT subject_id, status, message, observed_at
        FROM observed_states
        WHERE subject_kind = 'runtime'
        ORDER BY subject_id",
    )?;
    let rows = statement.query_map([], runtime_observed_state_from_row)?;
    let mut records = Vec::new();

    for row in rows {
        records.push(row?);
    }

    Ok(records)
}
```

Add helper methods:

```rust
impl RuntimeSubject {
    fn subject_id(&self) -> String {
        match self {
            Self::Gateway => "gateway".to_string(),
            Self::PhpWorker { php_track } => format!("php-worker:{php_track}"),
        }
    }

    fn from_subject_id(subject_id: String) -> Result<Self, StateError> {
        if subject_id == "gateway" {
            return Ok(Self::Gateway);
        }
        if let Some(php_track) = subject_id.strip_prefix("php-worker:")
            && !php_track.is_empty()
        {
            return Ok(Self::PhpWorker {
                php_track: php_track.to_string(),
            });
        }

        Err(StateError::InvalidObservedStateSubject { subject_id })
    }
}

impl RuntimeObservedStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }

    fn from_database(status: String) -> Result<Self, StateError> {
        match status.as_str() {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            "degraded" => Ok(Self::Degraded),
            "failed" => Ok(Self::Failed),
            _ => Err(StateError::UnknownObservedStateStatus { status }),
        }
    }
}
```

If `StateError` lacks suitable variants, add:

```rust
#[error("invalid observed state subject `{subject_id}`")]
InvalidObservedStateSubject { subject_id: String },

#[error("unknown observed state status `{status}`")]
UnknownObservedStateStatus { status: String },
```

- [ ] **Step 6: Export new state types**

Modify `crates/state/src/lib.rs`:

```rust
pub use database::{
    RuntimeObservedStateRecord, RuntimeObservedStatus, RuntimeSubject,
};
```

Fold those names into the existing grouped `pub use database` block in `crates/state/src/lib.rs` instead of adding a duplicate `pub use database` block.

- [ ] **Step 7: Run and accept state snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- php_worker_port_allocator_persists_one_port_per_track
cargo insta test --accept --test-runner nextest -- runtime_observed_state_round_trips_through_observed_states
cargo nextest run -E 'test(pv_paths_include_gateway_and_worker_runtime_artifacts)'
```

Expected: PASS and new/updated snapshots accepted.

- [ ] **Step 8: Commit Task 2**

```bash
git add crates/state/src/paths.rs crates/state/src/database.rs crates/state/src/lib.rs crates/state/src/error.rs crates/state/tests/state_foundation.rs crates/state/tests/snapshots
git commit -m "feat(state): model PHP worker runtime state"
```

## Task 3: Gateway And Worker Config Rendering

**Files:**
- Create: `crates/daemon/src/gateway_config.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/tests/gateway_config.rs`

- [ ] **Step 1: Write failing config renderer snapshot tests**

Create `crates/daemon/tests/gateway_config.rs`:

```rust
use anyhow::Result;
use camino::Utf8PathBuf;
use daemon::gateway_config::{
    GatewayConfigInput, GatewayProjectRoute, PhpWorkerConfigInput, PhpWorkerProject,
    render_gateway_config, render_php_worker_config,
};
use insta::assert_snapshot;

#[test]
fn gateway_config_renderer_outputs_gateway_caddyfile() -> Result<()> {
    let rendered = render_gateway_config(&GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca-key.pem"),
        routes: vec![GatewayProjectRoute {
            primary_hostname: "acme.test".to_string(),
            hostnames: vec!["acme.test".to_string(), "api.acme.test".to_string()],
            worker_port: 45001,
        }],
    })?;

    assert_snapshot!(rendered);

    Ok(())
}

#[test]
fn worker_config_renderer_outputs_track_caddyfile() -> Result<()> {
    let rendered = render_php_worker_config(&PhpWorkerConfigInput {
        php_track: "8.4".to_string(),
        port: 45001,
        projects: vec![PhpWorkerProject {
            primary_hostname: "acme.test".to_string(),
            hostnames: vec!["acme.test".to_string(), "api.acme.test".to_string()],
            project_root: Utf8PathBuf::from("/Users/alice/Code/acme"),
            document_root: Utf8PathBuf::from("/Users/alice/Code/acme/public"),
        }],
    })?;

    assert_snapshot!(rendered);

    Ok(())
}
```

- [ ] **Step 2: Run failing config tests**

Run:

```bash
cargo nextest run -E 'test(gateway_config_renderer_outputs_gateway_caddyfile) or test(worker_config_renderer_outputs_track_caddyfile)'
```

Expected: FAIL to compile because `daemon::gateway_config` does not exist.

- [ ] **Step 3: Implement pure config model and renderers**

Create `crates/daemon/src/gateway_config.rs` with these public input types:

```rust
use camino::Utf8PathBuf;

use crate::DaemonError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayConfigInput {
    pub http_port: u16,
    pub https_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
    pub routes: Vec<GatewayProjectRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayProjectRoute {
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub worker_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerConfigInput {
    pub php_track: String,
    pub port: u16,
    pub projects: Vec<PhpWorkerProject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerProject {
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub project_root: Utf8PathBuf,
    pub document_root: Utf8PathBuf,
}
```

Implement `render_gateway_config()` so the snapshot starts with this shape:

```text
{
    admin off
    http_port 48080
    https_port 48443
    pki {
        ca local {
            root {
                format pem_file
                cert /Users/alice/.pv/certificates/ca.pem
                key /Users/alice/.pv/certificates/ca-key.pem
            }
        }
    }
}

acme.test, api.acme.test {
    bind 127.0.0.1 ::1
    tls {
        issuer internal {
            ca local
        }
    }
    reverse_proxy 127.0.0.1:45001 {
        header_up Host {host}
        header_up X-Forwarded-Host {host}
        header_up X-Forwarded-Proto {scheme}
        header_up X-Forwarded-For {remote_host}
    }
}
```

Implement `render_php_worker_config()` so the snapshot starts with this shape:

```text
http://acme.test:45001, http://api.acme.test:45001 {
    bind 127.0.0.1 ::1
    root * /Users/alice/Code/acme/public
    php_server
    file_server
}
```

Sort routes/projects by primary hostname and sort hostnames inside each block. Use small helper functions to render comma-separated host lists. Return `DaemonError::UnexpectedProtocolResponse` only if impossible input is detected, such as an empty hostname list; otherwise keep renderers infallible in practice.

- [ ] **Step 4: Export the module for tests**

Modify `crates/daemon/src/lib.rs`:

```rust
pub mod gateway_config;
```

Keep it public because integration tests live outside the crate.

- [ ] **Step 5: Run and accept config snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- gateway_config_renderer_outputs_gateway_caddyfile
cargo insta test --accept --test-runner nextest -- worker_config_renderer_outputs_track_caddyfile
```

Expected: PASS and snapshots created under `crates/daemon/tests/snapshots/`.

- [ ] **Step 6: Commit Task 3**

```bash
git add crates/daemon/src/gateway_config.rs crates/daemon/src/lib.rs crates/daemon/tests/gateway_config.rs crates/daemon/tests/snapshots
git commit -m "feat(daemon): render gateway runtime configs"
```

## Task 4: Runtime Plan Builder

**Files:**
- Create: `crates/daemon/src/gateway.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/tests/gateway_reconciliation.rs`

- [ ] **Step 1: Write failing runtime plan tests**

Create `crates/daemon/tests/gateway_reconciliation.rs`:

```rust
use anyhow::Result;
use camino_tempfile::tempdir;
use daemon::gateway::build_runtime_plan;
use insta::assert_debug_snapshot;
use state::{Database, LinkProjectInput, PvPaths};

#[test]
fn runtime_plan_groups_linked_projects_by_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let acme_root = tempdir.path().join("acme");
    let api_root = tempdir.path().join("api");
    state::fs::write_sensitive_file(&acme_root.join("public/index.php"), "<?php echo 'acme';")?;
    state::fs::write_sensitive_file(&api_root.join("public/index.php"), "<?php echo 'api';")?;
    state::fs::write_sensitive_file(&acme_root.join("pv.yml"), "php: \"8.4\"\ndocument_root: public\nhostnames:\n  - api.acme.test\n")?;
    state::fs::write_sensitive_file(&api_root.join("pv.yml"), "php: \"8.3\"\ndocument_root: public\n")?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: acme_root.clone(),
        original_path: acme_root.clone(),
        primary_hostname: "acme.test".to_string(),
        config_path: acme_root.join("pv.yml"),
        desired_php_track: Some("8.4".to_string()),
        additional_hostnames: vec!["api.acme.test".to_string()],
    })?;
    database.link_project(LinkProjectInput {
        path: api_root.clone(),
        original_path: api_root.clone(),
        primary_hostname: "other.test".to_string(),
        config_path: api_root.join("pv.yml"),
        desired_php_track: Some("8.3".to_string()),
        additional_hostnames: vec![],
    })?;

    let plan = build_runtime_plan(&paths)?;

    assert_debug_snapshot!(plan);

    Ok(())
}
```

- [ ] **Step 2: Run failing plan test**

Run:

```bash
cargo nextest run -E 'test(runtime_plan_groups_linked_projects_by_php_track)'
```

Expected: FAIL to compile because `daemon::gateway::build_runtime_plan` does not exist.

- [ ] **Step 3: Add runtime plan data structures**

Create `crates/daemon/src/gateway.rs` with:

```rust
use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use config::ProjectConfigFile;
use resources::{ArtifactManifestCache, ResourceName, TrackSelector};
use state::{Database, PortRequest, ProjectRecord, PvPaths};

use crate::DaemonError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePlan {
    pub gateway: GatewayRuntimePlan,
    pub workers: Vec<PhpWorkerRuntimePlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayRuntimePlan {
    pub http_port: u16,
    pub https_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerRuntimePlan {
    pub php_track: String,
    pub port: u16,
    pub projects: Vec<RuntimeProject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeProject {
    pub id: String,
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub project_root: Utf8PathBuf,
    pub document_root: Utf8PathBuf,
}
```

- [ ] **Step 4: Implement `build_runtime_plan`**

Add:

```rust
pub fn build_runtime_plan(paths: &PvPaths) -> Result<RuntimePlan, DaemonError> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(|port| port_is_available(port))?;
    let mut workers = BTreeMap::<String, PhpWorkerRuntimePlan>::new();

    for project in database.projects()? {
        let config_file = ProjectConfigFile::read_from_root(&project.path)?;
        let php_track = resolve_project_php_track(paths, &project, config_file.config.php.as_deref())?;
        let document_root = config_file
            .config
            .document_root
            .as_ref()
            .map(|document_root| project.path.join(document_root))
            .unwrap_or_else(|| project.path.clone());
        if !workers.contains_key(&php_track) {
            let assignment = database.assign_port(
                PortRequest::php_worker(
                    php_track.clone(),
                    state::RUNTIME_PORT_FALLBACK_START,
                    state::RUNTIME_PORT_FALLBACK_START,
                    state::RUNTIME_PORT_FALLBACK_END,
                ),
                |port| port_is_available(port),
            )?;
            workers.insert(
                php_track.clone(),
                PhpWorkerRuntimePlan {
                    php_track: php_track.clone(),
                    port: assignment.port,
                    projects: Vec::new(),
                },
            );
        }
        let Some(worker) = workers.get_mut(&php_track) else {
            return Err(DaemonError::UnexpectedProtocolResponse {
                reason: format!("PHP worker plan for track `{php_track}` was not inserted"),
            });
        };
        let mut hostnames = Vec::new();
        hostnames.push(project.primary_hostname.clone());
        hostnames.extend(config_file.config.hostnames.clone());
        hostnames.sort();
        hostnames.dedup();
        worker.projects.push(RuntimeProject {
            id: project.id,
            primary_hostname: project.primary_hostname,
            hostnames,
            project_root: project.path,
            document_root,
        });
    }

    Ok(RuntimePlan {
        gateway: GatewayRuntimePlan {
            http_port: gateway_ports.http.port,
            https_port: gateway_ports.https.port,
            ca_certificate_path: paths.ca_certificate(),
            ca_private_key_path: paths.ca_private_key(),
        },
        workers: workers.into_values().collect(),
    })
}
```

Use this helper for track resolution:

```rust
fn resolve_project_php_track(
    paths: &PvPaths,
    project: &ProjectRecord,
    parsed_track: Option<&str>,
) -> Result<String, DaemonError> {
    if let Some(track) = parsed_track.or(project.desired_php_track.as_deref()) {
        return Ok(track.to_string());
    }

    let php = ResourceName::new("php".to_string())?;
    let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
    let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

    Ok(track.as_str().to_string())
}
```

Use a local availability helper that relies on the standard library:

```rust
fn port_is_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}
```

If Clippy flags direct bind use, add a narrow `#[expect(clippy::disallowed_types, reason = "daemon runtime planning checks local port availability")]` or move the check to an existing platform/socket helper.

- [ ] **Step 5: Export `gateway` for tests**

Modify `crates/daemon/src/lib.rs`:

```rust
pub mod gateway;
```

- [ ] **Step 6: Run and accept runtime plan snapshot**

Run:

```bash
cargo insta test --accept --test-runner nextest -- runtime_plan_groups_linked_projects_by_php_track
```

Expected: PASS and a snapshot showing two workers, one for `8.3` and one for `8.4`.

- [ ] **Step 7: Commit Task 4**

```bash
git add crates/daemon/src/gateway.rs crates/daemon/src/lib.rs crates/daemon/tests/gateway_reconciliation.rs crates/daemon/tests/snapshots
git commit -m "feat(daemon): build PHP runtime plans"
```

## Task 5: Config Validation, Atomic Promotion, And Process Specs

**Files:**
- Modify: `crates/daemon/src/gateway.rs`
- Modify: `crates/daemon/src/gateway_config.rs`
- Test: `crates/daemon/tests/gateway_reconciliation.rs`

- [ ] **Step 1: Add fake validation failure test**

Append to `crates/daemon/tests/gateway_reconciliation.rs`:

```rust
#[tokio::test]
async fn gateway_config_validation_failure_preserves_active_config() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    state::fs::write_sensitive_file(&paths.gateway_root_config(), "previous config\n")?;

    let result = daemon::gateway::promote_validated_config_for_test(
        &paths.gateway_root_config(),
        "new config\n",
        |_candidate_path| Err(daemon::DaemonError::UnexpectedProtocolResponse {
            reason: "validation failed".to_string(),
        }),
    );

    assert!(result.is_err());
    assert_eq!(
        state::testing::read_to_string(&paths.gateway_root_config())?,
        "previous config\n"
    );

    Ok(())
}
```

- [ ] **Step 2: Run failing validation test**

Run:

```bash
cargo nextest run -E 'test(gateway_config_validation_failure_preserves_active_config)'
```

Expected: FAIL to compile because `promote_validated_config_for_test` does not exist.

- [ ] **Step 3: Add atomic config promotion helper**

In `crates/daemon/src/gateway_config.rs`, add:

```rust
use camino::{Utf8Path, Utf8PathBuf};

pub(crate) fn promote_validated_config(
    path: &Utf8Path,
    content: &str,
    validate: impl FnOnce(&Utf8Path) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    let candidate = candidate_config_path(path);
    write_candidate_config(&candidate, content)?;

    let validation = validate(&candidate);
    if let Err(error) = validation {
        let _cleanup = remove_file_if_exists(&candidate);
        return Err(error);
    }

    rename_config(&candidate, path)?;

    Ok(())
}

fn candidate_config_path(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("Caddyfile");
    path.with_file_name(format!("{file_name}.candidate.{}.tmp", std::process::id()))
}

#[expect(
    clippy::disallowed_methods,
    reason = "Gateway config promotion validates generated candidate files before atomic rename"
)]
fn write_candidate_config(path: &Utf8Path, content: &str) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Gateway config promotion atomically promotes validated generated config"
)]
fn rename_config(from: &Utf8Path, to: &Utf8Path) -> Result<(), DaemonError> {
    std::fs::rename(from, to)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Gateway config promotion removes failed generated candidate files"
)]
fn remove_file_if_exists(path: &Utf8Path) -> Result<(), DaemonError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
```

Expose a test-only wrapper from `crates/daemon/src/gateway.rs`:

```rust
#[cfg(test)]
pub fn promote_validated_config_for_test(
    path: &camino::Utf8Path,
    content: &str,
    validate: impl FnOnce(&camino::Utf8Path) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    crate::gateway_config::promote_validated_config(path, content, validate)
}
```

For integration tests, `#[cfg(test)]` is not visible. Instead make this wrapper public but clearly named `promote_validated_config_for_test`, and keep it small.

- [ ] **Step 4: Add FrankenPHP command helpers**

In `crates/daemon/src/gateway.rs`, add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrankenphpCommand {
    executable: Utf8PathBuf,
}

impl FrankenphpCommand {
    pub fn new(executable: impl Into<Utf8PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn validate_arguments(&self, config_path: &camino::Utf8Path) -> Vec<String> {
        vec![
            "validate".to_string(),
            "--config".to_string(),
            config_path.to_string(),
            "--adapter".to_string(),
            "caddyfile".to_string(),
        ]
    }

    pub fn run_arguments(&self, config_path: &camino::Utf8Path) -> Vec<String> {
        vec![
            "run".to_string(),
            "--config".to_string(),
            config_path.to_string(),
            "--adapter".to_string(),
            "caddyfile".to_string(),
        ]
    }
}
```

Add a `validate_config()` function that runs the managed binary with the `validate` arguments and returns `DaemonError::UnexpectedProtocolResponse` with stderr/stdout text when the command exits non-zero. Use a narrow `#[expect(clippy::disallowed_types, reason = "daemon runtime owns FrankenPHP config validation process execution")]` for `std::process::Command`.

- [ ] **Step 5: Build Gateway and worker `ProcessSpec`s**

Add helpers:

```rust
fn gateway_process_spec(paths: &PvPaths, command: &FrankenphpCommand) -> ProcessSpec {
    ProcessSpec {
        name: "gateway".to_string(),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.gateway_root_config()),
        config_path: paths.gateway_root_config(),
        log_path: paths.gateway_log(),
        pid_path: paths.gateway_pid(),
        metadata_path: paths.gateway_runtime_metadata(),
        resource_name: "gateway".to_string(),
        track: "core".to_string(),
    }
}

fn worker_process_spec(
    paths: &PvPaths,
    php_track: &str,
    command: &FrankenphpCommand,
) -> ProcessSpec {
    ProcessSpec {
        name: format!("php-worker-{php_track}"),
        command: command.executable.clone(),
        arguments: command.run_arguments(&paths.worker_root_config(php_track)),
        config_path: paths.worker_root_config(php_track),
        log_path: paths.worker_log(php_track),
        pid_path: paths.worker_pid(php_track),
        metadata_path: paths.worker_runtime_metadata(php_track),
        resource_name: "php-worker".to_string(),
        track: php_track.to_string(),
    }
}
```

- [ ] **Step 6: Run validation failure test**

Run:

```bash
cargo nextest run -E 'test(gateway_config_validation_failure_preserves_active_config)'
```

Expected: PASS.

- [ ] **Step 7: Commit Task 5**

```bash
git add crates/daemon/src/gateway.rs crates/daemon/src/gateway_config.rs crates/daemon/tests/gateway_reconciliation.rs
git commit -m "feat(daemon): validate gateway runtime configs"
```

## Task 6: Supervised Gateway And Worker Reconciliation

**Files:**
- Modify: `crates/daemon/src/gateway.rs`
- Modify: `crates/daemon/src/jobs.rs`
- Test: `crates/daemon/tests/gateway_reconciliation.rs`
- Test: `crates/daemon/tests/daemon_foundation.rs`

- [ ] **Step 1: Add fake runtime reconciliation test**

Append a test that uses a fake executable script capable of validating config and serving HTTP:

```rust
#[tokio::test]
async fn gateway_reconciliation_starts_gateway_and_one_worker_per_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let fake_frankenphp = write_fake_frankenphp(&tempdir.path().join("fake-frankenphp"))?;
    let project_root = tempdir.path().join("acme");
    state::fs::write_sensitive_file(&project_root.join("public/index.php"), "<?php echo 'acme';")?;
    state::fs::write_sensitive_file(&project_root.join("pv.yml"), "php: \"8.4\"\ndocument_root: public\n")?;
    seed_linked_project(&paths, &project_root, "acme.test", "8.4")?;
    seed_installed_runtime_tracks(&paths, &fake_frankenphp, "8.4")?;

    let summary = daemon::gateway::reconcile_gateway_runtimes(&paths).await?;
    let database = Database::open(&paths)?;

    assert_eq!(summary, "Gateway runtime reconciled");
    assert!(paths.gateway_pid().exists());
    assert!(paths.worker_pid("8.4").exists());
    assert_debug_snapshot!(database.runtime_observed_states()?);

    Ok(())
}
```

Implement helper functions in the test file:

```rust
fn write_fake_frankenphp(path: &camino::Utf8Path) -> Result<camino::Utf8PathBuf> {
    state::fs::write_sensitive_file(
        path,
        r#"#!/bin/sh
if [ "$1" = "validate" ]; then
  test -f "$3"
  exit $?
fi
if [ "$1" = "run" ]; then
  port="$(sed -n 's/.*PV_FAKE_PORT \([0-9][0-9]*\).*/\1/p' "$3" | head -n 1)"
  exec python3 -m http.server "$port" --bind 127.0.0.1
fi
exit 2
"#,
    )?;
    set_executable(path)?;
    Ok(path.to_path_buf())
}
```

Have `render_gateway_config` and `render_php_worker_config` emit a harmless first-line comment containing the primary listen port:

```text
# PV_FAKE_PORT 45001
```

The fake script parses that comment. The comment is useful in snapshots and is ignored by Caddy.

- [ ] **Step 2: Run failing reconciliation test**

Run:

```bash
cargo nextest run -E 'test(gateway_reconciliation_starts_gateway_and_one_worker_per_php_track)'
```

Expected: FAIL to compile because `reconcile_gateway_runtimes` and test helpers do not exist.

- [ ] **Step 3: Implement runtime reconciliation**

In `crates/daemon/src/gateway.rs`, add:

```rust
pub async fn reconcile_gateway_runtimes(paths: &PvPaths) -> Result<String, DaemonError> {
    let plan = build_runtime_plan(paths)?;
    let frankenphp = installed_frankenphp_command(paths, &plan)?;
    write_and_validate_gateway_config(paths, &plan, &frankenphp)?;
    for worker in &plan.workers {
        write_and_validate_worker_config(paths, worker, &frankenphp)?;
    }
    start_or_adopt_gateway(paths, &frankenphp, &plan).await?;
    for worker in &plan.workers {
        start_or_adopt_worker(paths, worker, &frankenphp).await?;
    }
    record_stopped_workers_without_projects(paths, &plan)?;

    Ok("Gateway runtime reconciled".to_string())
}
```

Implementation details:

- `installed_frankenphp_command` reads `managed_resource_tracks` for resource `frankenphp`, desired/installed track matching the first required worker track. For PR 14, tests should seed one path per track. If multiple tracks point at identical FrankenPHP executable layouts, reuse the matching track command for the worker and Gateway.
- Gateway can use the default/first installed FrankenPHP artifact because Gateway only routes/proxies. PHP execution happens inside per-track workers.
- `write_and_validate_gateway_config` renders `GatewayConfigInput`, validates with `frankenphp validate --config <path> --adapter caddyfile`, then writes with `promote_validated_config`.
- `write_and_validate_worker_config` does the same for each worker.
- `start_or_adopt_gateway` calls `ProcessSupervisor::adopt`; if adoption fails, it starts a process and waits for HTTP readiness on `127.0.0.1:<gateway.http_port>`.
- `start_or_adopt_worker` waits for HTTP readiness on `127.0.0.1:<worker.port>`.
- Record `RuntimeObservedStatus::Running` on success and `RuntimeObservedStatus::Failed` with the error message on failure.
- Do not stop unrelated PHP workers when a single worker fails.

- [ ] **Step 4: Wire reconciliation jobs**

Modify `crates/daemon/src/jobs.rs`:

```rust
async fn complete_reconciliation_job(
    paths: &PvPaths,
    job_id: &str,
    scope: &ReconciliationScope,
) -> Result<String, DaemonError> {
    let result = match scope {
        ReconciliationScope::System => complete_system_reconciliation(paths, job_id).await,
        ReconciliationScope::Resource { name, .. }
            if matches!(name.as_str(), "php" | "frankenphp") =>
        {
            complete_gateway_reconciliation(paths, job_id).await
        }
        ReconciliationScope::Resource { .. } => {
            complete_stub_reconciliation(paths, job_id).map(str::to_string)
        }
        ReconciliationScope::Project { id } => {
            complete_project_then_gateway_reconciliation(paths, job_id, id).await
        }
    };

    if let Err(error) = &result {
        let error_message = error.to_string();
        let mut database = Database::open(paths)?;
        database.fail_job(job_id, &error_message)?;
    }

    result
}
```

Add:

```rust
async fn complete_system_reconciliation(paths: &PvPaths, job_id: &str) -> Result<String, DaemonError> {
    let summary = crate::gateway::reconcile_gateway_runtimes(paths).await?;
    let mut database = Database::open(paths)?;
    database.complete_job(job_id, &summary)?;
    Ok(summary)
}

async fn complete_gateway_reconciliation(paths: &PvPaths, job_id: &str) -> Result<String, DaemonError> {
    let summary = crate::gateway::reconcile_gateway_runtimes(paths).await?;
    let mut database = Database::open(paths)?;
    database.complete_job(job_id, &summary)?;
    Ok(summary)
}

async fn complete_project_then_gateway_reconciliation(
    paths: &PvPaths,
    job_id: &str,
    id: &crate::reconciliation::ReconciliationScopeComponent,
) -> Result<String, DaemonError> {
    let project_summary = reconcile_project_env(paths, id.as_str())?;
    let gateway_summary = crate::gateway::reconcile_gateway_runtimes(paths).await?;
    let summary = format!("{}; {}", project_summary.as_str(), gateway_summary);
    let mut database = Database::open(paths)?;
    database.complete_job(job_id, &summary)?;
    Ok(summary)
}
```

Update callers of `complete_reconciliation_job` to `.await` the function. This keeps Gateway reconciliation on the existing daemon runtime and avoids a nested Tokio runtime.

- [ ] **Step 5: Update system reconciliation expectations**

Run:

```bash
cargo nextest run -E 'test(socket_protocol_streams_job_progress_and_persists_final_status) or test(blocking_client_waits_for_reconciliation_stream_completion)'
```

Expected: these may now fail because `system` reconciliation needs installed FrankenPHP state. Adjust system reconciliation to only run Gateway runtimes when at least one valid installed FrankenPHP artifact exists; otherwise complete with summary `Gateway runtime skipped; FrankenPHP is not installed`. Keep setup safe before real artifacts exist.

- [ ] **Step 6: Re-run daemon tests and accept snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- gateway_reconciliation_starts_gateway_and_one_worker_per_php_track
cargo insta test --accept --test-runner nextest -- socket_protocol_streams_job_progress_and_persists_final_status
cargo nextest run -E 'test(blocking_client_waits_for_reconciliation_stream_completion)'
```

Expected: PASS.

- [ ] **Step 7: Commit Task 6**

```bash
git add crates/daemon/src/gateway.rs crates/daemon/src/jobs.rs crates/daemon/tests/gateway_reconciliation.rs crates/daemon/tests/daemon_foundation.rs crates/daemon/tests/snapshots
git commit -m "feat(daemon): reconcile gateway runtimes"
```

## Task 7: Reload Or Restart Behavior

**Files:**
- Modify: `crates/daemon/src/supervisor.rs`
- Modify: `crates/daemon/src/gateway.rs`
- Test: `crates/daemon/tests/supervisor_foundation.rs`
- Test: `crates/daemon/tests/gateway_reconciliation.rs`

- [ ] **Step 1: Add supervisor reload test**

Add to `crates/daemon/tests/supervisor_foundation.rs`:

```rust
#[tokio::test]
async fn supervisor_sends_reload_signal_to_owned_runtime() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let marker = paths.run().join("reload-marker");
    let process = ProcessSupervisor::new(paths.clone())
        .start(process_spec(
            &paths,
            "reloadable-runtime",
            "/bin/sh",
            vec![
                "-c".to_string(),
                format!("trap 'touch {marker}' USR1; while true; do sleep 1; done"),
            ],
        ))
        .await?;
    let spec = process_spec(
        &paths,
        "reloadable-runtime",
        "/bin/sh",
        vec![
            "-c".to_string(),
            format!("trap 'touch {marker}' USR1; while true; do sleep 1; done"),
        ],
    );

    ProcessSupervisor::new(paths.clone()).reload(&spec)?;
    wait_for_path(&marker).await?;
    process.stop(Duration::from_secs(1)).await?;

    Ok(())
}
```

- [ ] **Step 2: Run failing reload test**

Run:

```bash
cargo nextest run -E 'test(supervisor_sends_reload_signal_to_owned_runtime)'
```

Expected: FAIL to compile because `ProcessSupervisor::reload` does not exist.

- [ ] **Step 3: Implement reload signaling**

In `crates/daemon/src/supervisor.rs`, add:

```rust
impl ProcessSupervisor {
    pub fn reload(&self, spec: &ProcessSpec) -> Result<bool, DaemonError> {
        let Some(owned) = self.verify_ownership(spec)? else {
            return Ok(false);
        };
        let process_group = process_group_pid(owned.pid())?;
        signal_process_group(process_group, Signal::USR1)?;

        Ok(true)
    }
}
```

Use `Signal::USR1`, matching Caddy's documented signal reload behavior when started with `run --config`.

- [ ] **Step 4: Use reload in Gateway runtime reconciliation**

In `crates/daemon/src/gateway.rs`, update Gateway and worker lifecycle:

```rust
let reloaded = supervisor.reload(&spec)?;
if reloaded && wait_for_readiness(readiness, READINESS_TIMEOUT).await.is_ok() {
    return Ok(());
}

if let Some(adopted) = supervisor.adopt(&spec)? {
    stop_owned_runtime(adopted).await?;
}
let process = supervisor.start(spec).await?;
wait_for_readiness(readiness, READINESS_TIMEOUT).await?;
```

If there is no existing API to stop an adopted runtime, add the smallest safe helper needed in `supervisor.rs` that verifies ownership and signals the process group with TERM/KILL using the existing stop code path. Do not kill a PID without ownership verification.

- [ ] **Step 5: Add config-change reload fallback test**

Add to `crates/daemon/tests/gateway_reconciliation.rs`:

```rust
#[tokio::test]
async fn gateway_reconciliation_restarts_when_reload_is_unavailable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let fake_frankenphp = write_fake_frankenphp(&tempdir.path().join("fake-frankenphp"))?;
    seed_single_project_runtime(&paths, &tempdir, &fake_frankenphp, "8.4")?;

    daemon::gateway::reconcile_gateway_runtimes(&paths).await?;
    let first_metadata = state::testing::read_to_string(&paths.gateway_runtime_metadata())?;
    state::fs::delete_file(&paths.gateway_runtime_metadata())?;
    daemon::gateway::reconcile_gateway_runtimes(&paths).await?;
    let second_metadata = state::testing::read_to_string(&paths.gateway_runtime_metadata())?;

    assert_ne!(first_metadata, second_metadata);

    Ok(())
}
```

- [ ] **Step 6: Run reload tests**

Run:

```bash
cargo nextest run -E 'test(supervisor_sends_reload_signal_to_owned_runtime) or test(gateway_reconciliation_restarts_when_reload_is_unavailable)'
```

Expected: PASS.

- [ ] **Step 7: Commit Task 7**

```bash
git add crates/daemon/src/supervisor.rs crates/daemon/src/gateway.rs crates/daemon/tests/supervisor_foundation.rs crates/daemon/tests/gateway_reconciliation.rs
git commit -m "feat(daemon): reload gateway runtimes"
```

## Task 8: Opt-In Real-Artifact E2E

**Files:**
- Create: `crates/daemon/tests/real_artifact_gateway_e2e.rs`
- Create: `.github/workflows/real-artifact-e2e.yml`

- [ ] **Step 1: Add ignored real-artifact E2E test**

Create `crates/daemon/tests/real_artifact_gateway_e2e.rs`:

```rust
use anyhow::{Result, bail};
use camino_tempfile::tempdir;
use resources::{ManagedResourceCommands, TargetPlatform, TrackSelector, frankenphp_adapter, php_adapter};
use state::{Database, LinkProjectInput, PvPaths};

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_artifact_gateway_e2e_serves_tiny_php_project() -> Result<()> {
    if std::env::var("PV_E2E_REAL_ARTIFACTS").as_deref() != Ok("1") {
        return Ok(());
    }
    let manifest_url = match std::env::var("PV_E2E_ARTIFACT_MANIFEST_URL") {
        Ok(url) => url,
        Err(error) => bail!("PV_E2E_ARTIFACT_MANIFEST_URL is required: {error}"),
    };

    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());
    let client = resources::UreqResourceHttpClient::new();

    commands.install(&php_adapter()?, TrackSelector::Latest, &client)?;
    commands.install(&frankenphp_adapter()?, TrackSelector::Latest, &client)?;

    let project_root = tempdir.path().join("project");
    state::fs::write_sensitive_file(
        &project_root.join("public/index.php"),
        "<?php echo 'pv-real-artifact-ok';",
    )?;
    state::fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        "document_root: public\n",
    )?;
    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "real-artifact.test".to_string(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: vec![],
    })?;

    daemon::gateway::reconcile_gateway_runtimes(&paths).await?;
    let response = request_gateway_https_with_curl(&paths, "real-artifact.test")?;

    assert!(response.contains("pv-real-artifact-ok"));

    Ok(())
}
```

Add these helpers below the test:

```rust
fn target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "ignored real-artifact E2E shells out to curl to verify TLS with PV's CA"
)]
fn request_gateway_https_with_curl(paths: &PvPaths, hostname: &str) -> Result<String> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(|_port| true)?;
    let ca_certificate = paths.ca_certificate().to_string();
    let resolve = format!("{hostname}:{}:127.0.0.1", gateway_ports.https.port);
    let url = format!("https://{hostname}:{}/", gateway_ports.https.port);
    let output = std::process::Command::new("/usr/bin/curl")
        .args(vec![
            "--silent".to_string(),
            "--show-error".to_string(),
            "--fail".to_string(),
            "--cacert".to_string(),
            ca_certificate,
            "--resolve".to_string(),
            resolve,
            url,
        ])
        .output()?;

    if !output.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

- [ ] **Step 2: Run ignored test without env**

Run:

```bash
cargo nextest run -E 'test(real_artifact_gateway_e2e_serves_tiny_php_project)'
```

Expected: test is ignored by default and does not download artifacts.

- [ ] **Step 3: Run ignored test explicitly without env**

Run:

```bash
cargo nextest run --run-ignored ignored-only -E 'test(real_artifact_gateway_e2e_serves_tiny_php_project)'
```

Expected: PASS without downloading artifacts because `PV_E2E_REAL_ARTIFACTS` is absent.

- [ ] **Step 4: Add manual workflow**

Create `.github/workflows/real-artifact-e2e.yml`:

```yaml
name: Real Artifact E2E

on:
  workflow_dispatch:
    inputs:
      manifest_url:
        description: Candidate artifact manifest URL
        required: true
        type: string

jobs:
  real-artifact-e2e:
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Run real artifact Gateway E2E
        env:
          PV_E2E_REAL_ARTIFACTS: "1"
          PV_E2E_ARTIFACT_MANIFEST_URL: ${{ inputs.manifest_url }}
        run: cargo nextest run --run-ignored ignored-only -E 'test(real_artifact_gateway_e2e_serves_tiny_php_project)'
```

Do not add this workflow to normal `push` or `pull_request` triggers.

- [ ] **Step 5: Commit Task 8**

```bash
git add crates/daemon/tests/real_artifact_gateway_e2e.rs .github/workflows/real-artifact-e2e.yml
git commit -m "test(daemon): add opt-in real artifact gateway e2e"
```

## Task 9: Final Verification And Roadmap Update

**Files:**
- Modify: `IMPLEMENTATION.md`

- [ ] **Step 1: Run focused verification**

Run:

```bash
cargo nextest run -E 'test(php_adapter_validates_expected_executable_layout) or test(frankenphp_adapter_validates_expected_executable_layout) or test(runtime_adapters_reject_missing_executables)'
cargo nextest run -E 'test(pv_paths_include_gateway_and_worker_runtime_artifacts) or test(php_worker_port_allocator_persists_one_port_per_track) or test(runtime_observed_state_round_trips_through_observed_states)'
cargo nextest run -E 'test(gateway_config_renderer_outputs_gateway_caddyfile) or test(worker_config_renderer_outputs_track_caddyfile) or test(runtime_plan_groups_linked_projects_by_php_track)'
cargo nextest run -E 'test(gateway_reconciliation_starts_gateway_and_one_worker_per_php_track) or test(gateway_config_validation_failure_preserves_active_config) or test(supervisor_sends_reload_signal_to_owned_runtime)'
cargo nextest run --run-ignored ignored-only -E 'test(real_artifact_gateway_e2e_serves_tiny_php_project)'
```

Expected: all pass. The ignored E2E pass without env proves the default path does not download large artifacts.

- [ ] **Step 2: Run formatting and diff checks**

Run:

```bash
cargo fmt --all -- --check
git diff --check
```

Expected: both pass.

- [ ] **Step 3: Run broader checks before PR**

Run:

```bash
cargo nextest run --workspace
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo shear
```

Expected: all pass. If `cargo shear` is unavailable locally, record that in the PR notes rather than editing dependencies blindly.

- [ ] **Step 4: Run real-artifact E2E when candidate artifacts exist**

When candidate artifacts are available, run:

```bash
: "${PV_E2E_ARTIFACT_MANIFEST_URL:?set PV_E2E_ARTIFACT_MANIFEST_URL to the candidate manifest URL}"
PV_E2E_REAL_ARTIFACTS=1 cargo nextest run --run-ignored ignored-only -E 'test(real_artifact_gateway_e2e_serves_tiny_php_project)'
```

Expected: PASS and the response includes `pv-real-artifact-ok`.

If candidate artifacts are not available, leave the gated test in place and include this exact note in the PR body:

```text
Real-artifact Gateway E2E is implemented but not run in this branch because candidate PHP/FrankenPHP artifacts are not available yet. Default tests use fake artifacts and do not download large runtime archives.
```

- [ ] **Step 5: Update roadmap after PR number exists**

After opening the PR, update the PR tracking table in `IMPLEMENTATION.md`:

```markdown
| PR 14  | PHP/FrankenPHP adapters, workers, Gateway, first Project-serving test | PV-070, PV-071, PV-072, PV-073, PV-074, PV-078 | PR 7, PR 9, PR 13 | No | Yes (#123) |
```

Replace `123` with the actual PR number returned by GitHub. Do not update the row before the PR exists.

- [ ] **Step 6: Commit final docs update**

```bash
git add IMPLEMENTATION.md
git commit -m "docs: mark PR 14 complete"
```

Only run this commit after the PR number exists.
