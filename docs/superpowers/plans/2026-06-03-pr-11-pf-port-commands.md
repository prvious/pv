# PR 11 pf Port Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build PR 11 by preparing PV-owned macOS `pf` redirect artifacts and adding non-privileged `pv ports:*` commands for install preparation, status, and uninstall preparation.

**Architecture:** The `state` crate persists distinct Gateway HTTP and HTTPS high-port assignments with an all-or-nothing helper. The `macos` crate owns `pf` rendering, parsing, read-only file inspection, and socket-table conflict detection. The CLI wires those helpers into human-output-only `ports:status`, `ports:install`, and `ports:uninstall` commands without writing `/etc`, invoking `pfctl`, or printing raw privileged command snippets.

**Tech Stack:** Rust, clap, rusqlite transactions, `state::fs`, `insta`, `cargo nextest`, and `netstat-esr 0.8.1` aliased as `netstat` for read-only socket-table inspection.

---

## File Structure

- Modify `Cargo.toml`: add a workspace dependency alias for `netstat-esr 0.8.1`.
- Modify `Cargo.lock`: add the precise dependency resolution from `cargo update -p netstat-esr --precise 0.8.1`.
- Modify `crates/macos/Cargo.toml`: depend on the workspace `netstat` alias.
- Modify `crates/state/src/paths.rs`: add prepared `pf` artifact path helpers.
- Modify `crates/state/src/database.rs`: add Gateway HTTP/HTTPS port constants, owner identities, requests, and a transactional assignment helper.
- Modify `crates/state/src/lib.rs`: export the new Gateway port types/constants.
- Modify `crates/state/tests/state_foundation.rs`: add focused Gateway port assignment coverage.
- Modify `crates/macos/src/lib.rs`: add `pf` config render/parse/inspect helpers and socket-table conflict helper.
- Modify `crates/macos/tests/resolver_config.rs`: extend nearby macOS integration tests with `pf` snapshots.
- Modify `crates/cli/src/environment.rs`: add injectable `pf` system paths and TCP listener checks.
- Modify `crates/cli/src/args.rs`: add `ports:status`, `ports:install`, and `ports:uninstall`.
- Modify `crates/cli/src/commands/mod.rs`: route the new commands.
- Create `crates/cli/src/commands/ports.rs`: implement the ports command handlers.
- Create `crates/cli/tests/ports.rs`: add CLI integration snapshots for the new command surface.
- Modify `IMPLEMENTATION.md`: after opening the PR, mark PR 11 as `Yes (#<pr-number>)`.

## Task 1: State Gateway Port Assignments

**Files:**
- Modify: `crates/state/src/paths.rs`
- Modify: `crates/state/src/database.rs`
- Modify: `crates/state/src/lib.rs`
- Modify: `crates/state/tests/state_foundation.rs`

- [ ] **Step 1: Add failing tests for prepared `pf` paths and Gateway port assignment**

Append these tests near the existing port allocator tests in `crates/state/tests/state_foundation.rs`:

```rust
#[test]
fn pv_paths_include_prepared_pf_artifacts() {
    let paths = PvPaths::for_home("/Users/alice");

    assert_eq!(
        paths.pf_anchor_config().as_str(),
        "/Users/alice/.pv/config/pf/com.prvious.pv"
    );
    assert_eq!(
        paths.pf_conf_reference_config().as_str(),
        "/Users/alice/.pv/config/pf/pf.conf"
    );
}

#[test]
fn gateway_port_allocator_persists_distinct_http_and_https_assignments() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned = database.assign_gateway_ports(|port| {
        port == GATEWAY_HTTP_PREFERRED_PORT || port == GATEWAY_HTTPS_PREFERRED_PORT
    })?;
    let reused = database.assign_gateway_ports(|_port| false)?;

    assert_eq!(assigned.http.port, GATEWAY_HTTP_PREFERRED_PORT);
    assert_eq!(assigned.https.port, GATEWAY_HTTPS_PREFERRED_PORT);
    assert_eq!(assigned.http.owner, PortOwner::Gateway(GatewayPort::Http));
    assert_eq!(assigned.https.owner, PortOwner::Gateway(GatewayPort::Https));
    assert_eq!(reused.http.port, assigned.http.port);
    assert_eq!(reused.https.port, assigned.https.port);

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((assigned, reused, database.assigned_ports()?));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn gateway_port_allocator_uses_fallbacks_when_preferred_ports_are_unavailable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned = database.assign_gateway_ports(|port| {
        port != GATEWAY_HTTP_PREFERRED_PORT && port != GATEWAY_HTTPS_PREFERRED_PORT
    })?;

    assert_eq!(assigned.http.port, RUNTIME_PORT_FALLBACK_START);
    assert_eq!(assigned.https.port, RUNTIME_PORT_FALLBACK_START + 1);

    Ok(())
}

#[test]
fn gateway_port_allocator_rolls_back_when_https_cannot_be_assigned() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let error = database
        .assign_gateway_ports(|port| port == GATEWAY_HTTP_PREFERRED_PORT)
        .expect_err("HTTPS assignment should fail");

    assert!(matches!(
        error,
        StateError::NoAvailablePort {
            name,
            preferred_port: GATEWAY_HTTPS_PREFERRED_PORT,
            fallback_start: RUNTIME_PORT_FALLBACK_START,
            fallback_end: RUNTIME_PORT_FALLBACK_END,
            ..
        } if name == "gateway https"
    ));
    assert_eq!(database.assigned_ports()?, Vec::new());

    Ok(())
}
```

Also add these imports to the existing top-level import list:

```rust
use state::{
    Database, EnvContextValues, GATEWAY_HTTP_PREFERRED_PORT, GATEWAY_HTTPS_PREFERRED_PORT,
    GatewayPort, JobStatus, ManagedResourceDesiredState, PortOwner, PortRequest, PvPaths,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, StateError,
};
```

- [ ] **Step 2: Run the focused failing state tests**

Run:

```bash
cargo nextest run -E 'test(pv_paths_include_prepared_pf_artifacts) or test(gateway_port_allocator_persists_distinct_http_and_https_assignments) or test(gateway_port_allocator_uses_fallbacks_when_preferred_ports_are_unavailable) or test(gateway_port_allocator_rolls_back_when_https_cannot_be_assigned)'
```

Expected: compile failure for missing path helpers, constants, `GatewayPort`, and `Database::assign_gateway_ports`.

- [ ] **Step 3: Add prepared `pf` path helpers**

In `crates/state/src/paths.rs`, add these methods inside `impl PvPaths` after `resolver_config()`:

```rust
pub fn pf_anchor_config(&self) -> Utf8PathBuf {
    self.config().join("pf/com.prvious.pv")
}

pub fn pf_conf_reference_config(&self) -> Utf8PathBuf {
    self.config().join("pf/pf.conf")
}
```

- [ ] **Step 4: Add Gateway owner types and constants**

In `crates/state/src/database.rs`, add constants next to `DNS_PREFERRED_PORT`:

```rust
pub const GATEWAY_HTTP_PREFERRED_PORT: u16 = 48080;
pub const GATEWAY_HTTPS_PREFERRED_PORT: u16 = 48443;
```

Replace the `Gateway` variant with a structured variant and add the protocol enum:

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GatewayPort {
    Http,
    Https,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortOwner {
    Dns,
    Gateway(GatewayPort),
    ProjectWorker {
        project_id: String,
        php_track: String,
    },
    Resource {
        name: String,
        track: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayPortAssignments {
    pub http: PortAssignment,
    pub https: PortAssignment,
}
```

- [ ] **Step 5: Add Gateway port requests**

In `impl PortRequest`, replace the old `gateway()` helper with:

```rust
pub fn gateway(
    gateway_port: GatewayPort,
    preferred_port: u16,
    fallback_start: u16,
    fallback_end: u16,
) -> Self {
    Self::new(
        PortOwner::Gateway(gateway_port),
        preferred_port,
        fallback_start,
        fallback_end,
    )
}

pub fn pv_gateway_http() -> Self {
    Self::gateway(
        GatewayPort::Http,
        GATEWAY_HTTP_PREFERRED_PORT,
        RUNTIME_PORT_FALLBACK_START,
        RUNTIME_PORT_FALLBACK_END,
    )
}

pub fn pv_gateway_https() -> Self {
    Self::gateway(
        GatewayPort::Https,
        GATEWAY_HTTPS_PREFERRED_PORT,
        RUNTIME_PORT_FALLBACK_START,
        RUNTIME_PORT_FALLBACK_END,
    )
}
```

- [ ] **Step 6: Update Gateway database identities**

In `impl PortOwner`, update `identity()`, `from_database()`, and `display_name()` with these Gateway arms:

```rust
Self::Gateway(gateway_port) => Ok(PortIdentity {
    owner_kind: "gateway",
    owner_id: gateway_port.as_str().to_string(),
    owner_track: String::new(),
}),
```

```rust
"gateway" if owner_track.is_empty() => GatewayPort::from_database(&owner_id)
    .map(Self::Gateway),
"gateway" => Err(StateError::InvalidPortOwner {
    owner: describe_port_identity(&owner_kind, &owner_id, &owner_track),
    reason: "gateway ports must use owner id `http` or `https` and an empty owner track",
}),
```

```rust
Self::Gateway(gateway_port) => format!("gateway {}", gateway_port.as_str()),
```

Add this impl near `impl PortOwner`:

```rust
impl GatewayPort {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    fn from_database(owner_id: &str) -> Result<Self, StateError> {
        match owner_id {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            _ => Err(StateError::InvalidPortOwner {
                owner: format!("gateway:{owner_id}:"),
                reason: "gateway ports must use owner id `http` or `https` and an empty owner track",
            }),
        }
    }
}
```

- [ ] **Step 7: Add transactional Gateway assignment helper**

In `impl Database`, add:

```rust
pub fn assign_gateway_ports(
    &mut self,
    mut is_available: impl FnMut(u16) -> bool,
) -> Result<GatewayPortAssignments, StateError> {
    let transaction = self
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut assigned_ports = assigned_port_numbers_in_transaction(&transaction)?;
    let http = assign_port_in_transaction(
        &transaction,
        PortRequest::pv_gateway_http(),
        &mut assigned_ports,
        &mut is_available,
    )?;
    let https = assign_port_in_transaction(
        &transaction,
        PortRequest::pv_gateway_https(),
        &mut assigned_ports,
        &mut is_available,
    )?;

    transaction.commit()?;

    Ok(GatewayPortAssignments { http, https })
}
```

Add these helpers near the existing port helper functions:

```rust
fn assign_port_in_transaction(
    transaction: &Transaction<'_>,
    request: PortRequest,
    assigned_ports: &mut BTreeSet<u16>,
    is_available: &mut impl FnMut(u16) -> bool,
) -> Result<PortAssignment, StateError> {
    let identity = request.owner.identity()?;

    if let Some(existing) = port_assignment_in_transaction(transaction, &identity)? {
        assigned_ports.insert(existing.port);
        return Ok(existing);
    }

    for candidate in request.candidates() {
        if assigned_ports.contains(&candidate) || !is_available(candidate) {
            continue;
        }

        let updated_at = timestamp()?;
        upsert_port_in_transaction(transaction, &identity, candidate, &updated_at)?;
        assigned_ports.insert(candidate);

        return Ok(PortAssignment {
            owner: request.owner,
            port: candidate,
            updated_at,
        });
    }

    Err(StateError::NoAvailablePort {
        name: request.name(),
        preferred_port: request.preferred_port,
        fallback_start: request.fallback_start,
        fallback_end: request.fallback_end,
        attempts: request.candidates().len(),
    })
}

fn assigned_port_numbers_in_transaction(
    transaction: &Transaction<'_>,
) -> Result<BTreeSet<u16>, StateError> {
    let mut statement = transaction.prepare("SELECT port FROM ports")?;
    let rows = statement.query_map([], |row| row.get::<_, u16>(0))?;
    let mut assigned_ports = BTreeSet::new();

    for row in rows {
        assigned_ports.insert(row?);
    }

    Ok(assigned_ports)
}
```

- [ ] **Step 8: Export the new state symbols**

In `crates/state/src/lib.rs`, extend the `pub use database::{ ... }` list:

```rust
GATEWAY_HTTP_PREFERRED_PORT, GATEWAY_HTTPS_PREFERRED_PORT, GatewayPort,
GatewayPortAssignments,
```

- [ ] **Step 9: Run and accept focused state snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- gateway_port_allocator_persists_distinct_http_and_https_assignments
cargo nextest run -E 'test(pv_paths_include_prepared_pf_artifacts) or test(gateway_port_allocator_persists_distinct_http_and_https_assignments) or test(gateway_port_allocator_uses_fallbacks_when_preferred_ports_are_unavailable) or test(gateway_port_allocator_rolls_back_when_https_cannot_be_assigned)'
```

Expected: all selected tests pass.

- [ ] **Step 10: Commit state changes**

Run:

```bash
git add crates/state/src/paths.rs crates/state/src/database.rs crates/state/src/lib.rs crates/state/tests/state_foundation.rs crates/state/tests/snapshots
git commit -m "feat(state): add gateway port assignments"
```

## Task 2: macOS pf Rendering, Inspection, and Socket Detection

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/macos/Cargo.toml`
- Modify: `crates/macos/src/lib.rs`
- Modify: `crates/macos/tests/resolver_config.rs`

- [ ] **Step 1: Add the socket-table dependency precisely**

In the workspace `Cargo.toml`, add:

```toml
netstat = { package = "netstat-esr", version = "0.8.1" }
```

In `crates/macos/Cargo.toml`, add:

```toml
netstat = { workspace = true }
```

Run:

```bash
cargo update -p netstat-esr --precise 0.8.1
```

Expected: `Cargo.lock` gains `netstat-esr 0.8.1` and its minimal transitive dependencies.

- [ ] **Step 2: Add failing macOS `pf` tests**

Append these tests in `crates/macos/tests/resolver_config.rs` after the resolver inspection test:

```rust
#[test]
fn pf_config_renders_pv_owned_anchor_and_pf_conf_reference() {
    let config = PfRedirectConfig::new(48080, 48443);
    let anchor = config.render_anchor();
    let reference = PfConfReference::default().render();

    assert_eq!(PfRedirectConfig::parse_anchor(&anchor), Some(config));
    assert_eq!(
        PfConfReference::parse_block(&reference),
        Some(PfConfReference::default())
    );
    assert_debug_snapshot!((anchor, reference));
}

#[test]
fn pf_anchor_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current-anchor");
    let stale_path = tempdir.path().join("stale-anchor");
    let malformed_path = tempdir.path().join("malformed-anchor");
    let conflict_path = tempdir.path().join("conflict-anchor");
    let unreadable_path = tempdir.path().join("anchor-directory");
    let expected = PfRedirectConfig::new(48080, 48443);

    fs::write_sensitive_file(&current_path, &expected.render_anchor())?;
    fs::write_sensitive_file(&stale_path, &PfRedirectConfig::new(45000, 45001).render_anchor())?;
    fs::write_sensitive_file(&malformed_path, "# Managed by PV\npass in all\n")?;
    fs::write_sensitive_file(&conflict_path, "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_pf_anchor_file(&tempdir.path().join("missing-anchor"), Some(&expected)),
        inspect_pf_anchor_file(&current_path, Some(&expected)),
        inspect_pf_anchor_file(&stale_path, Some(&expected)),
        inspect_pf_anchor_file(&malformed_path, Some(&expected)),
        inspect_pf_anchor_file(&conflict_path, Some(&expected)),
        inspect_pf_anchor_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

#[test]
fn pf_conf_reference_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()>
{
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current-pf-conf");
    let stale_path = tempdir.path().join("stale-pf-conf");
    let conflict_path = tempdir.path().join("conflict-pf-conf");
    let unrelated_path = tempdir.path().join("unrelated-pf-conf");
    let unreadable_path = tempdir.path().join("pf-conf-directory");
    let expected = PfConfReference::default();

    fs::write_sensitive_file(
        &current_path,
        &format!("set block-policy drop\n{}\npass out all\n", expected.render()),
    )?;
    fs::write_sensitive_file(
        &stale_path,
        "# Managed by PV\nanchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/tmp/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(
        &conflict_path,
        "anchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
    )?;
    fs::write_sensitive_file(&unrelated_path, "set block-policy drop\npass out all\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_pf_conf_reference(&tempdir.path().join("missing-pf-conf"), Some(&expected)),
        inspect_pf_conf_reference(&current_path, Some(&expected)),
        inspect_pf_conf_reference(&stale_path, Some(&expected)),
        inspect_pf_conf_reference(&conflict_path, Some(&expected)),
        inspect_pf_conf_reference(&unrelated_path, Some(&expected)),
        inspect_pf_conf_reference(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}
```

Update the import in that file to:

```rust
use macos::{
    PfConfReference, PfRedirectConfig, ResolverConfig, inspect_pf_anchor_file,
    inspect_pf_conf_reference, inspect_resolver_file,
};
```

- [ ] **Step 3: Run the focused failing macOS tests**

Run:

```bash
cargo nextest run -p macos -E 'test(pf_config_renders_pv_owned_anchor_and_pf_conf_reference) or test(pf_anchor_inspection_reports_missing_current_stale_conflict_and_unreadable) or test(pf_conf_reference_inspection_reports_missing_current_stale_conflict_and_unreadable)'
```

Expected: compile failure for missing `pf` types and helpers.

- [ ] **Step 4: Add `pf` constants, types, and renderers**

In `crates/macos/src/lib.rs`, add:

```rust
use std::collections::BTreeSet;
use std::net::IpAddr;
```

Add constants near the resolver constants:

```rust
pub const SYSTEM_PF_ANCHOR_PATH: &str = "/etc/pf.anchors/com.prvious.pv";
pub const SYSTEM_PF_CONF_PATH: &str = "/etc/pf.conf";
const PF_ANCHOR_SOURCE_MARKER: &str =
    "# Source: PV prepared pf anchor for /etc/pf.anchors/com.prvious.pv";
const PF_CONF_SOURCE_MARKER: &str = "# Source: PV prepared pf.conf reference for /etc/pf.conf";
const PF_ANCHOR_NAME: &str = "com.prvious.pv";
```

Add these types:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PfRedirectConfig {
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct PfConfReference;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PfFileState<T> {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        value: T,
    },
    Stale {
        path: Utf8PathBuf,
        expected: Option<T>,
        actual: Option<T>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}
```

Add render/parse implementations:

```rust
impl PfRedirectConfig {
    pub const fn new(http_port: u16, https_port: u16) -> Self {
        Self {
            http_port,
            https_port,
        }
    }

    pub fn render_anchor(&self) -> String {
        format!(
            "{PV_MARKER}\n{PF_ANCHOR_SOURCE_MARKER}\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port {}\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port {}\n",
            self.http_port, self.https_port
        )
    }

    pub fn parse_anchor(content: &str) -> Option<Self> {
        let mut http_port = None;
        let mut https_port = None;

        for line in content.lines().map(str::trim) {
            if let Some(port) = line.strip_prefix(
                "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port ",
            ) {
                http_port = port.parse::<u16>().ok();
                continue;
            }

            if let Some(port) = line.strip_prefix(
                "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port ",
            ) {
                https_port = port.parse::<u16>().ok();
            }
        }

        Some(Self::new(http_port?, https_port?))
    }
}

impl PfConfReference {
    pub fn render(self) -> String {
        format!(
            "{PV_MARKER}\n{PF_CONF_SOURCE_MARKER}\nanchor \"{PF_ANCHOR_NAME}\"\nload anchor \"{PF_ANCHOR_NAME}\" from \"{SYSTEM_PF_ANCHOR_PATH}\"\n"
        )
    }

    pub fn parse_block(content: &str) -> Option<Self> {
        let has_anchor = content
            .lines()
            .map(str::trim)
            .any(|line| line == format!("anchor \"{PF_ANCHOR_NAME}\""));
        let has_load = content.lines().map(str::trim).any(|line| {
            line == format!("load anchor \"{PF_ANCHOR_NAME}\" from \"{SYSTEM_PF_ANCHOR_PATH}\"")
        });

        if has_anchor && has_load {
            Some(Self)
        } else {
            None
        }
    }
}
```

- [ ] **Step 5: Add `pf` inspection helpers**

In `crates/macos/src/lib.rs`, add:

```rust
pub fn inspect_pf_anchor_file(
    path: &Utf8Path,
    expected: Option<&PfRedirectConfig>,
) -> PfFileState<PfRedirectConfig> {
    inspect_pv_file(path, expected, PfRedirectConfig::parse_anchor, true)
}

pub fn inspect_pf_conf_reference(
    path: &Utf8Path,
    expected: Option<&PfConfReference>,
) -> PfFileState<PfConfReference> {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return PfFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    let has_pv_marker = content.lines().any(|line| line.trim() == PV_MARKER);
    let has_anchor_name = content.lines().map(str::trim).any(|line| {
        line.contains("com.prvious.pv") || line.contains("/etc/pf.anchors/com.prvious.pv")
    });

    if !has_pv_marker {
        return if has_anchor_name {
            PfFileState::Conflict {
                path: path.to_path_buf(),
            }
        } else {
            PfFileState::Missing {
                path: path.to_path_buf(),
            }
        };
    }

    let actual = PfConfReference::parse_block(&content);
    classify_pv_file_state(path, expected, actual)
}

fn inspect_pv_file<T: Clone + Eq>(
    path: &Utf8Path,
    expected: Option<&T>,
    parse: impl FnOnce(&str) -> Option<T>,
    conflict_when_unmarked: bool,
) -> PfFileState<T> {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return PfFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) && conflict_when_unmarked {
        return PfFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = parse(&content);
    classify_pv_file_state(path, expected, actual)
}

fn classify_pv_file_state<T: Clone + Eq>(
    path: &Utf8Path,
    expected: Option<&T>,
    actual: Option<T>,
) -> PfFileState<T> {
    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => PfFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (Some(expected), actual) => PfFileState::Stale {
            path: path.to_path_buf(),
            expected: Some(expected.clone()),
            actual,
        },
        (None, Some(actual)) => PfFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (None, None) => PfFileState::Stale {
            path: path.to_path_buf(),
            expected: None,
            actual: None,
        },
    }
}
```

- [ ] **Step 6: Replace `MacosError` with a typed enum and add socket-table detection**

Replace the current unit-like `MacosError` with:

```rust
#[derive(Debug, Error)]
pub enum MacosError {
    #[error("could not inspect socket table: {0}")]
    SocketTable(#[from] netstat::Error),
}
```

Add:

```rust
pub fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, MacosError> {
    let sockets = netstat::get_sockets_info(
        netstat::AddressFamilyFlags::IPV4,
        netstat::ProtocolFlags::TCP,
    )?;
    let mut ports = BTreeSet::new();

    for socket in sockets {
        let netstat::ProtocolSocketInfo::Tcp(tcp) = socket.protocol_socket_info else {
            continue;
        };

        if tcp.state == netstat::TcpState::Listen
            && matches!(tcp.local_addr, IpAddr::V4(address) if address.is_loopback())
        {
            ports.insert(tcp.local_port);
        }
    }

    Ok(ports)
}

pub fn loopback_tcp_port_has_listener(port: u16) -> Result<bool, MacosError> {
    Ok(loopback_tcp_listener_ports()?.contains(&port))
}
```

- [ ] **Step 7: Run and accept focused macOS snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- pf_config_renders_pv_owned_anchor_and_pf_conf_reference
cargo insta test --accept --test-runner nextest -- pf_anchor_inspection_reports_missing_current_stale_conflict_and_unreadable
cargo insta test --accept --test-runner nextest -- pf_conf_reference_inspection_reports_missing_current_stale_conflict_and_unreadable
cargo nextest run -p macos -E 'test(pf_config_renders_pv_owned_anchor_and_pf_conf_reference) or test(pf_anchor_inspection_reports_missing_current_stale_conflict_and_unreadable) or test(pf_conf_reference_inspection_reports_missing_current_stale_conflict_and_unreadable)'
```

Expected: all selected tests pass.

- [ ] **Step 8: Commit macOS changes**

Run:

```bash
git add Cargo.toml Cargo.lock crates/macos/Cargo.toml crates/macos/src/lib.rs crates/macos/tests/resolver_config.rs crates/macos/tests/snapshots
git commit -m "feat(macos): prepare pf redirect artifacts"
```

## Task 3: CLI ports Commands

**Files:**
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/cli/src/args.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Create: `crates/cli/src/commands/ports.rs`
- Create: `crates/cli/tests/ports.rs`

- [ ] **Step 1: Add failing CLI integration tests**

Create `crates/cli/tests/ports.rs`:

```rust
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use macos::{PfConfReference, PfRedirectConfig};
use state::{Database, PortOwner, PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    pf_anchor_path: PathBuf,
    pf_conf_path: PathBuf,
    listening_ports: BTreeSet<u16>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, pf_anchor_path: &Utf8Path, pf_conf_path: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            pf_anchor_path: pf_anchor_path.as_std_path().to_path_buf(),
            pf_conf_path: pf_conf_path.as_std_path().to_path_buf(),
            listening_ports: BTreeSet::new(),
        }
    }

    fn with_listener(mut self, port: u16) -> Self {
        self.listening_ports.insert(port);
        self
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

    fn pf_anchor_path(&self) -> PathBuf {
        self.pf_anchor_path.clone()
    }

    fn pf_conf_path(&self) -> PathBuf {
        self.pf_conf_path.clone()
    }

    fn loopback_tcp_listener_ports(&self) -> Result<BTreeSet<u16>, macos::MacosError> {
        Ok(self.listening_ports.clone())
    }
}

#[test]
fn ports_install_prepares_pf_artifacts_without_touching_system_paths() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(&home, &current_dir, &system_anchor_path, &system_pf_conf_path);

    let output = run_pv(&["ports:install"], &environment)?;
    let paths = pv_paths(&home);
    let prepared_anchor = read_required_file(&paths.pf_anchor_config())?;
    let prepared_reference = read_required_file(&paths.pf_conf_reference_config())?;
    let parsed_anchor = PfRedirectConfig::parse_anchor(&prepared_anchor)
        .ok_or_else(|| anyhow::anyhow!("prepared pf anchor did not parse"))?;
    let parsed_reference = PfConfReference::parse_block(&prepared_reference)
        .ok_or_else(|| anyhow::anyhow!("prepared pf.conf reference did not parse"))?;
    let system_anchor_after_install = read_optional_file(&system_anchor_path)?;
    let system_pf_conf_after_install = read_optional_file(&system_pf_conf_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert_eq!(parsed_anchor.http_port, 48080);
    assert_eq!(parsed_anchor.https_port, 48443);
    assert_eq!(parsed_reference, PfConfReference::default());
    assert!(system_anchor_after_install.is_none());
    assert!(system_pf_conf_after_install.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            paths.pf_anchor_config(),
            prepared_anchor,
            paths.pf_conf_reference_config(),
            prepared_reference,
            system_anchor_after_install,
            system_pf_conf_after_install,
        ));
    });

    Ok(())
}

#[test]
fn ports_install_fails_on_low_port_conflict_before_writing_prepared_artifacts() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(&home, &current_dir, &system_anchor_path, &system_pf_conf_path)
        .with_listener(80);
    let paths = pv_paths(&home);

    let output = run_pv(&["ports:install"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert!(read_optional_file(&paths.pf_anchor_config())?.is_none());
    assert!(read_optional_file(&paths.pf_conf_reference_config())?.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn ports_status_reports_prepared_and_system_pf_states_without_mutating_state() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(&home, &current_dir, &system_anchor_path, &system_pf_conf_path);
    let paths = pv_paths(&home);
    let current_anchor = PfRedirectConfig::new(48080, 48443).render_anchor();
    let stale_anchor = PfRedirectConfig::new(45000, 45001).render_anchor();
    let current_reference = PfConfReference::default().render();

    let missing = run_pv(&["ports:status"], &environment)?;
    let database_after_missing = read_optional_file(paths.db())?;
    let prepared_anchor_after_missing = read_optional_file(&paths.pf_anchor_config())?;
    let prepared_reference_after_missing = read_optional_file(&paths.pf_conf_reference_config())?;

    write_file(&paths.pf_anchor_config(), &current_anchor)?;
    write_file(&paths.pf_conf_reference_config(), &current_reference)?;
    let prepared_only = run_pv(&["ports:status"], &environment)?;

    write_file(&system_anchor_path, &current_anchor)?;
    write_file(&system_pf_conf_path, &current_reference)?;
    let current = run_pv(&["ports:status"], &environment)?;

    write_file(&system_anchor_path, &stale_anchor)?;
    write_file(&system_pf_conf_path, "anchor \"com.prvious.pv\"\n")?;
    let stale_and_conflict = run_pv(&["ports:status"], &environment)?;

    assert_eq!(missing.exit_code, ExitCode::SUCCESS);
    assert_eq!(prepared_only.exit_code, ExitCode::SUCCESS);
    assert_eq!(current.exit_code, ExitCode::SUCCESS);
    assert_eq!(stale_and_conflict.exit_code, ExitCode::SUCCESS);
    assert!(database_after_missing.is_none());
    assert!(prepared_anchor_after_missing.is_none());
    assert!(prepared_reference_after_missing.is_none());

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            missing,
            prepared_only,
            current,
            stale_and_conflict,
        ));
    });

    Ok(())
}

