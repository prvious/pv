# PR 16-20 Managed Resource Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build shared runtime and SQL foundations, then land full vertical slices for Mailpit, Redis, MySQL, Postgres, and RustFS Managed Resource adapters.

**Architecture:** Backing Managed Resource runtime orchestration stays daemon-local. The `resources` crate keeps artifact identity, manifest, install, and layout validation responsibilities. State adds only the narrow resource port-role support required for named multi-port runtimes; adapter env context and allocation readiness use the existing Managed Resource track and Resource allocation tables.

**Tech Stack:** Rust 2024, Tokio, rusqlite-backed state, clap, insta, nextest, `sqlx 0.9.0`, `redis 1.2.2`, `object_store 0.13.2`, `aws-sdk-s3 1.135.0`, Solo orchestration, git worktrees.

---

## Source Documents

Before editing code in any worktree, read these files in that worktree:

```bash
sed -n '1,220p' CONTRIBUTING.md
sed -n '1,260p' DESIGN.md
sed -n '1,360p' IMPLEMENTATION.md
sed -n '1,260p' docs/superpowers/specs/2026-06-08-pr-16-20-managed-resource-adapters-design.md
```

Expected: the commands print the current project rules and the approved adapter design. Stop and ask if the checked-out files conflict with this plan.

## Workstream Graph

Use this blocker graph:

| Workstream | Branch | Base | Blockers |
| --- | --- | --- | --- |
| Runtime foundation | `pr-16-20-runtime-foundation` | current `main` after this plan commit | none |
| SQL foundation | `pr-16-20-sql-foundation` | `pr-16-20-runtime-foundation` | runtime foundation |
| Mailpit adapter | `pr-16-mailpit-adapter` | `pr-16-20-runtime-foundation` | runtime foundation |
| Redis adapter | `pr-17-redis-adapter` | `pr-16-20-runtime-foundation` | runtime foundation |
| RustFS adapter | `pr-20-rustfs-adapter` | `pr-16-20-runtime-foundation` | runtime foundation |
| MySQL adapter | `pr-18-mysql-adapter` | `pr-16-20-sql-foundation` | runtime foundation, SQL foundation |
| Postgres adapter | `pr-19-postgres-adapter` | `pr-16-20-sql-foundation` | runtime foundation, SQL foundation |

Use the `superpowers:using-git-worktrees` skill when execution starts. Suggested worktree commands after approval:

```bash
git fetch origin main
git worktree add ../pv-pr16-20-runtime-foundation -b pr-16-20-runtime-foundation HEAD
```

After the runtime foundation branch is committed:

```bash
git worktree add ../pv-pr16-20-sql-foundation -b pr-16-20-sql-foundation pr-16-20-runtime-foundation
git worktree add ../pv-pr16-mailpit-adapter -b pr-16-mailpit-adapter pr-16-20-runtime-foundation
git worktree add ../pv-pr17-redis-adapter -b pr-17-redis-adapter pr-16-20-runtime-foundation
git worktree add ../pv-pr20-rustfs-adapter -b pr-20-rustfs-adapter pr-16-20-runtime-foundation
```

After the SQL foundation branch is committed:

```bash
git worktree add ../pv-pr18-mysql-adapter -b pr-18-mysql-adapter pr-16-20-sql-foundation
git worktree add ../pv-pr19-postgres-adapter -b pr-19-postgres-adapter pr-16-20-sql-foundation
```

## File Structure

Runtime foundation creates or modifies:

- Create `crates/state/src/sql/007_resource_port_roles.sql`: migration that adds `owner_port` to the port assignment key.
- Modify `crates/state/src/migrations.rs`: register migration 7.
- Modify `crates/state/src/database.rs`: extend `PortOwner::Resource`, `PortRequest`, port serialization, runtime observed subjects, and validation.
- Modify `crates/state/src/lib.rs`: export the new public state types or constructors used by daemon tests.
- Modify `crates/state/tests/state_foundation.rs`: state migration, port-role, and resource runtime subject coverage.
- Modify `crates/state/tests/snapshots/*.snap`: accepted snapshot changes from state tests.
- Modify `crates/state/src/paths.rs`: add resource runtime path helpers.
- Create `crates/daemon/src/managed_resources/mod.rs`: daemon-local runtime catalog, resource demand reconciliation, process start/adopt/stop, env context recording, allocation hook dispatch, and test injection.
- Create `crates/daemon/src/managed_resources/fake.rs`: test-only fake multi-port runtime adapter.
- Modify `crates/daemon/src/lib.rs`: register `managed_resources`.
- Modify `crates/daemon/src/jobs.rs`: route non-Gateway resource reconciliation to Managed Resource runtime reconciliation and make Project/System reconciliation await resource runtime prep before Project env rendering.
- Modify `crates/daemon/src/project_env.rs`: make reconciliation async and call Managed Resource reconciliation after demand planning and before env rendering.
- Modify `crates/daemon/src/error.rs`: add concise runtime adapter errors.
- Create `crates/daemon/tests/managed_resource_runtime.rs`: test-only multi-port runtime integration tests.
- Modify `crates/cli/src/commands/mod.rs`: add a private reusable artifact-resource command helper module.
- Create `crates/cli/src/commands/artifact_resource.rs`: private helper for install/update/uninstall/list commands that adapter command modules reuse.
- Modify `crates/cli/tests` or `it/cli.rs` only if helper behavior has direct observable output; otherwise leave public command snapshots to adapter PRs.

SQL foundation creates or modifies:

- Modify `Cargo.toml`: add workspace `sqlx`.
- Modify `crates/daemon/Cargo.toml`: add `sqlx` dependency.
- Create `crates/daemon/src/managed_resources/sql.rs`: shared MySQL/Postgres admin connection, readiness, database create/check, env helpers.
- Modify `crates/daemon/src/managed_resources/mod.rs`: expose SQL helpers to child modules.
- Create `crates/daemon/tests/sql_foundation.rs`: shared SQL helper tests with a recording test admin.

Each adapter PR creates or modifies:

- Modify `crates/resources/src/runtime.rs`: add the adapter layout validator function for that resource.
- Modify `crates/daemon/src/managed_resources/mod.rs`: register the adapter module in the production runtime catalog.
- Create `crates/daemon/src/managed_resources/<adapter>.rs`: runtime definition, readiness, env context, allocation admin, and adapter tests.
- Modify `crates/cli/src/args.rs`: add public command variants for the adapter namespace.
- Modify `crates/cli/src/commands/mod.rs`: route command variants.
- Create `crates/cli/src/commands/<adapter>.rs`: command namespace functions using `artifact_resource`.
- Modify `it/cli.rs`: add CLI help/namespace snapshots.
- Modify completion snapshots under `it/snapshots/` through `cargo insta`.
- Add or modify focused tests under `crates/daemon/tests/` and `crates/resources/tests/`.

