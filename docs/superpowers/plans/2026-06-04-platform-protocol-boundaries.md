# Platform And Protocol Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move PV's daemon protocol contracts into a `protocol` crate and move macOS host integration into a `platform` crate while keeping v1 macOS-only behavior unchanged.

**Architecture:** Extract the daemon wire contract first so `daemon` no longer owns protocol types. Then replace `crates/macos` with `crates/platform`, update application code to import host integration through `platform`, and document the new boundary in `DESIGN.md` and `IMPLEMENTATION.md`.

**Tech Stack:** Rust 2024 workspace, Cargo, Tokio, tokio-util line codecs, serde/serde_json, thiserror, nextest, insta snapshots.

---

## Baseline

Current branch for this work: `refactor/platform-protocol-boundaries`.

Baseline commands already run on this branch:

```bash
cargo build --workspace --locked
cargo nextest run -p cli -p daemon -p macos --locked
```

Expected baseline result:

- `cargo build --workspace --locked`: pass
- `cargo nextest run -p cli -p daemon -p macos --locked`: 127 tests run, 127 passed

## File Structure

Create:

- `crates/protocol/Cargo.toml`: new protocol crate manifest.
- `crates/protocol/src/lib.rs`: daemon wire types, protocol version, line transport, and protocol write helper.
- `docs/superpowers/plans/2026-06-04-platform-protocol-boundaries.md`: this implementation plan.

Move:

- `crates/macos/` -> `crates/platform/`

Delete:

- `crates/daemon/src/protocol.rs`: after its shared types and transport helpers move to `protocol`.
- `crates/macos/`: removed by the directory move.

Modify:

- `Cargo.toml`: replace `crates/macos` workspace member with `crates/platform` and add `crates/protocol`.
- `crates/daemon/Cargo.toml`: add `protocol = { path = "../protocol" }`.
- `crates/daemon/src/client.rs`: import protocol types from `protocol`.
- `crates/daemon/src/error.rs`: convert protocol helper errors into `DaemonError`.
- `crates/daemon/src/jobs.rs`: import protocol types from `protocol`.
- `crates/daemon/src/lib.rs`: remove internal `protocol` module and re-export `protocol::PROTOCOL_VERSION`.
- `crates/daemon/src/server.rs`: import protocol types from `protocol`.
- `crates/platform/Cargo.toml`: rename package from `macos` to `platform`.
- `crates/platform/src/lib.rs`: keep existing macOS implementation, rename the public error and native trust inspector to host-facing names.
- `crates/platform/tests/resolver_config.rs`: import from `platform`.
- `crates/cli/Cargo.toml`: replace `macos` dependency with `platform`.
- `crates/cli/src/environment.rs`: route system path constants and host inspection through `platform`.
- `crates/cli/src/error.rs`: expose `Platform` error variant instead of `Macos`.
- `crates/cli/src/commands/ca.rs`: import from `platform`.
- `crates/cli/src/commands/dns.rs`: import from `platform`.
- `crates/cli/src/commands/ports.rs`: import from `platform`.
- `crates/cli/tests/ca.rs`: import from `platform`.
- `crates/cli/tests/dns.rs`: import from `platform`.
- `crates/cli/tests/ports.rs`: import from `platform`.
- `Cargo.lock`: update internal workspace package entries for `protocol` and `platform` only.
- `DESIGN.md`: record that v1 is macOS-only behind a host platform boundary.
- `IMPLEMENTATION.md`: update crate layout, crate ownership, and PR sequence for the boundary refactor.

## Task 1: Extract The Protocol Crate

**Files:**

- Create: `crates/protocol/Cargo.toml`
- Create: `crates/protocol/src/lib.rs`
- Modify: `Cargo.lock`
- Delete: `crates/daemon/src/protocol.rs`
- Modify: `Cargo.toml`
- Modify: `crates/daemon/Cargo.toml`
- Modify: `crates/daemon/src/client.rs`
- Modify: `crates/daemon/src/error.rs`
- Modify: `crates/daemon/src/jobs.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/server.rs`

- [ ] **Step 1: Add the protocol crate manifest**

Add `crates/protocol` to the root workspace members in `Cargo.toml`:

```toml
[workspace]
members = [
    ".",
    "crates/cli",
    "crates/config",
    "crates/daemon",
    "crates/macos",
    "crates/protocol",
    "crates/resources",
    "crates/state",
]
resolver = "3"
```

Create `crates/protocol/Cargo.toml`:

```toml
[package]
name = "protocol"
version = "0.1.0"
edition.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
futures-util = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tokio-util = { workspace = true }

[dev-dependencies]
anyhow = { workspace = true }
```

- [ ] **Step 2: Move shared protocol code into the new crate**

Create `crates/protocol/src/lib.rs`:

```rust
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LinesCodec, LinesCodecError};

pub const PROTOCOL_VERSION: u16 = 1;

const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

pub type DaemonTransport<Stream> = Framed<Stream, LinesCodec>;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("daemon protocol JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("daemon protocol frame error: {0}")]
    Frame(#[from] LinesCodecError),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DaemonRequest {
    pub protocol_version: u16,

    #[serde(flatten)]
    pub command: DaemonCommand,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DaemonCommand {
    Health,
    RunJob { kind: String, scope: String },
}

#[derive(Debug, Serialize)]
pub struct DaemonResponse<'message> {
    #[serde(rename = "type")]
    pub line_type: &'static str,
    pub protocol_version: u16,
    pub status: ResponseStatus,
    pub message: &'message str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<&'message str>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Ok,
    Accepted,
    Error,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonEvent<'message> {
    JobStarted {
        job_id: &'message str,
        kind: &'message str,
        scope: &'message str,
    },
    Progress {
        job_id: &'message str,
        message: &'message str,
    },
    Log {
        job_id: &'message str,
        message: &'message str,
    },
    JobCompleted {
        job_id: &'message str,
        summary: &'message str,
    },
    JobFailed {
        job_id: &'message str,
        error: &'message str,
    },
}

pub fn transport<Stream>(stream: Stream) -> DaemonTransport<Stream>
where
    Stream: AsyncRead + AsyncWrite,
{
    Framed::new(
        stream,
        LinesCodec::new_with_max_length(MAX_PROTOCOL_LINE_BYTES),
    )
}

pub async fn write_line<Stream>(
    transport: &mut DaemonTransport<Stream>,
    line: &impl Serialize,
) -> Result<(), ProtocolError>
where
    Stream: AsyncWrite + Unpin,
{
    let encoded = serde_json::to_string(line)?;

    transport.send(encoded).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use serde_json::json;
    use tokio::io::duplex;

    use super::{DaemonResponse, PROTOCOL_VERSION, ResponseStatus, transport, write_line};

    #[tokio::test]
    async fn transport_frames_generic_async_streams() -> anyhow::Result<()> {
        let (client, server) = duplex(1024);
        let mut writer = transport(client);
        let mut reader = transport(server);

        write_line(
            &mut writer,
            &DaemonResponse {
                line_type: "response",
                protocol_version: PROTOCOL_VERSION,
                status: ResponseStatus::Ok,
                message: "daemon healthy",
                job_id: None,
            },
        )
        .await?;

        let Some(line) = reader.next().await else {
            anyhow::bail!("reader closed before receiving a protocol line");
        };

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line?)?,
            json!({
                "type": "response",
                "protocol_version": PROTOCOL_VERSION,
                "status": "ok",
                "message": "daemon healthy",
            })
        );

        Ok(())
    }
}
```

- [ ] **Step 3: Point daemon at the protocol crate**

Add the dependency in `crates/daemon/Cargo.toml`:

```toml
protocol = { path = "../protocol" }
```

Remove the internal module from `crates/daemon/src/lib.rs`:

```rust
mod client;
mod dns;
mod error;
mod ipc;
mod jobs;
mod project_env;
mod reconciliation;
mod server;
mod supervisor;
mod watcher;
```

Keep the public version re-export in `crates/daemon/src/lib.rs`:

```rust
pub use protocol::PROTOCOL_VERSION;
```

Add transparent protocol conversion in `crates/daemon/src/error.rs`:

```rust
use protocol::ProtocolError;
```

```rust
#[error(transparent)]
Protocol(#[from] ProtocolError),
```

Keep the existing `Json` and `Frame` variants because daemon still parses inbound JSON and reads framed lines directly.