#[test]
fn ports_uninstall_removes_prepared_artifacts_and_defers_system_removal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(&home, &current_dir, &system_anchor_path, &system_pf_conf_path);
    let paths = pv_paths(&home);
    let anchor = PfRedirectConfig::new(48080, 48443).render_anchor();
    let reference = PfConfReference::default().render();

    write_file(&paths.pf_anchor_config(), &anchor)?;
    write_file(&paths.pf_conf_reference_config(), &reference)?;
    write_file(&system_anchor_path, &anchor)?;
    write_file(&system_pf_conf_path, &reference)?;

    let output = run_pv(&["ports:uninstall"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.is_empty());
    assert_no_privileged_guidance(&output.stdout);
    assert!(read_optional_file(&paths.pf_anchor_config())?.is_none());
    assert!(read_optional_file(&paths.pf_conf_reference_config())?.is_none());
    assert_eq!(read_required_file(&system_anchor_path)?, anchor);
    assert_eq!(read_required_file(&system_pf_conf_path)?, reference);

    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn ports_install_reuses_persisted_gateway_ports_even_when_they_have_listeners() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_anchor_path = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
    let system_pf_conf_path = tempdir.path().join("etc/pf.conf");
    let environment = TestEnvironment::new(&home, &current_dir, &system_anchor_path, &system_pf_conf_path)
        .with_listener(48080)
        .with_listener(48443);
    let paths = pv_paths(&home);
    let mut database = Database::open(&paths)?;
    let seeded = database.assign_gateway_ports(|port| port == 48080 || port == 48443)?;
    drop(database);

    let output = run_pv(&["ports:install"], &environment)?;
    let prepared_anchor = read_required_file(&paths.pf_anchor_config())?;
    let parsed_anchor = PfRedirectConfig::parse_anchor(&prepared_anchor)
        .ok_or_else(|| anyhow::anyhow!("prepared pf anchor did not parse"))?;
    let assignments = Database::open(&paths)?.assigned_ports()?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_eq!(parsed_anchor.http_port, seeded.http.port);
    assert_eq!(parsed_anchor.https_port, seeded.https.port);
    assert!(assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(state::GatewayPort::Http) && assignment.port == seeded.http.port));
    assert!(assignments
        .iter()
        .any(|assignment| assignment.owner == PortOwner::Gateway(state::GatewayPort::Https) && assignment.port == seeded.https.port));

    Ok(())
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