## Dependency Matrix

Add dependencies only in the PR that first uses them.

Runtime foundation: no new external crates.

SQL foundation:

```toml
[workspace.dependencies]
sqlx = { version = "0.9.0", default-features = false, features = ["mysql", "postgres", "runtime-tokio", "tls-rustls"] }
```

Then:

```bash
cargo update -p sqlx --precise 0.9.0
```

Redis adapter:

```toml
[workspace.dependencies]
redis = { version = "1.2.2", default-features = false, features = ["tokio-comp"] }
```

Then:

```bash
cargo update -p redis --precise 1.2.2
```

RustFS adapter:

```toml
[workspace.dependencies]
object_store = { version = "0.13.2", default-features = false, features = ["aws"] }
aws-sdk-s3 = { version = "1.135.0", default-features = false, features = ["rustls", "rt-tokio", "http-1x"] }
```

Then:

```bash
cargo update -p object_store --precise 0.13.2
cargo update -p aws-sdk-s3 --precise 1.135.0
```

Use `sqlx::query(...)`, `sqlx::query_as(...)`, and dynamic SQL strings. Do not use `sqlx::query!` macros or offline metadata.

Use Redis async APIs with the current method names:

```rust
let client = redis::Client::open("redis://127.0.0.1:6379/")?;
let mut connection = client.get_multiplexed_async_connection().await?;
let pong: String = redis::cmd("PING").query_async(&mut connection).await?;
```

Use `object_store` only for S3 object checks. Use `aws-sdk-s3` for RustFS bucket creation:

```rust
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::{Client, Config};

let config = Config::builder()
    .behavior_version(BehaviorVersion::latest())
    .credentials_provider(Credentials::new(
        access_key,
        secret_key,
        None,
        None,
        "pv-rustfs",
    ))
    .region(Region::new("us-east-1"))
    .endpoint_url(endpoint)
    .force_path_style(true)
    .build();
let client = Client::from_conf(config);
client.create_bucket().bucket(bucket).send().await?;
```

## Task 1: Runtime Foundation

**Files:**

- Create: `crates/state/src/sql/007_resource_port_roles.sql`
- Modify: `crates/state/src/migrations.rs`
- Modify: `crates/state/src/database.rs`
- Modify: `crates/state/src/lib.rs`
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/state/tests/state_foundation.rs`
- Create: `crates/daemon/src/managed_resources/mod.rs`
- Create: `crates/daemon/src/managed_resources/fake.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/jobs.rs`
- Modify: `crates/daemon/src/project_env.rs`
- Modify: `crates/daemon/src/error.rs`
- Create: `crates/daemon/tests/managed_resource_runtime.rs`
- Create: `crates/cli/src/commands/artifact_resource.rs`
- Modify: `crates/cli/src/commands/mod.rs`

- [ ] **Step 1: Write state tests for multi-port resource owners**

Add this test shape to `crates/state/tests/state_foundation.rs` near the existing port allocator tests:

```rust
#[test]
fn resource_port_allocator_distinguishes_named_ports_for_one_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let smtp = PortRequest::resource_port("mailpit", "1.0", "smtp", 1025, 45000, 45009);
    let dashboard = PortRequest::resource_port("mailpit", "1.0", "dashboard", 8025, 45000, 45009);

    let assigned_smtp = database.assign_port(smtp.clone(), |_port| true)?;
    let assigned_dashboard = database.assign_port(dashboard.clone(), |_port| true)?;
    let reused_smtp = database.assign_port(smtp, |_port| true)?;
    let released_dashboard = database.release_port(PortOwner::Resource {
        name: "mailpit".to_string(),
        track: "1.0".to_string(),
        port: "dashboard".to_string(),
    })?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            assigned_smtp,
            assigned_dashboard,
            reused_smtp,
            released_dashboard,
            database.assigned_ports()?,
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
```

Extend `runtime_observed_state_round_trips_through_observed_states` with a resource subject:

```rust
database.record_runtime_observed_snapshot(
    RuntimeSubject::Resource {
        name: "mailpit".to_string(),
        track: "1.0".to_string(),
    },
    RuntimeObservedStatus::Running,
    Some("mailpit is ready"),
)?;
```

- [ ] **Step 2: Run the new state tests and confirm failure**

Run:

```bash
cargo nextest run -p state resource_port_allocator_distinguishes_named_ports_for_one_track --locked
cargo nextest run -p state runtime_observed_state_round_trips_through_observed_states --locked
```

Expected: compile failure for `PortRequest::resource_port`, `PortOwner::Resource { port }`, or `RuntimeSubject::Resource`.

- [ ] **Step 3: Add migration 7**

Create `crates/state/src/sql/007_resource_port_roles.sql`:

```sql
ALTER TABLE ports RENAME TO ports_without_owner_port;

CREATE TABLE ports (
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    owner_track TEXT NOT NULL,
    owner_port TEXT NOT NULL,
    port INTEGER NOT NULL UNIQUE,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (owner_kind, owner_id, owner_track, owner_port)
);

INSERT INTO ports (owner_kind, owner_id, owner_track, owner_port, port, updated_at)
SELECT
    owner_kind,
    owner_id,
    owner_track,
    CASE owner_kind WHEN 'resource' THEN 'default' ELSE '' END,
    port,
    updated_at
FROM ports_without_owner_port;

DROP TABLE ports_without_owner_port;
```

Modify `crates/state/src/migrations.rs`:

```rust
const RESOURCE_PORT_ROLES_SQL: &str = include_str!("sql/007_resource_port_roles.sql");
```

Add to `DEFAULT_MIGRATIONS` after migration 6:

```rust
Migration::new(7, "resource_port_roles", RESOURCE_PORT_ROLES_SQL),
```

- [ ] **Step 4: Extend port ownership state**

In `crates/state/src/database.rs`, change the enum to:

```rust
pub enum PortOwner {
    Dns,
    Gateway(GatewayPort),
    PhpWorker { php_track: String },
    Resource { name: String, track: String, port: String },
}
```

Add a named constructor while keeping the existing constructor as the default port:

```rust
pub fn resource(
    name: impl Into<String>,
    track: impl Into<String>,
    preferred_port: u16,
    fallback_start: u16,
    fallback_end: u16,
) -> Self {
    Self::resource_port(name, track, "default", preferred_port, fallback_start, fallback_end)
}