- [ ] **Step 4: Update daemon imports**

In `crates/daemon/src/client.rs`, replace:

```rust
use crate::protocol::{DaemonCommand, DaemonRequest, PROTOCOL_VERSION, ResponseStatus, write_line};
```

with:

```rust
use protocol::{DaemonCommand, DaemonRequest, PROTOCOL_VERSION, ResponseStatus, write_line};
```

Also replace:

```rust
let mut transport = crate::protocol::transport(stream);
```

with:

```rust
let mut transport = protocol::transport(stream);
```

In `crates/daemon/src/server.rs`, replace the protocol import block with:

```rust
use protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, DaemonTransport, PROTOCOL_VERSION,
    ResponseStatus, write_line,
};
```

Also replace test imports:

```rust
use protocol::transport;
```

In `crates/daemon/src/jobs.rs`, replace the protocol import block with:

```rust
use protocol::{
    DaemonEvent, DaemonResponse, DaemonTransport, PROTOCOL_VERSION, ResponseStatus, write_line,
};
```

Also replace:

```rust
crate::protocol::transport(server)
```

with:

```rust
protocol::transport(server)
```

- [ ] **Step 5: Delete the old daemon protocol module**

Remove `crates/daemon/src/protocol.rs`.

Run:

```bash
rg -n "crate::protocol|mod protocol" crates/daemon/src
```

Expected: no matches.

- [ ] **Step 6: Refresh Cargo.lock for the new internal crate**

Run:

```bash
cargo check -p protocol -p daemon
```

Expected: command passes and `Cargo.lock` changes only to add the internal `protocol` package and daemon's dependency edge to it. Do not run `cargo update` for this task.

- [ ] **Step 7: Verify protocol extraction**

Run:

```bash
cargo nextest run -p protocol -p daemon --locked
```

Expected: all selected tests pass, including `protocol::tests::transport_frames_generic_async_streams` and existing daemon protocol/job tests.

- [ ] **Step 8: Commit protocol extraction**

Run:

```bash
git add Cargo.toml Cargo.lock crates/protocol crates/daemon
git commit -m "refactor: extract daemon protocol crate"
```

## Task 2: Move The macOS Crate Into platform

**Files:**