fn read_required_file(path: &Utf8Path) -> anyhow::Result<String> {
    read_optional_file(path)?
        .ok_or_else(|| anyhow::anyhow!("expected fixture file to exist: {path}"))
}

fn read_optional_file(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    state::fs::write_sensitive_file(path, content)?;

    Ok(())
}

fn assert_no_privileged_guidance(output: &str) {
    for pattern in ["sudo", "pfctl", "sudo rm", "sudo install"] {
        assert!(
            !output.contains(pattern),
            "output contains privileged guidance `{pattern}`: {output}"
        );
    }
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assertion);
}
```

- [ ] **Step 2: Run the focused failing CLI tests**

Run:

```bash
cargo nextest run -p cli -E 'test(ports_install_prepares_pf_artifacts_without_touching_system_paths) or test(ports_install_fails_on_low_port_conflict_before_writing_prepared_artifacts) or test(ports_status_reports_prepared_and_system_pf_states_without_mutating_state) or test(ports_uninstall_removes_prepared_artifacts_and_defers_system_removal) or test(ports_install_reuses_persisted_gateway_ports_even_when_they_have_listeners)'
```

Expected: compile failure for missing `ports:*` command routing and environment hooks.

- [ ] **Step 3: Add injectable environment hooks**

In `crates/cli/src/environment.rs`, add these default trait methods:

```rust
fn pf_anchor_path(&self) -> PathBuf {
    PathBuf::from(macos::SYSTEM_PF_ANCHOR_PATH)
}