pub fn resource_port(
    name: impl Into<String>,
    track: impl Into<String>,
    port: impl Into<String>,
    preferred_port: u16,
    fallback_start: u16,
    fallback_end: u16,
) -> Self {
    Self::new(
        PortOwner::Resource {
            name: name.into(),
            track: track.into(),
            port: port.into(),
        },
        preferred_port,
        fallback_start,
        fallback_end,
    )
}
```

Extend `PortIdentity` with `owner_port: String`, update every query that reads or writes `ports`, and require empty `owner_port` for DNS, gateway, and PHP worker rows. For resource rows, validate `name`, `track`, and `port` with the existing managed resource identity validation rules.

Use this display text:

```rust
Self::Resource { name, track, port } => {
    format!("resource {name:?} track {track:?} port {port:?}")
}
```

- [ ] **Step 5: Extend runtime observed subjects**

In `RuntimeSubject`, add:

```rust
Resource { name: String, track: String },
```

Serialize as:

```rust
Self::Resource { name, track } => {
    validate_managed_resource_identity("name", name)?;
    validate_concrete_track(track)?;

    Ok(format!("resource:{name}:{track}"))
}
```

Parse with `strip_prefix("resource:")` and `split_once(':')`. Since resource names and tracks reject `:`, this format is unambiguous.

- [ ] **Step 6: Run state tests and accept snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -p state -- resource_port_allocator_distinguishes_named_ports_for_one_track
cargo insta test --accept --test-runner nextest -p state -- runtime_observed_state_round_trips_through_observed_states
```

Expected: both tests pass and only relevant `crates/state/tests/snapshots/*.snap` files change.

- [ ] **Step 7: Add resource runtime path helpers**

In `crates/state/src/paths.rs`, add:

```rust
pub fn resource_runtime_config(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
    self.config()
        .join(format!("resources/{resource_name}-{track}.json"))
}

pub fn resource_log(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
    self.logs()
        .join(format!("resources/{resource_name}-{track}.log"))
}

pub fn resource_pid(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
    self.run()
        .join(format!("resources/{resource_name}-{track}.pid"))
}

pub fn resource_runtime_metadata(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
    self.run()
        .join(format!("resources/{resource_name}-{track}.json"))
}

pub fn resource_data_dir(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
    self.resources()
        .join(resource_name)
        .join(track)
        .join("data")
}
```

Add a small path snapshot test if existing path summary tests cover new helpers; otherwise keep coverage in daemon runtime tests.

- [ ] **Step 8: Create daemon runtime foundation types**

Create `crates/daemon/src/managed_resources/mod.rs` with these public-to-crate types:

```rust
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use state::{Database, EnvContextValues, PvPaths, ResourceAllocationRecord};

use crate::{DaemonError, ProcessSpec, ReadinessCheck};

const RESOURCE_HOST: &str = "127.0.0.1";
const RESOURCE_READINESS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourcePortSpec {
    pub name: &'static str,
    pub preferred_port: u16,
    pub env_key: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourcePortAssignment {
    pub name: String,
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedResourceRuntimeContext {
    pub resource_name: String,
    pub track: String,
    pub artifact_path: Utf8PathBuf,
    pub data_dir: Utf8PathBuf,
    pub ports: BTreeMap<String, u16>,
}

pub(crate) trait ManagedResourceRuntimeAdapter: Send + Sync {
    fn resource_name(&self) -> &'static str;
    fn artifact_adapter(&self) -> Result<resources::RuntimeArtifactAdapter, DaemonError>;
    fn port_specs(&self) -> &'static [ManagedResourcePortSpec];
    fn build_process_spec(&self, paths: &PvPaths, context: &ManagedResourceRuntimeContext)
        -> Result<ProcessSpec, DaemonError>;
    fn readiness(&self, context: &ManagedResourceRuntimeContext) -> Result<ReadinessCheck, DaemonError>;
    fn resource_env(&self, context: &ManagedResourceRuntimeContext) -> Result<EnvContextValues, DaemonError>;
    async fn reconcile_allocations(
        &self,
        _paths: &PvPaths,
        _database: &mut Database,
        _context: &ManagedResourceRuntimeContext,
        _allocations: &[ResourceAllocationRecord],
    ) -> Result<(), DaemonError> {
        Ok(())
    }
}
```

Implement a `ManagedResourceRuntimeCatalog` that stores adapters by canonical resource name and carries install options:

```rust
#[derive(Clone, Debug)]
pub(crate) struct ManagedResourceInstallOptions {
    pub manifest_url: String,
    pub target_platform: resources::TargetPlatform,
}
```

Production uses `https://artifacts.prvious.test/manifest.json` and the current platform. Tests construct a catalog with a local fixture manifest URL and fake adapters. Production starts with no real backing-resource adapters in this task; test code can build a catalog with fake adapters.

- [ ] **Step 9: Add daemon errors**

In `crates/daemon/src/error.rs`, add:

```rust
#[error("Managed Resource runtime `{resource}` is not supported yet")]
UnsupportedManagedResourceRuntime { resource: String },

#[error("Managed Resource runtime `{resource}` track `{track}` is missing installed artifact path")]
ManagedResourceArtifactMissing { resource: String, track: String },

#[error("Managed Resource runtime `{resource}` track `{track}` is missing port `{port}`")]
ManagedResourcePortMissing {
    resource: String,
    track: String,
    port: String,
},
```

- [ ] **Step 10: Wire resource reconciliation into Project env rendering**

Change `reconcile_project_env` in `crates/daemon/src/project_env.rs` to async:

```rust
pub(crate) async fn reconcile_project_env(
    paths: &PvPaths,
    project_id: &str,
) -> Result<ProjectEnvReconciliationSummary, DaemonError>
```

After `apply_project_resource_plan(database, &project.id, &plan)?;`, call:

```rust
crate::managed_resources::reconcile_project_resources(paths, database, &project, &plan).await?;
```

Make `ProjectResourcePlan` and `ProjectResourceAllocationPlan` fields `pub(crate)` so the daemon-local runtime module can inspect demanded resources and allocations. Do not expose these types outside `daemon`.

In `crates/daemon/src/jobs.rs`, update:

```rust
let project_env_summary = reconcile_project_env(paths, id.as_str()).await?;
```

Make `reconcile_system_projects` async and await each Project reconciliation in sequence. This preserves the current ordered error reporting.

- [ ] **Step 11: Implement resource runtime reconciliation**