- Move: `crates/macos/` -> `crates/platform/`
- Modify: `Cargo.toml`
- Modify: `crates/platform/Cargo.toml`
- Modify: `crates/platform/src/lib.rs`
- Modify: `crates/platform/tests/resolver_config.rs`
- Modify: `crates/cli/Cargo.toml`
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/cli/src/error.rs`
- Modify: `crates/cli/src/commands/ca.rs`
- Modify: `crates/cli/src/commands/dns.rs`
- Modify: `crates/cli/src/commands/ports.rs`
- Modify: `crates/cli/tests/ca.rs`
- Modify: `crates/cli/tests/dns.rs`
- Modify: `crates/cli/tests/ports.rs`
- Modify: `Cargo.lock`

- [ ] **Step 1: Move the crate directory**

Run:

```bash
git mv crates/macos crates/platform
```

Update the root workspace members in `Cargo.toml`:

```toml
[workspace]
members = [
    ".",
    "crates/cli",
    "crates/config",
    "crates/daemon",
    "crates/platform",
    "crates/protocol",
    "crates/resources",
    "crates/state",
]
resolver = "3"
```

Update `crates/platform/Cargo.toml`:

```toml
[package]
name = "platform"
version = "0.1.0"
edition.workspace = true
publish.workspace = true
```

Keep the existing dependencies from the old `macos` crate manifest.

- [ ] **Step 2: Rename host-facing public names**

In `crates/platform/src/lib.rs`, rename the error type:

```rust
pub enum PlatformError {
```

Replace every `MacosError` reference in `crates/platform/src/lib.rs` with `PlatformError`.

In `crates/platform/src/lib.rs`, rename the native trust inspector:

```rust
pub struct NativeSystemTrustInspector;
```

Replace:

```rust
impl SystemTrustInspector for MacosSystemTrustInspector {
```

with:

```rust
impl SystemTrustInspector for NativeSystemTrustInspector {
```

Keep macOS-specific implementation details, constants, and Keychain terminology inside `platform`; do not introduce Linux or Windows modules.

- [ ] **Step 3: Update platform tests**

In `crates/platform/tests/resolver_config.rs`, replace:

```rust
use macos::{
```

with:

```rust
use platform::{
```

Replace test error references:

```rust
platform::PlatformError
```

and:

```rust
platform::PlatformError::Keychain("fixture failure".to_string())
```

Run:

```bash
cargo nextest run -p platform --locked
```

Expected before snapshot acceptance: tests may pass, or insta may report new snapshot names because the crate moved from `macos` to `platform`.

If insta reports only crate-name/path snapshot changes, run:

```bash
cargo insta test --accept --test-runner nextest --manifest-path crates/platform/Cargo.toml
```

Then rerun:

```bash
cargo nextest run -p platform --locked
```

Expected: all platform tests pass.

- [ ] **Step 4: Route CLI through platform**

In `crates/cli/Cargo.toml`, replace:

```toml
macos = { path = "../macos" }
```

with:

```toml
platform = { path = "../platform" }
```

In `crates/cli/src/environment.rs`, replace default host integration methods with:

```rust
fn resolver_test_path(&self) -> PathBuf {
    PathBuf::from(platform::SYSTEM_RESOLVER_TEST_PATH)
}

fn pf_anchor_path(&self) -> PathBuf {
    PathBuf::from(platform::SYSTEM_PF_ANCHOR_PATH)
}

fn pf_conf_path(&self) -> PathBuf {
    PathBuf::from(platform::SYSTEM_PF_CONF_PATH)
}

fn loopback_tcp_listener_ports(
    &self,
) -> Result<std::collections::BTreeSet<u16>, platform::PlatformError> {
    platform::loopback_tcp_listener_ports()
}

fn trusted_ca_certificates(
    &self,
) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
    platform::SystemTrustInspector::trusted_certificates(&platform::NativeSystemTrustInspector)
}
```

In `crates/cli/src/error.rs`, replace:

```rust
Macos(#[from] macos::MacosError),
```

with:

```rust
Platform(#[from] platform::PlatformError),
```

- [ ] **Step 5: Update CLI command imports and references**

In `crates/cli/src/commands/dns.rs`, replace:

```rust
use macos::{ResolverConfig, ResolverFileState};
```

with:

```rust
use platform::{ResolverConfig, ResolverFileState};
```

Replace `macos::inspect_resolver_file` with `platform::inspect_resolver_file`.

In `crates/cli/src/commands/ports.rs`, replace:

```rust
use macos::{PfConfReference, PfFileState, PfRedirectConfig};
```

with:

```rust
use platform::{PfConfReference, PfFileState, PfRedirectConfig};
```

Replace:

- `macos::inspect_pf_anchor_file` -> `platform::inspect_pf_anchor_file`
- `macos::inspect_pf_conf_reference` -> `platform::inspect_pf_conf_reference`

In `crates/cli/src/commands/ca.rs`, replace:

```rust
use macos::{CaFileState, GeneratedLocalCa, LocalCaMetadata, TrustDomainState};
```

with:

```rust
use platform::{CaFileState, GeneratedLocalCa, LocalCaMetadata, TrustDomainState};
```

Replace:

- `macos::generate_local_ca` -> `platform::generate_local_ca`
- `macos::inspect_local_ca_files` -> `platform::inspect_local_ca_files`
- `macos::inspect_system_ca_trust` -> `platform::inspect_system_ca_trust`
- `macos::SystemTrustInspector` -> `platform::SystemTrustInspector`
- `macos::KeychainCertificate` -> `platform::KeychainCertificate`
- `macos::MacosError` -> `platform::PlatformError`
- `macos::MacosError::LocalCaPostWriteMissing` -> `platform::PlatformError::LocalCaPostWriteMissing`
- `macos::MacosError::LocalCaPostWriteRepairRequired` -> `platform::PlatformError::LocalCaPostWriteRepairRequired`
- `macos::MacosError::LocalCaPostWriteUnreadable` -> `platform::PlatformError::LocalCaPostWriteUnreadable`

In the `#[cfg(test)]` module of `crates/cli/src/commands/ca.rs`, replace:

```rust
use macos::{CaRepairReason, MacosError};
```

with:

```rust
use platform::{CaRepairReason, PlatformError};
```

Then replace `MacosError::` with `PlatformError::`.

- [ ] **Step 6: Update CLI test imports and trait signatures**

In `crates/cli/tests/dns.rs`, replace:

```rust
use macos::ResolverConfig;
```

with:

```rust
use platform::ResolverConfig;
```

In `crates/cli/tests/ports.rs`, replace:

```rust
use macos::{PfConfReference, PfRedirectConfig};
```

with:

```rust
use platform::{PfConfReference, PfRedirectConfig};
```

Also replace:

```rust
) -> Result<BTreeSet<u16>, macos::MacosError> {
```

with:

```rust
) -> Result<BTreeSet<u16>, platform::PlatformError> {
```

In `crates/cli/tests/ca.rs`, replace:

```rust
use macos::{KeychainCertificate, KeychainTrustResult, MacosError, generate_local_ca};
```

with:

```rust
use platform::{KeychainCertificate, KeychainTrustResult, PlatformError, generate_local_ca};
```

Replace:

```rust
Result<Vec<KeychainCertificate>, MacosError>
```

with:

```rust
Result<Vec<KeychainCertificate>, PlatformError>
```

Replace `MacosError::Keychain` with `PlatformError::Keychain`.

- [ ] **Step 7: Refresh Cargo.lock for the internal crate rename**

Run:

```bash
cargo check -p platform -p cli
```

Expected: command passes and `Cargo.lock` replaces the internal `macos` package entry with `platform`. No third-party dependency versions should change.

- [ ] **Step 8: Verify there are no direct macos crate imports**

Run:

```bash
rg -n "macos::|use macos|macos =" crates Cargo.toml
```

Expected: no matches.

Run:

```bash
cargo nextest run -p cli -p platform --locked
```

Expected: all selected tests pass. If snapshot names changed only because `macos` became `platform`, accept with:

```bash
cargo insta test --accept --test-runner nextest --manifest-path crates/platform/Cargo.toml
cargo insta test --accept --test-runner nextest --manifest-path crates/cli/Cargo.toml
```

Then rerun:

```bash
cargo nextest run -p cli -p platform --locked
```

Expected: all selected tests pass.

- [ ] **Step 9: Commit platform move**

Run:

```bash
git add Cargo.toml Cargo.lock crates/platform crates/cli
git add -u crates/macos
git commit -m "refactor: move macos integration into platform crate"
```

## Task 3: Update Architecture Documentation

**Files:**

- Modify: `DESIGN.md`
- Modify: `IMPLEMENTATION.md`

- [ ] **Step 1: Update DESIGN.md platform scope**

In `DESIGN.md`, keep the existing macOS-only v1 language and add this paragraph after the current macOS platform-scope paragraph:

```markdown
PV uses a host platform boundary in the Rust workspace. Application crates such as `cli`, `daemon`, `state`, `config`, and `resources` should not depend directly on macOS implementation APIs. The v1 concrete host platform implementation is macOS-only, but app-facing code should call the `platform` crate for host integration concerns.
```

- [ ] **Step 2: Update IMPLEMENTATION.md M0 package table**

In `IMPLEMENTATION.md`, add a new M0 row after `PV-004`:

```markdown
| PV-005 | Add host platform and daemon protocol boundaries | Enabler | PV-001, PV-025, PR 10, PR 11, PR 12 | PR 13, future host integration work | Workspace has `platform` and `protocol` crates; app crates do not depend directly on macOS host integration APIs; daemon wire types live outside daemon runtime code. |
```

- [ ] **Step 3: Update IMPLEMENTATION.md crate layout**

Replace the baseline crate layout block with:

```text
crates/
  cli/
  daemon/
  state/
  config/
  resources/
  protocol/
  platform/
```

- [ ] **Step 4: Update IMPLEMENTATION.md crate ownership table**

Replace the `macos` ownership row with these rows:

```markdown
| `protocol` | Shared daemon wire contracts: protocol version, request/response envelopes, job/progress event schema, and framing helpers that do not depend on daemon runtime logic. |
| `platform` | Host OS integration boundary. The v1 concrete implementation is macOS: LaunchAgent, `/etc/resolver/test`, `pf`, System keychain CA trust, shell profile targets, and privileged command helpers. |
```

Add this paragraph after the crate ownership table:

```markdown
Application crates should depend on `platform`, not directly on host-specific implementation crates or modules. Code belongs in `platform` when it decides, inspects, mutates, models, or names host OS integration. Unix filesystem permissions, symlink mechanics, Unix sockets, and signal handling may remain in their owning domain crates when they are local mechanics rather than host integration policy.
```

- [ ] **Step 5: Add the refactor PR row**

In the Suggested PR Sequence table, insert this row after PR 12 and before PR 13:

```markdown
| PR 12A | Platform and protocol workspace boundary refactor | PV-005 | PR 10, PR 11, PR 12 | No | No |
```

Update the PR 13 dependency cell from:

```markdown
PR 7, PR 10, PR 11, PR 12
```

to:

```markdown
PR 7, PR 10, PR 11, PR 12, PR 12A
```

- [ ] **Step 6: Verify documentation references**

Run:

```bash
rg -n "crates/macos|`macos`|macos crate" DESIGN.md IMPLEMENTATION.md docs/superpowers/specs/2026-06-04-platform-protocol-boundaries-design.md
```

Expected: no stale references that describe `macos` as the current workspace crate. Historical references in the committed design spec are acceptable only where they describe the old crate being moved.

- [ ] **Step 7: Commit docs update**

Run:

```bash
git add DESIGN.md IMPLEMENTATION.md
git commit -m "docs: record platform protocol boundaries"
```

## Task 4: Boundary Audit And Final Verification

**Files:**

- Inspect: `Cargo.toml`
- Inspect: `crates/*/Cargo.toml`
- Inspect: `crates/**/*.rs`
- Inspect: `DESIGN.md`
- Inspect: `IMPLEMENTATION.md`

- [ ] **Step 1: Audit direct macOS crate leakage**

Run:

```bash
rg -n "macos::|use macos|macos =" crates Cargo.toml
```

Expected: no matches.

- [ ] **Step 2: Audit target-specific host integration outside platform**

Run:

```bash
rg -n "#\\[cfg|cfg\\(" crates -g '*.rs'
```

Expected reviewed categories:

- `#[cfg(test)]` remains in tests and test modules.
- `#[cfg(unix)]` in `state`, `resources`, `config`, and `daemon` remains only for local Unix mechanics.
- `#[cfg(target_os = "macos")]` appears only inside `crates/platform`, if it appears at all.

If `target_os = "macos"` appears outside `crates/platform`, move that host integration logic into `platform` or document why it is not host integration before committing.

- [ ] **Step 3: Check workspace metadata**

Run:

```bash
cargo metadata --format-version=1 --no-deps
```

Expected:

- workspace members include `platform`
- workspace members include `protocol`
- workspace members do not include `macos`

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo nextest run -p protocol -p daemon -p platform -p cli --locked
```

Expected: all selected tests pass.

- [ ] **Step 5: Run formatting and diff checks**

Run:

```bash
cargo fmt --all -- --check
git diff --check
```

Expected: both commands pass.

- [ ] **Step 6: Run workspace build**

Run:

```bash
cargo build --workspace --locked
```

Expected: build passes.

- [ ] **Step 7: Commit final verification-only fixes if needed**

If formatting or snapshot acceptance changed files after the prior commits, commit only those mechanical updates:

```bash
git add .
git commit -m "test: update platform boundary snapshots"
```

If no files changed, do not create an empty commit.

- [ ] **Step 8: Final branch status**

Run:

```bash
git status --short --branch
git log --oneline origin/main..HEAD
```

Expected:

- branch is `refactor/platform-protocol-boundaries`
- working tree has no tracked changes
- commits include the design spec, this implementation plan, and the refactor commits

## PR Review Notes

When opening the PR, call out:

- PV v1 remains macOS-only.
- `protocol` is pure daemon wire contract and does not depend on application crates.
- `platform` is the host integration boundary and currently contains the moved macOS implementation.
- Machine-global owner metadata is intentionally not included.
- Remaining `#[cfg(unix)]` usage outside `platform` was audited as local domain mechanics.