fn pf_conf_path(&self) -> PathBuf {
    PathBuf::from(macos::SYSTEM_PF_CONF_PATH)
}

fn loopback_tcp_listener_ports(&self) -> Result<std::collections::BTreeSet<u16>, macos::MacosError> {
    macos::loopback_tcp_listener_ports()
}
```

- [ ] **Step 4: Wire command arguments and routing**

In `crates/cli/src/args.rs`, add enum variants after the DNS commands:

```rust
#[command(name = "ports:status", about = "Show PV pf redirect status")]
PortsStatus,

#[command(name = "ports:install", about = "Prepare PV pf redirect config")]
PortsInstall,

#[command(
    name = "ports:uninstall",
    about = "Remove prepared PV pf redirect config"
)]
PortsUninstall,
```

In `crates/cli/src/commands/mod.rs`, add:

```rust
mod ports;
```

Add match arms:

```rust
Command::PortsStatus => ports::status(environment, stdout),
Command::PortsInstall => ports::install(environment, stdout),
Command::PortsUninstall => ports::uninstall(environment, stdout),
```

- [ ] **Step 5: Implement `crates/cli/src/commands/ports.rs`**

Create `crates/cli/src/commands/ports.rs` with:

```rust
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use macos::{PfConfReference, PfFileState, PfRedirectConfig};
use state::{Database, GatewayPortAssignments, PvPaths, StateError};

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