In `crates/daemon/src/managed_resources/mod.rs`, implement:

```rust
pub(crate) async fn reconcile_project_resources(
    paths: &PvPaths,
    database: &mut Database,
    project: &state::ProjectRecord,
    plan: &crate::project_env::ProjectResourcePlan,
) -> Result<(), DaemonError>
```

Behavior:

1. For every resource in `plan.resources`, look up a runtime adapter in the catalog.
2. If no adapter is registered, return `DaemonError::UnsupportedManagedResourceRuntime`.
3. Ensure the track is installed. If a track row exists without `current_artifact_path`, return `ManagedResourceArtifactMissing` in this task. Demand-driven install from manifests is added in Step 12.
4. Assign every named port with `PortRequest::resource_port`.
5. Build `ManagedResourceRuntimeContext`.
6. Start or adopt the process with `ProcessSupervisor`.
7. Wait for readiness.
8. Record `RuntimeSubject::Resource { name, track }` as `Running`.
9. Record resource env context with `database.record_managed_resource_track_env_context`.
10. Load desired allocations for the Project/resource/track and call `adapter.reconcile_allocations`.

Use the existing gateway runtime start/adopt pattern. PV must verify ownership before adopting or stopping.

- [ ] **Step 12: Add demand-driven install for missing tracks**

Add this helper in `managed_resources/mod.rs`:

```rust
fn install_missing_track_blocking(
    paths: PvPaths,
    resource_name: String,
    track: String,
) -> Result<(), DaemonError>
```

The helper uses:

```rust
let commands = resources::ManagedResourceCommands::new(
    paths,
    install_options.manifest_url,
    install_options.target_platform,
);
let adapter = catalog
    .adapter(&resource_name)?
    .artifact_adapter()?;
let client = resources::UreqResourceHttpClient::default();
commands.install(&adapter, resources::TrackSelector::Track(resources::TrackName::new(track)?), &client)?;
```

Run this helper from async reconciliation with `tokio::task::spawn_blocking`. The production adapter map controls which `resources::RuntimeArtifactAdapter` is available. In this task, only test adapters use this path. The fake-runtime tests must exercise both preinstalled artifacts and missing-track install from a local fixture manifest.

Add `current_target_platform()` by reusing the CLI mapping:

```rust
fn current_target_platform() -> resources::TargetPlatform {
    resources::TargetPlatform {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}
```

- [ ] **Step 13: Add test-only fake runtime**

Create `crates/daemon/src/managed_resources/fake.rs` gated with `#[cfg(test)]`. The fake adapter uses canonical resource `mailpit` and ports `smtp` and `dashboard`, but it is registered only through test catalog construction.

The fake `build_process_spec` runs `/bin/sh` with a script that writes a log line and serves two TCP listeners. Use a fixture script under a temp artifact root in the test, not a checked-in binary.

Add a test in `crates/daemon/tests/managed_resource_runtime.rs`:

```rust
#[tokio::test]
async fn demanded_resource_starts_fake_multi_port_runtime_before_env_rendering() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mailpit:
  version: "1.0"
  env:
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
    MAILPIT_DASHBOARD: "${dashboard_url}"
"#,
    )?;
    seed_fake_mailpit_artifact(&paths, "1.0")?;

    let catalog = fake_runtime_catalog()?;
    reconcile_project_env_with_catalog(&paths, &project.id, &catalog).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "demanded_resource_starts_fake_multi_port_runtime_before_env_rendering",
        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", "1.0")?,
            database.assigned_ports()?,
            database.runtime_observed_states()?,
        ),
    )?;

    Ok(())
}
```

Also add tests for readiness failure and demand removal stopping the fake runtime. The demand-removal test should rewrite Project config without `mailpit`, run reconciliation again, and assert observed state is `Stopped` for the resource subject.

- [ ] **Step 14: Add private artifact command helper**

Create `crates/cli/src/commands/artifact_resource.rs`:

```rust
use std::io::Write;
use std::process::ExitCode;

use resources::{
    ManagedResourceCommands, ManagedResourceUninstallOptions, ResourceAdapter,
    ResourceHttpClient, ResourceName, TargetPlatform, TrackName, TrackSelector,
    UreqResourceHttpClient,
};
use state::{Database, PvPaths};

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

pub(crate) struct ArtifactResourceCommandSpec {
    pub resource_name: &'static str,
    pub display_name: &'static str,
    pub adapter: fn() -> resources::Result<resources::RuntimeArtifactAdapter>,
}
```

Implement `install`, `update`, `uninstall`, and `list` functions that mirror PHP/Composer output style and call `request_system_reconciliation` after state-changing commands. Keep open/dashboard commands in each adapter module because only Mailpit and RustFS expose read-only open behavior in this wave.

Add `mod artifact_resource;` to `crates/cli/src/commands/mod.rs`.

- [ ] **Step 15: Run foundation verification**

Run:

```bash
cargo nextest run -p state -p daemon -p cli --locked resource_port_allocator_distinguishes_named_ports_for_one_track
cargo nextest run -p state -p daemon -p cli --locked demanded_resource_starts_fake_multi_port_runtime_before_env_rendering
cargo insta test --accept --test-runner nextest -p state -p daemon -- resource_port_allocator_distinguishes_named_ports_for_one_track demanded_resource_starts_fake_multi_port_runtime_before_env_rendering
cargo fmt --all -- --check
cargo clippy -p state -p daemon -p cli --all-targets --locked -- -D warnings
git diff --check
```

Expected: commands pass; snapshots change only for the new or updated tests.

- [ ] **Step 16: Commit runtime foundation**

Run:

```bash
git status --short
git add crates/state crates/daemon crates/cli
git commit -m "feat: add managed resource runtime foundation"
```

Expected: one conventional commit on `pr-16-20-runtime-foundation`.

## Task 2: SQL Foundation

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/daemon/Cargo.toml`
- Create: `crates/daemon/src/managed_resources/sql.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Create: `crates/daemon/tests/sql_foundation.rs`
- Modify: `Cargo.lock`

- [ ] **Step 1: Add SQL dependency**

Add workspace dependency:

```toml
sqlx = { version = "0.9.0", default-features = false, features = ["mysql", "postgres", "runtime-tokio", "tls-rustls"] }
```

Add daemon dependency:

```toml
sqlx = { workspace = true }
```

Run:

```bash
cargo update -p sqlx --precise 0.9.0
```

Expected: `Cargo.lock` updates for `sqlx` and its transitive dependencies only.

- [ ] **Step 2: Write SQL foundation tests**

Create `crates/daemon/tests/sql_foundation.rs` with recording-admin tests:

```rust
#[tokio::test]
async fn sql_foundation_creates_database_and_marks_allocation_ready() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = seed_project_with_desired_mysql_allocation(&paths)?;
    let mut database = Database::open(&paths)?;
    let mut admin = RecordingSqlAdmin::default();

    daemon::managed_resources::sql::ensure_database_allocation_for_test(
        &mut database,
        &mut admin,
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        SqlEngine::Mysql,
        &sql_admin_context(),
    )
    .await?;

    assert_debug_snapshot!((
        admin.operations(),
        database.resource_allocations(&project.id, "mysql")?,
    ));

    Ok(())
}
```

Add a Postgres variant and a test that leaves an already-ready allocation untouched except for verifying the database exists.

- [ ] **Step 3: Implement shared SQL helpers**

Create `crates/daemon/src/managed_resources/sql.rs`:

```rust
use state::{Database, EnvContextValues};

use crate::DaemonError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlEngine {
    Mysql,
    Postgres,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SqlAdminContext {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SqlAllocationContext {
    pub database: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}
```

Implement:

```rust
pub(crate) fn sql_resource_env(context: &SqlAdminContext, engine: SqlEngine) -> EnvContextValues
pub(crate) fn sql_allocation_env(context: &SqlAllocationContext, engine: SqlEngine) -> EnvContextValues
pub(crate) async fn ping_admin(context: &SqlAdminContext, engine: SqlEngine) -> Result<(), DaemonError>
pub(crate) async fn create_database_if_missing(context: &SqlAdminContext, engine: SqlEngine, database: &str) -> Result<(), DaemonError>
```

Use dynamic queries:

```rust
sqlx::query("SELECT 1").execute(&pool).await?;
```

For MySQL database creation, use a backtick-quoted identifier built by a helper that accepts only ASCII alphanumeric and underscore. For Postgres, use a double-quoted identifier helper with the same accepted character set. Generated allocation names already use that character set for SQL allocations.

- [ ] **Step 4: Expose SQL helpers to adapter modules**

In `crates/daemon/src/managed_resources/mod.rs`, add:

```rust
pub(crate) mod sql;
```

- [ ] **Step 5: Run SQL foundation verification**

Run:

```bash
cargo nextest run -p daemon --locked sql_foundation_creates_database_and_marks_allocation_ready
cargo insta test --accept --test-runner nextest -p daemon -- sql_foundation
cargo fmt --all -- --check
cargo clippy -p daemon --all-targets --locked -- -D warnings
git diff --check
```

Expected: daemon tests pass and snapshots cover generated SQL env values.

- [ ] **Step 6: Commit SQL foundation**

Run:

```bash
git status --short
git add Cargo.toml Cargo.lock crates/daemon
git commit -m "feat: add managed resource SQL foundation"
```

## Task 3: Mailpit Adapter PR 16

**Files:**