const LOW_PORTS: [u16; 2] = [80, 443];

pub(crate) fn status(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_anchor_path = paths.pf_anchor_config();
    let prepared_reference_path = paths.pf_conf_reference_config();
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;
    let prepared_anchor_state = macos::inspect_pf_anchor_file(&prepared_anchor_path, None);
    let prepared_reference_state =
        macos::inspect_pf_conf_reference(&prepared_reference_path, None);
    let expected_anchor = pf_config_from_anchor_state(&prepared_anchor_state);
    let expected_reference = pf_reference_from_state(&prepared_reference_state);
    let system_anchor_state =
        macos::inspect_pf_anchor_file(&system_anchor_path, expected_anchor.as_ref());
    let system_reference_state =
        macos::inspect_pf_conf_reference(&system_pf_conf_path, expected_reference.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Port redirect status")?;
    write_pf_anchor_state(&mut output, "Prepared pf anchor", &prepared_anchor_state)?;
    write_pf_reference_state(
        &mut output,
        "Prepared pf.conf reference",
        &prepared_reference_state,
    )?;
    write_pf_anchor_state(&mut output, "System pf anchor", &system_anchor_state)?;
    write_pf_reference_state(
        &mut output,
        "System pf.conf reference",
        &system_reference_state,
    )?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn install(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let listening_ports = environment.loopback_tcp_listener_ports()?;
    let low_port_conflicts = low_port_conflicts(&listening_ports);
    let mut output = Output::new(stdout, OutputMode::plain());

    if !low_port_conflicts.is_empty() {
        output.line("Port redirect preparation failed")?;
        for port in low_port_conflicts {
            output.line(&format!(
                "Loopback TCP port {port} already has a listener."
            ))?;
        }
        output.line("Stop the conflicting service, then run `pv ports:install` again.")?;

        return Ok(ExitCode::FAILURE);
    }

    let mut database = Database::open(&paths)?;
    let assignments = database.assign_gateway_ports(|port| !listening_ports.contains(&port))?;
    let config = pf_config_from_assignments(&assignments);
    let reference = PfConfReference;
    let prepared_anchor_path = paths.pf_anchor_config();
    let prepared_reference_path = paths.pf_conf_reference_config();
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;

    state::fs::write_sensitive_file(&prepared_anchor_path, &config.render_anchor())?;
    state::fs::write_sensitive_file(&prepared_reference_path, &reference.render())?;

    let system_anchor_state = macos::inspect_pf_anchor_file(&system_anchor_path, Some(&config));
    let system_reference_state =
        macos::inspect_pf_conf_reference(&system_pf_conf_path, Some(&reference));

    output.line("Prepared PV port redirect config")?;
    output.line(&format!("  anchor path: {prepared_anchor_path}"))?;
    output.line(&format!("  pf.conf reference path: {prepared_reference_path}"))?;
    output.line(&format!("  HTTP redirect: 127.0.0.1:80 -> 127.0.0.1:{}", config.http_port))?;
    output.line(&format!("  HTTPS redirect: 127.0.0.1:443 -> 127.0.0.1:{}", config.https_port))?;
    output.line("Privileged install deferred to PR 13 setup/system-integration work.")?;
    write_pf_anchor_install_guidance(&mut output, &system_anchor_state)?;
    write_pf_reference_install_guidance(&mut output, &system_reference_state)?;

    Ok(ExitCode::FAILURE)
}

pub(crate) fn uninstall(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let prepared_anchor_path = paths.pf_anchor_config();
    let prepared_reference_path = paths.pf_conf_reference_config();
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;
    let deleted_anchor = delete_optional_file(&prepared_anchor_path)?;
    let deleted_reference = delete_optional_file(&prepared_reference_path)?;
    let system_anchor_state = macos::inspect_pf_anchor_file(&system_anchor_path, None);
    let system_reference_state = macos::inspect_pf_conf_reference(&system_pf_conf_path, None);
    let mut output = Output::new(stdout, OutputMode::plain());

    write_delete_result(&mut output, "Prepared pf anchor", &prepared_anchor_path, deleted_anchor)?;
    write_delete_result(
        &mut output,
        "Prepared pf.conf reference",
        &prepared_reference_path,
        deleted_reference,
    )?;

    let anchor_exit = write_pf_anchor_uninstall_guidance(&mut output, &system_anchor_state)?;
    let reference_exit = write_pf_reference_uninstall_guidance(&mut output, &system_reference_state)?;

    if anchor_exit == ExitCode::SUCCESS && reference_exit == ExitCode::SUCCESS {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn low_port_conflicts(listening_ports: &std::collections::BTreeSet<u16>) -> Vec<u16> {
    let mut conflicts = Vec::new();

    for port in LOW_PORTS {
        if listening_ports.contains(&port) {
            conflicts.push(port);
        }
    }

    conflicts
}

fn pf_config_from_assignments(assignments: &GatewayPortAssignments) -> PfRedirectConfig {
    PfRedirectConfig::new(assignments.http.port, assignments.https.port)
}

fn pf_config_from_anchor_state(
    state: &PfFileState<PfRedirectConfig>,
) -> Option<PfRedirectConfig> {
    match state {
        PfFileState::Current { value, .. }
        | PfFileState::Stale {
            actual: Some(value), ..
        } => Some(value.clone()),
        PfFileState::Missing { .. }
        | PfFileState::Stale { actual: None, .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn pf_reference_from_state(state: &PfFileState<PfConfReference>) -> Option<PfConfReference> {
    match state {
        PfFileState::Current { value, .. }
        | PfFileState::Stale {
            actual: Some(value), ..
        } => Some(*value),
        PfFileState::Missing { .. }
        | PfFileState::Stale { actual: None, .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}
```

Continue the same file with output helpers copied from `dns.rs` style:

```rust
fn write_pf_anchor_state(
    output: &mut Output<'_, impl Write>,
    label: &str,
    state: &PfFileState<PfRedirectConfig>,
) -> io::Result<()> {
    match state {
        PfFileState::Missing { path } => {
            output.line(&format!("{label}: missing"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Current { path, value } => {
            output.line(&format!("{label}: current"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  HTTP redirect: 127.0.0.1:80 -> 127.0.0.1:{}", value.http_port))?;
            output.line(&format!("  HTTPS redirect: 127.0.0.1:443 -> 127.0.0.1:{}", value.https_port))
        }
        PfFileState::Stale {
            path,
            expected,
            actual,
        } => {
            output.line(&format!("{label}: stale"))?;
            output.line(&format!("  path: {path}"))?;
            write_optional_pf_config(output, "expected", expected.as_ref())?;
            write_optional_pf_config(output, "actual", actual.as_ref())
        }
        PfFileState::Conflict { path } => {
            output.line(&format!("{label}: not PV-owned"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("{label}: unreadable"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn write_pf_reference_state(
    output: &mut Output<'_, impl Write>,
    label: &str,
    state: &PfFileState<PfConfReference>,
) -> io::Result<()> {
    match state {
        PfFileState::Missing { path } => {
            output.line(&format!("{label}: missing"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Current { path, .. } => {
            output.line(&format!("{label}: current"))?;
            output.line(&format!("  path: {path}"))?;
            output.line("  anchor: com.prvious.pv")
        }
        PfFileState::Stale { path, .. } => {
            output.line(&format!("{label}: stale"))?;
            output.line(&format!("  path: {path}"))?;
            output.line("  anchor: com.prvious.pv")
        }
        PfFileState::Conflict { path } => {
            output.line(&format!("{label}: not PV-owned"))?;
            output.line(&format!("  path: {path}"))
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("{label}: unreadable"))?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn write_optional_pf_config(
    output: &mut Output<'_, impl Write>,
    label: &str,
    config: Option<&PfRedirectConfig>,
) -> io::Result<()> {
    match config {
        Some(config) => {
            output.line(&format!("  {label} HTTP port: {}", config.http_port))?;
            output.line(&format!("  {label} HTTPS port: {}", config.https_port))
        }
        None => output.line(&format!("  {label}: unparseable")),
    }
}

fn write_pf_anchor_install_guidance(
    output: &mut Output<'_, impl Write>,
    state: &PfFileState<PfRedirectConfig>,
) -> io::Result<()> {
    match state {
        PfFileState::Missing { path } => output.line(&format!("System pf anchor is not installed: {path}")),
        PfFileState::Current { path, .. } => output.line(&format!("System pf anchor already matches PV: {path}")),
        PfFileState::Stale { path, .. } => output.line(&format!("PV-owned system pf anchor is stale: {path}")),
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf anchor is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("System pf anchor could not be inspected: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn write_pf_reference_install_guidance(
    output: &mut Output<'_, impl Write>,
    state: &PfFileState<PfConfReference>,
) -> io::Result<()> {
    match state {
        PfFileState::Missing { path } => output.line(&format!("System pf.conf reference is not installed: {path}")),
        PfFileState::Current { path, .. } => output.line(&format!("System pf.conf reference already matches PV: {path}")),
        PfFileState::Stale { path, .. } => output.line(&format!("PV-owned system pf.conf reference is stale: {path}")),
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf.conf reference is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("System pf.conf reference could not be inspected: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}
```

Add the uninstall and path helpers:

```rust
fn write_pf_anchor_uninstall_guidance(
    output: &mut Output<'_, impl Write>,
    state: &PfFileState<PfRedirectConfig>,
) -> io::Result<ExitCode> {
    match state {
        PfFileState::Missing { path } => {
            output.line(&format!("System pf anchor already absent: {path}"))?;
            Ok(ExitCode::SUCCESS)
        }
        PfFileState::Current { path, .. } | PfFileState::Stale { path, .. } => {
            output.line(&format!("PV-owned system pf anchor remains: {path}"))?;
            output.line("Privileged removal deferred to PR 13 setup/system-integration work.")?;
            Ok(ExitCode::FAILURE)
        }
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf anchor is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;
            Ok(ExitCode::FAILURE)
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("System pf anchor could not be inspected: {path}"))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;
            Ok(ExitCode::FAILURE)
        }
    }
}

fn write_pf_reference_uninstall_guidance(
    output: &mut Output<'_, impl Write>,
    state: &PfFileState<PfConfReference>,
) -> io::Result<ExitCode> {
    match state {
        PfFileState::Missing { path } => {
            output.line(&format!("System pf.conf reference already absent: {path}"))?;
            Ok(ExitCode::SUCCESS)
        }
        PfFileState::Current { path, .. } | PfFileState::Stale { path, .. } => {
            output.line(&format!("PV-owned system pf.conf reference remains: {path}"))?;
            output.line("Privileged removal deferred to PR 13 setup/system-integration work.")?;
            Ok(ExitCode::FAILURE)
        }
        PfFileState::Conflict { path } => {
            output.line(&format!("System pf.conf reference is not PV-owned: {path}"))?;
            output.line("Leaving it in place.")?;
            Ok(ExitCode::FAILURE)
        }
        PfFileState::Unreadable { path, message } => {
            output.line(&format!("System pf.conf reference could not be inspected: {path}"))?;
            output.line(&format!("  {message}"))?;
            output.line("Leaving it in place.")?;
            Ok(ExitCode::FAILURE)
        }
    }
}

fn write_delete_result(
    output: &mut Output<'_, impl Write>,
    label: &str,
    path: &Utf8Path,
    deleted: bool,
) -> io::Result<()> {
    if deleted {
        output.line(&format!("Deleted {label}: {path}"))
    } else {
        output.line(&format!("{label} already absent: {path}"))
    }
}

fn delete_optional_file(path: &Utf8Path) -> Result<bool, ExecuteError> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(true),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn pf_anchor_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_anchor_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_conf_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_conf_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}
```

- [ ] **Step 6: Add missing error conversion**

If `?` on `macos::MacosError` fails to compile in CLI, add this variant in `crates/cli/src/error.rs`:

```rust
#[error(transparent)]
Macos(#[from] macos::MacosError),
```

Then add a `finish_execution` branch in `crates/cli/src/lib.rs` that formats it as a user-facing error:

```rust
Err(ExecuteError::Macos(error)) => {
    let mut output = Output::new(stderr, output_mode);
    output.error(&error.to_string())?;

    Ok(ExitCode::FAILURE)
}
```

- [ ] **Step 7: Run and accept focused CLI snapshots**

Run:

```bash
cargo insta test --accept --test-runner nextest -- ports_install_prepares_pf_artifacts_without_touching_system_paths
cargo insta test --accept --test-runner nextest -- ports_install_fails_on_low_port_conflict_before_writing_prepared_artifacts
cargo insta test --accept --test-runner nextest -- ports_status_reports_prepared_and_system_pf_states_without_mutating_state
cargo insta test --accept --test-runner nextest -- ports_uninstall_removes_prepared_artifacts_and_defers_system_removal
cargo nextest run -p cli -E 'test(ports_install_prepares_pf_artifacts_without_touching_system_paths) or test(ports_install_fails_on_low_port_conflict_before_writing_prepared_artifacts) or test(ports_status_reports_prepared_and_system_pf_states_without_mutating_state) or test(ports_uninstall_removes_prepared_artifacts_and_defers_system_removal) or test(ports_install_reuses_persisted_gateway_ports_even_when_they_have_listeners)'
```

Expected: all selected tests pass.

- [ ] **Step 8: Commit CLI changes**

Run:

```bash
git add crates/cli/src/environment.rs crates/cli/src/args.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/ports.rs crates/cli/src/error.rs crates/cli/src/lib.rs crates/cli/tests/ports.rs crates/cli/tests/snapshots
git commit -m "feat(cli): add pf port commands"
```

## Task 4: Cross-Crate Verification and Scope Guard

**Files:**
- Inspect: `DESIGN.md`
- Inspect: `docs/superpowers/specs/2026-06-03-pr-11-pf-port-commands-design.md`
- Inspect: implementation diff

- [ ] **Step 1: Verify no privileged mutation slipped in**

Run:

```bash
rg -n "sudo|pfctl|/etc/pf\\.conf|/etc/pf\\.anchors|std::process::Command|Command::new" crates/cli crates/macos crates/state
```

Expected:
- `/etc/pf.conf` and `/etc/pf.anchors/com.prvious.pv` appear only as constants, output labels, parser fixtures, or tests.
- No `sudo` appears in command output.
- No `pfctl` appears in command output.
- No new process-spawning code appears.

- [ ] **Step 2: Run focused package tests**

Run:

```bash
cargo nextest run -p state -E 'test(gateway_port_allocator) or test(pv_paths_include_prepared_pf_artifacts)'
cargo nextest run -p macos -E 'test(pf_)'
cargo nextest run -p cli -E 'test(ports_)'
```

Expected: all selected tests pass.

- [ ] **Step 3: Run formatting and diff hygiene**

Run:

```bash
cargo fmt --all -- --check
git diff --check
```

Expected: both pass.

- [ ] **Step 4: Run full workspace verification**

Run:

```bash
cargo nextest run --workspace
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo shear
```

Expected: all pass. If `cargo shear` is unavailable, record that and run the other checks.

- [ ] **Step 5: Commit any verification fixes**

If formatting, clippy, or focused test fixes changed files, run:

```bash
git add <changed-files>
git commit -m "fix: polish pf port command implementation"
```

Expected: branch is clean after the commit.

## Task 5: PR, Roadmap Update, and Cleanup Handoff

**Files:**
- Modify: `IMPLEMENTATION.md`

- [ ] **Step 1: Push the branch and open the PR**

Run:

```bash
git status --short --branch
git push -u origin feat/pr11-pf-port-commands
gh pr create --title "feat: add pf port command preparation" --body-file -
```

Use this PR body:

```markdown
## Summary
- add distinct persisted Gateway HTTP/HTTPS port assignments
- render and inspect PV-owned prepared `pf` anchor and pf.conf reference artifacts
- add non-privileged `pv ports:status`, `pv ports:install`, and `pv ports:uninstall`

## Scope
- does not write `/etc/pf.conf` or `/etc/pf.anchors/com.prvious.pv`
- does not invoke `pfctl`
- does not run `sudo`
- leaves privileged installation/removal to PR 13 setup/system-integration work

## Tests
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo shear`
- `cargo fmt --all -- --check`
- `git diff --check`
```

Expected: GitHub returns the PR URL and number.

- [ ] **Step 2: Update `IMPLEMENTATION.md` with the PR number**

Change the PR 11 row from:

```markdown
| PR 11 | `pf` config generation and port commands | PV-052, PV-053 | PR 4, PR 5 | Yes | No |
```

to:

```markdown
| PR 11 | `pf` config generation and port commands | PV-052, PV-053 | PR 4, PR 5 | Yes | Yes (#<pr-number>) |
```

Replace `<pr-number>` with the actual PR number from `gh pr create`.

- [ ] **Step 3: Verify and push the roadmap update**

Run:

```bash
cargo fmt --all -- --check
git diff --check
git add IMPLEMENTATION.md
git commit -m "docs: mark PR 11 roadmap item"
git push
```

Expected: docs commit pushes to the PR branch.

- [ ] **Step 4: Check PR status**

Run:

```bash
gh pr checks --watch
gh pr view --json number,url,headRefOid,mergeStateStatus,latestReviews,comments
```

Expected: CI is passing or pending with no local action required. If CodeRabbit or another reviewer leaves actionable comments, verify each comment against the source before changing code.

## Self-Review Results

- Spec coverage: covered state persistence and reuse, prepared anchor/reference rendering, read-only system inspection, low-port conflict detection, `ports:status`, `ports:install`, `ports:uninstall`, no privileged mutation, CLI snapshots, and final roadmap tracking.
- Placeholder scan: no open-ended implementation instructions remain; each task has concrete files, code, commands, and expected outcomes.
- Type consistency: the plan consistently uses `GatewayPort`, `GatewayPortAssignments`, `PfRedirectConfig`, `PfConfReference`, and `PfFileState<T>` across state, macOS, and CLI tasks.