- Modify: `crates/resources/src/runtime.rs`
- Create: `crates/daemon/src/managed_resources/mailpit.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/cli/src/args.rs`
- Create: `crates/cli/src/commands/mailpit.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `it/cli.rs`
- Modify: `it/snapshots/*.snap`

- [ ] **Step 1: Add Mailpit layout validator**

In `crates/resources/src/runtime.rs`, add:

```rust
pub fn mailpit_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("mailpit")?,
        "bin/mailpit",
    ))
}
```

Add a resources test with a temp artifact root that passes when `bin/mailpit` exists and fails with `InvalidArtifactLayout` when missing.

- [ ] **Step 2: Add Mailpit runtime module**

Create `crates/daemon/src/managed_resources/mailpit.rs`:

```rust
const PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec { name: "smtp", preferred_port: 1025, env_key: "smtp_port" },
    ManagedResourcePortSpec { name: "dashboard", preferred_port: 8025, env_key: "dashboard_port" },
];
```

Build process arguments:

```rust
vec![
    "--smtp".to_string(),
    format!("127.0.0.1:{}", context.port("smtp")?),
    "--listen".to_string(),
    format!("127.0.0.1:{}", context.port("dashboard")?),
]
```

Readiness is HTTP on dashboard path `/`.

Resource env values:

```rust
smtp_host = "127.0.0.1"
smtp_port = smtp port as string
dashboard_url = "http://127.0.0.1:{dashboard}"
```

No allocations are reconciled for Mailpit.

- [ ] **Step 3: Add Mailpit daemon tests**

Add tests in `crates/daemon/tests/managed_resource_runtime.rs` or `crates/daemon/tests/mailpit_adapter.rs`:

```rust
#[tokio::test]
async fn mailpit_reconciliation_records_smtp_and_dashboard_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mailpit_env(&paths, &tempdir.path().join("project"))?;
    seed_mailpit_fixture_artifact(&paths, "1.0")?;

    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "mailpit_reconciliation_records_smtp_and_dashboard_env",
        (
            read_dotenv(&project)?,
            database.managed_resource_track("mailpit", "1.0")?,
            database.runtime_observed_states()?,
        ),
    )?;

    Ok(())
}
```

The fixture `bin/mailpit` must be test-only and must implement the two expected listen ports.

Add a second daemon test named `mailpit_project_demand_installs_missing_fixture_track_before_start`. It should seed only a local fixture manifest and artifact archive, not a preinstalled track row. Project reconciliation must install the fixture track, start Mailpit, record env context, and render `.env`.

- [ ] **Step 4: Add Mailpit CLI namespace**

In `crates/cli/src/args.rs`, add variants for:

```text
mailpit:install [version]
mailpit:update
mailpit:uninstall <version> [--prune] [--force]
mailpit:list
mailpit:open
mail:install [version]
mail:update
mail:uninstall <version> [--prune] [--force]
mail:list
mail:open
```

Use the existing PHP argument structs where the field names match; create `MailpitInstallArgs` and `MailpitUninstallArgs` only if clap help text needs resource-specific wording.

Create `crates/cli/src/commands/mailpit.rs`. Install/update/uninstall/list call `artifact_resource`. `open` reads `managed_resource_tracks.env_json`; it opens only when `dashboard_url` exists and the observed runtime subject for `mailpit` track is `Running`. If not running, print:

```text
Mailpit is not running for any linked Project
```

Do not enqueue reconciliation from `mailpit:open` or `mail:open`.

- [ ] **Step 5: Add Mailpit CLI snapshots**

In `it/cli.rs`, add:

```rust
#[test]
fn mailpit_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["mailpit:install", "--help"])?,
        run_pv(&["mailpit:update", "--help"])?,
        run_pv(&["mailpit:uninstall", "--help"])?,
        run_pv(&["mailpit:list", "--help"])?,
        run_pv(&["mailpit:open", "--help"])?,
        run_pv(&["mail:open", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}
```

Run:

```bash
cargo insta test --accept --test-runner nextest --test cli -- mailpit_commands_are_documented
```

- [ ] **Step 6: Verify and commit Mailpit**

Run:

```bash
cargo nextest run -p resources -p daemon --locked mailpit
cargo insta test --accept --test-runner nextest -p daemon -- mailpit
cargo insta test --accept --test-runner nextest --test cli -- mailpit_commands_are_documented
cargo fmt --all -- --check
cargo clippy -p resources -p daemon -p cli --all-targets --locked -- -D warnings
git diff --check
git add crates/resources crates/daemon crates/cli it
git commit -m "feat: add Mailpit managed resource adapter"
```

## Task 4: Redis Adapter PR 17

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/daemon/Cargo.toml`
- Modify: `crates/resources/src/runtime.rs`
- Create: `crates/daemon/src/managed_resources/redis.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/cli/src/args.rs`
- Create: `crates/cli/src/commands/redis.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `it/cli.rs`
- Modify: `it/snapshots/*.snap`

- [ ] **Step 1: Add Redis dependency and layout validator**

Add workspace and daemon dependencies as shown in the dependency matrix, then run:

```bash
cargo update -p redis --precise 1.2.2
```

In `crates/resources/src/runtime.rs`, add:

```rust
pub fn redis_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("redis")?,
        "bin/redis-server",
    ))
}
```

- [ ] **Step 2: Add Redis runtime module**

Create `crates/daemon/src/managed_resources/redis.rs`.

Port spec:

```rust
const PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec { name: "redis", preferred_port: 6379, env_key: "port" },
];
```

Process arguments:

```rust
vec![
    "--bind".to_string(),
    "127.0.0.1".to_string(),
    "--port".to_string(),
    context.port("redis")?.to_string(),
    "--dir".to_string(),
    context.data_dir.as_str().to_string(),
    "--save".to_string(),
    String::new(),
    "--appendonly".to_string(),
    "no".to_string(),
]
```

Readiness uses Redis PING:

```rust
let url = format!("redis://127.0.0.1:{}/", context.port("redis")?);
let client = redis::Client::open(url)?;
let mut connection = client.get_multiplexed_async_connection().await?;
let pong: String = redis::cmd("PING").query_async(&mut connection).await?;
if pong != "PONG" {
    return Err(DaemonError::DaemonRejected {
        message: format!("Redis PING returned {pong}"),
    });
}
```

Resource env:

```text
host=127.0.0.1
port=<redis port>
url=redis://127.0.0.1:<redis port>/0
```

Allocation env for each desired allocation:

```text
host=127.0.0.1
port=<redis port>
prefix=<resource_allocations.generated_name>
url=redis://127.0.0.1:<redis port>/0
```

Mark each desired allocation ready after PING succeeds. Redis v1 does not create logical databases or ACL users.

- [ ] **Step 3: Add Redis tests**

Add daemon test:

```rust
#[tokio::test]
async fn redis_reconciliation_marks_prefix_allocation_ready_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_redis_allocation_env(&paths, &tempdir.path().join("project"))?;
    seed_redis_fixture_artifact(&paths, "7.2")?;

    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "redis_reconciliation_marks_prefix_allocation_ready_and_renders_env",
        (
            read_dotenv(&project)?,
            database.managed_resource_track("redis", "7.2")?,
            database.resource_allocations(&project.id, "redis")?,
        ),
    )?;

    Ok(())
}
```

The fixture `bin/redis-server` must accept the arguments above and implement PING.

Add a second daemon test named `redis_project_demand_installs_missing_fixture_track_before_start`. It should seed only a local fixture manifest and artifact archive, not a preinstalled track row. Project reconciliation must install the fixture track, start Redis, mark the prefix allocation ready, and render `.env`.

- [ ] **Step 4: Add Redis CLI namespace**

Add public commands:

```text
redis:install [version]
redis:update
redis:uninstall <version> [--prune] [--force]
redis:list
```

Create `crates/cli/src/commands/redis.rs` using the private `artifact_resource` helper. Add `redis_commands_are_documented` in `it/cli.rs`.

- [ ] **Step 5: Verify and commit Redis**

Run:

```bash
cargo nextest run -p resources -p daemon --locked redis
cargo insta test --accept --test-runner nextest -p daemon -- redis
cargo insta test --accept --test-runner nextest --test cli -- redis_commands_are_documented
cargo fmt --all -- --check
cargo clippy -p resources -p daemon -p cli --all-targets --locked -- -D warnings
git diff --check
git add Cargo.toml Cargo.lock crates/resources crates/daemon crates/cli it
git commit -m "feat: add Redis managed resource adapter"
```

## Task 5: RustFS Adapter PR 20

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/daemon/Cargo.toml`
- Modify: `crates/resources/src/runtime.rs`
- Create: `crates/daemon/src/managed_resources/rustfs.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/cli/src/args.rs`
- Create: `crates/cli/src/commands/rustfs.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `it/cli.rs`
- Modify: `it/snapshots/*.snap`

- [ ] **Step 1: Add RustFS dependencies and layout validator**

Add workspace and daemon dependencies as shown in the dependency matrix, then run:

```bash
cargo update -p object_store --precise 0.13.2
cargo update -p aws-sdk-s3 --precise 1.135.0
```

In `crates/resources/src/runtime.rs`, add:

```rust
pub fn rustfs_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("rustfs")?,
        "bin/rustfs",
    ))
}
```

- [ ] **Step 2: Add RustFS runtime module**

Port specs:

```rust
const PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec { name: "api", preferred_port: 9000, env_key: "port" },
    ManagedResourcePortSpec { name: "console", preferred_port: 9001, env_key: "console_port" },
];
```

Generate stable access keys when `managed_resource_tracks.env_json` is empty. Use values:

```text
access_key=pv-rustfs
secret_key=<generated 32 character hex string>
```

Process arguments:

```rust
vec![
    "--address".to_string(),
    format!("127.0.0.1:{}", context.port("api")?),
    "--console-address".to_string(),
    format!("127.0.0.1:{}", context.port("console")?),
    context.data_dir.as_str().to_string(),
]
```

Readiness is HTTP on the API port path `/`.

Resource env:

```text
access_key=<stable access key>
secret_key=<stable secret key>
endpoint=http://127.0.0.1:<api port>
host=127.0.0.1
port=<api port>
url=http://127.0.0.1:<api port>
```

Allocation behavior:

1. Create bucket with `aws-sdk-s3` `create_bucket`.
2. Treat already-existing bucket as success.
3. Verify object operations by creating a short object with `object_store::aws::AmazonS3Builder` and `ObjectStoreExt::put` followed by `head`.
4. Mark allocation ready with `bucket`, `access_key`, `secret_key`, `endpoint`, `host`, `port`, and `url`.

- [ ] **Step 3: Add RustFS tests**

Add daemon test:

```rust
#[tokio::test]
async fn rustfs_reconciliation_creates_bucket_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_rustfs_bucket_env(&paths, &tempdir.path().join("project"))?;
    seed_rustfs_fixture_artifact(&paths, "1.0")?;

    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "rustfs_reconciliation_creates_bucket_and_renders_env",
        (
            read_dotenv(&project)?,
            database.managed_resource_track("rustfs", "1.0")?,
            database.resource_allocations(&project.id, "rustfs")?,
        ),
    )?;

    Ok(())
}
```

The fixture `bin/rustfs` must accept the arguments above, expose the API and console ports, and implement enough S3-compatible behavior for create-bucket, put, and head.

Add a second daemon test named `rustfs_project_demand_installs_missing_fixture_track_before_start`. It should seed only a local fixture manifest and artifact archive, not a preinstalled track row. Project reconciliation must install the fixture track, start RustFS, create the bucket, mark the allocation ready, and render `.env`.

- [ ] **Step 4: Add RustFS CLI namespace**

Add public commands:

```text
rustfs:install [version]
rustfs:update
rustfs:uninstall <version> [--prune] [--force]
rustfs:list
rustfs:open
s3:install [version]
s3:update
s3:uninstall <version> [--prune] [--force]
s3:list
s3:open
```

`rustfs:open` and `s3:open` read the resource env and observed state only. They do not install, start, or enqueue reconciliation. When not running, print:

```text
RustFS is not running for any linked Project
```

- [ ] **Step 5: Verify and commit RustFS**

Run:

```bash
cargo nextest run -p resources -p daemon --locked rustfs
cargo insta test --accept --test-runner nextest -p daemon -- rustfs
cargo insta test --accept --test-runner nextest --test cli -- rustfs_commands_are_documented
cargo fmt --all -- --check
cargo clippy -p resources -p daemon -p cli --all-targets --locked -- -D warnings
git diff --check
git add Cargo.toml Cargo.lock crates/resources crates/daemon crates/cli it
git commit -m "feat: add RustFS managed resource adapter"
```

## Task 6: MySQL Adapter PR 18

**Files:**

- Modify: `crates/resources/src/runtime.rs`
- Create: `crates/daemon/src/managed_resources/mysql.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/cli/src/args.rs`
- Create: `crates/cli/src/commands/mysql.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `it/cli.rs`
- Modify: `it/snapshots/*.snap`

- [ ] **Step 1: Add MySQL layout validator**

In `crates/resources/src/runtime.rs`, add:

```rust
pub fn mysql_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("mysql")?,
        "bin/mysqld",
    ))
}
```

The adapter module may also require `bin/mysqladmin` if readiness uses that binary. The preferred path is `sqlx` readiness, so do not require `mysqladmin` unless implementation proves `mysqld` startup cannot be tested without it.

- [ ] **Step 2: Add MySQL runtime module**

Port spec:

```rust
const PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec { name: "mysql", preferred_port: 3306, env_key: "port" },
];
```

Generate stable admin credentials when env is empty:

```text
username=pv_root
password=<generated 32 character hex string>
```

Process startup:

1. Create data dir.
2. If the data dir has no initialized MySQL system database, run `bin/mysqld --initialize-insecure --datadir <data_dir> --basedir <artifact_path>`.
3. Start `bin/mysqld` with `--no-defaults`, `--datadir`, `--bind-address=127.0.0.1`, `--port`, and a socket path under `.pv/run/resources/mysql-<track>.sock`.
4. Use `sql::ping_admin` for readiness.
5. Store resource env with SQL foundation `sql_resource_env`.

Allocation behavior:

1. Use `sql::create_database_if_missing(SqlEngine::Mysql, generated_name)`.
2. Mark allocation ready with SQL foundation `sql_allocation_env`.

- [ ] **Step 3: Add MySQL tests**

Add daemon test:

```rust
#[tokio::test]
async fn mysql_reconciliation_creates_database_allocation_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_mysql_database_env(&paths, &tempdir.path().join("project"))?;
    seed_mysql_fixture_artifact(&paths, "8.0")?;

    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "mysql_reconciliation_creates_database_allocation_and_renders_env",
        (
            read_dotenv(&project)?,
            database.managed_resource_track("mysql", "8.0")?,
            database.resource_allocations(&project.id, "mysql")?,
        ),
    )?;

    Ok(())
}
```

The fixture `bin/mysqld` must simulate initialization, listen on the requested port, and allow the SQL foundation test admin seam to record create-database behavior without requiring real MySQL.

Add a second daemon test named `mysql_project_demand_installs_missing_fixture_track_before_start`. It should seed only a local fixture manifest and artifact archive, not a preinstalled track row. Project reconciliation must install the fixture track, start MySQL, create the database allocation through the SQL helper, and render `.env`.

- [ ] **Step 4: Add MySQL CLI namespace**

Add public commands:

```text
mysql:install [version]
mysql:update
mysql:uninstall <version> [--prune] [--force]
mysql:list
```

Create `crates/cli/src/commands/mysql.rs` using `artifact_resource`. Add `mysql_commands_are_documented` in `it/cli.rs`.

- [ ] **Step 5: Verify and commit MySQL**

Run:

```bash
cargo nextest run -p resources -p daemon --locked mysql
cargo insta test --accept --test-runner nextest -p daemon -- mysql
cargo insta test --accept --test-runner nextest --test cli -- mysql_commands_are_documented
cargo fmt --all -- --check
cargo clippy -p resources -p daemon -p cli --all-targets --locked -- -D warnings
git diff --check
git add crates/resources crates/daemon crates/cli it
git commit -m "feat: add MySQL managed resource adapter"
```

## Task 7: Postgres Adapter PR 19

**Files:**

- Modify: `crates/resources/src/runtime.rs`
- Create: `crates/daemon/src/managed_resources/postgres.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/cli/src/args.rs`
- Create: `crates/cli/src/commands/postgres.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `it/cli.rs`
- Modify: `it/snapshots/*.snap`

- [ ] **Step 1: Add Postgres layout validator**

In `crates/resources/src/runtime.rs`, add:

```rust
pub fn postgres_adapter() -> Result<RuntimeArtifactAdapter> {
    Ok(RuntimeArtifactAdapter::new(
        ResourceName::new("postgres")?,
        "bin/postgres",
    ))
}
```

Require `bin/initdb` as an additional file in `validate_installation` for Postgres. This can be done by creating a Postgres-specific adapter struct if the generic single-executable adapter is too narrow.

- [ ] **Step 2: Add Postgres runtime module**

Port spec:

```rust
const PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec { name: "postgres", preferred_port: 5432, env_key: "port" },
];
```

Generate stable admin credentials when env is empty:

```text
username=pv_root
password=<generated 32 character hex string>
```

Process startup:

1. Create data dir.
2. If the data dir lacks `PG_VERSION`, run `bin/initdb -D <data_dir> --username pv_root --pwfile <temporary password file> --auth-host=scram-sha-256 --auth-local=trust`.
3. Start `bin/postgres -D <data_dir> -h 127.0.0.1 -p <port>`.
4. Use `sql::ping_admin` for readiness.
5. Store resource env with SQL foundation `sql_resource_env`.

Allocation behavior:

1. Use `sql::create_database_if_missing(SqlEngine::Postgres, generated_name)`.
2. Mark allocation ready with SQL foundation `sql_allocation_env`.

- [ ] **Step 3: Add Postgres tests**

Add daemon test:

```rust
#[tokio::test]
async fn postgres_reconciliation_creates_database_allocation_and_renders_env() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project_with_postgres_database_env(&paths, &tempdir.path().join("project"))?;
    seed_postgres_fixture_artifact(&paths, "16")?;

    run_project_reconciliation(&paths, &project).await?;
    let database = Database::open(&paths)?;

    assert_with_normalized_timestamps(
        "postgres_reconciliation_creates_database_allocation_and_renders_env",
        (
            read_dotenv(&project)?,
            database.managed_resource_track("postgres", "16")?,
            database.resource_allocations(&project.id, "postgres")?,
        ),
    )?;

    Ok(())
}
```

The fixture `bin/postgres` and `bin/initdb` must simulate initialization and the requested port.

Add a second daemon test named `postgres_project_demand_installs_missing_fixture_track_before_start`. It should seed only a local fixture manifest and artifact archive, not a preinstalled track row. Project reconciliation must install the fixture track, start Postgres, create the database allocation through the SQL helper, and render `.env`.

- [ ] **Step 4: Add Postgres CLI namespace**

Add public commands:

```text
postgres:install [version]
postgres:update
postgres:uninstall <version> [--prune] [--force]
postgres:list
pg:install [version]
pg:update
pg:uninstall <version> [--prune] [--force]
pg:list
```

Create `crates/cli/src/commands/postgres.rs` using `artifact_resource`. Add `postgres_commands_are_documented` in `it/cli.rs`.

- [ ] **Step 5: Verify and commit Postgres**

Run:

```bash
cargo nextest run -p resources -p daemon --locked postgres
cargo insta test --accept --test-runner nextest -p daemon -- postgres
cargo insta test --accept --test-runner nextest --test cli -- postgres_commands_are_documented
cargo fmt --all -- --check
cargo clippy -p resources -p daemon -p cli --all-targets --locked -- -D warnings
git diff --check
git add crates/resources crates/daemon crates/cli it
git commit -m "feat: add Postgres managed resource adapter"
```

## Task 8: Final Orchestration Check

**Files:**

- No required source edits.
- Solo work items and scratchpad are updated outside git.

- [ ] **Step 1: Convert the final plan into Solo work items**

Use the existing Solo project and shared scratchpad. Update or create work items with these titles:

```text
Shared Managed Resource runtime foundation
Shared SQL foundation
PR 16 Mailpit adapter
PR 17 Redis adapter
PR 18 MySQL adapter
PR 19 Postgres adapter
PR 20 RustFS adapter
```

Set blockers:

```text
SQL foundation -> runtime foundation
Mailpit -> runtime foundation
Redis -> runtime foundation
RustFS -> runtime foundation
MySQL -> runtime foundation, SQL foundation
Postgres -> runtime foundation, SQL foundation
```

Add each worktree path and branch name to the item body after the worktree is created.

- [ ] **Step 2: Review branch independence**

Run in each worktree:

```bash
git status --short --branch
git log --oneline --decorate --max-count=5
```

Expected:

- Runtime foundation has one feature commit on top of the plan base.
- SQL foundation contains runtime foundation plus one SQL commit.
- Mailpit, Redis, and RustFS contain runtime foundation plus their adapter commit.
- MySQL and Postgres contain runtime foundation, SQL foundation, and their adapter commit.

- [ ] **Step 3: Run integration surface checks before PR handoff**

After all adapter branches are complete, run the broad but bounded check from the branch at the top of the stack:

```bash
cargo nextest run -p daemon -p resources -p state -p config -p cli --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
git diff --check
```

Expected: all commands pass. If this check fails from branch stacking, fix the earliest affected branch and rebase dependent worktrees.

## Final PR Acceptance Criteria

Runtime foundation:

- Resource port assignments support named ports for one resource/track.
- Resource runtime observed subjects round-trip through state.
- Project reconciliation can start/adopt/stop a test-only multi-port backing runtime before `.env` rendering.
- No public fake runtime appears in CLI help, shell completions, registry examples, or manifests.

SQL foundation:

- Shared SQL helpers cover MySQL and Postgres URL/env generation.
- Dynamic `sqlx` queries are used.
- No MySQL or Postgres adapter is implemented in the SQL foundation PR.

Each adapter:

- Adds its own public command namespace.
- Installs, updates, uninstalls, and lists artifacts through the shared command helper.
- Starts only from Project config demand.
- Installs a missing demanded track from a fixture manifest during Project reconciliation tests.
- Uses fixture artifacts for PR tests.
- Records resource env context before Project `.env` rendering.
- Marks allocations ready when the adapter supports allocations.
- Keeps `mailpit:open` and `rustfs:open` read-only.

## Documentation Links Used For Dependencies

- `sqlx` docs: https://docs.rs/sqlx/latest/sqlx/
- `redis` docs: https://docs.rs/redis/
- `object_store` docs: https://docs.rs/object_store/0.13.2/
- `aws-sdk-s3` docs: https://docs.rs/aws-sdk-s3/1.135.0/
