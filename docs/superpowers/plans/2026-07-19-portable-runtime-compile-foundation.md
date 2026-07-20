# Portable Runtime Compile Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the installed PV runtime compile natively on macOS, Linux, and Windows; keep help, version, and completion generation usable on every target; and make unfinished Linux and Windows host behavior fail explicitly instead of selecting Darwin behavior or invoking macOS mechanisms.

**Architecture:** Introduce a small typed host-target/capability model in `platform`, keep macOS as the only supported implementation, and add compile-time Linux/Windows stubs at the narrow crates that own filesystem policy, daemon IPC, process containment, and artifact selection. Gate host-dependent CLI entrypoints before side effects. This plan is the compile-foundation slice of the approved architecture, not the later semantic renaming, diagnostics, helper-replacement, or lifecycle-library work.

**Tech Stack:** Rust 2024, Tokio, Clap, thiserror, rustix on Unix targets, GitHub Actions native macOS/Linux/Windows runners, cargo-nextest, `insta`.

## Global Constraints

- Before implementation, read `CONTRIBUTING.md`, `DESIGN.md`, and `docs/superpowers/specs/2026-07-19-portable-platform-architecture-design.md` in full.
- Use the `superpowers:using-git-worktrees` skill before implementation so this multi-crate refactor is isolated from the current workspace.
- Preserve all existing observable macOS behavior and snapshots unless this plan explicitly names a new unsupported-platform message.
- macOS 13+ remains the only supported and published product target. Do not publish, advertise, or add release artifacts for Linux or Windows.
- Limit workspace scope to root `pv`, `cli`, `config`, `daemon`, `platform`, `protocol`, `resources`, `self-update`, and `state`. Do not modify `pv-release`, release recipes, publication workflows, installer generation, or signing behavior.
- Do not implement Linux or Windows resolver, firewall/low-port, trust-store, service registration, local IPC, process containment, filesystem ACL, resource artifact, or self-update behavior in this slice.
- Do not add a giant `Platform` trait, runtime operating-system dispatch, platform-specific crates, or new third-party dependencies.
- Keep target conditionals in the narrow module that owns the behavior. CLI conditionals may select tests or smoke commands, but application workflows must not branch on `launchd`, PF, Keychain, systemd, SCM, CryptoAPI, or named pipes.
- A Linux/Windows stub must return a typed unsupported error before creating files, spawning processes, selecting an artifact, or running a host command. Never return fabricated success or an empty observation.
- Do not treat Linux as equivalent to macOS merely because both are Unix. In particular, daemon IPC and macOS integration code select `target_os = "macos"`, not `cfg(unix)`.
- Do not update dependencies or `Cargo.lock`. If a target-gated dependency declaration unexpectedly changes the lockfile, restore the lockfile and use the existing version.
- Prefer integration tests and nearby `insta` snapshot patterns. For typed domain errors, assert the variant and fields before asserting display text.
- Avoid `panic!`, `unreachable!`, `.unwrap()`, unsafe code, broad Clippy ignores, and speculative abstractions. Use top-level imports and descriptive names.
- Use the exact Conventional Commit message listed at the end of each task.
- This plan intentionally defers broad `status`/`doctor` unsupported observations and the removal of macOS mechanism nouns from orchestration APIs to a follow-up semantic-capability/diagnostics plan. Until that plan lands, native CI must not claim full diagnostic portability.

---

## File Structure

- Modify `DESIGN.md` to record macOS-first stabilization and committed subsequent Linux/Windows support.
- Add `crates/platform/src/target.rs` for the shared `PlatformTarget` value.
- Add `crates/platform/src/capability.rs` for semantic `PlatformCapability` values and support checks.
- Modify `crates/platform/src/error.rs`, `crates/platform/src/lib.rs`, `crates/platform/src/trust.rs`, and `crates/platform/src/ca.rs` to expose and consume typed unsupported errors.
- Modify `crates/platform/Cargo.toml` so `security-framework` and Unix-only `rustix` usage are target-gated.
- Modify `crates/state/src/error.rs`, `crates/state/src/fs.rs`, `crates/state/src/update_lock.rs`, `crates/state/src/lib.rs`, and `crates/state/Cargo.toml` to compile on Windows and reject unavailable owner-only filesystem/locking behavior before side effects.
- Modify `crates/config/src/error.rs`, `crates/config/src/filesystem.rs`, and `crates/config/src/lib.rs` to preserve portable reads while rejecting permission-preserving writes on Windows.
- Modify `crates/resources/src/error.rs`, `crates/resources/src/platform.rs`, `crates/resources/src/fs.rs`, `crates/resources/src/install.rs`, and `crates/resources/src/lib.rs` to reject unsupported host targets and filesystem mechanics explicitly.
- Add `crates/daemon/src/ipc/unsupported.rs` and modify `crates/daemon/src/ipc/mod.rs`, `crates/daemon/src/ipc/unix.rs`, and `crates/daemon/src/client.rs` to hide the concrete transport and select macOS Unix sockets only.
- Modify `crates/daemon/src/supervisor.rs`, `crates/daemon/src/gateway.rs`, `crates/daemon/src/error.rs`, `crates/daemon/src/managed_resources/mod.rs`, `crates/daemon/src/jobs.rs`, `crates/daemon/src/lib.rs`, and `crates/daemon/Cargo.toml` to isolate macOS process-group mechanics, propagate fallible artifact targets, and return typed unsupported errors elsewhere.
- Modify `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/setup.rs`, `crates/cli/src/commands/artifact_resource.rs`, `crates/cli/src/commands/php.rs`, and `crates/cli/src/commands/composer.rs` to gate host commands and stop defaulting non-macOS hosts to Darwin artifacts.
- Modify `.github/workflows/ci.yml` to add non-publishing native Linux/Windows runtime compile and smoke jobs while retaining the full macOS behavior gate.
- Do not modify existing macOS snapshots unless a test added by this plan has a new snapshot file.

---

### Task 1: Record The Product Platform Direction

**Files:**
- Modify: `DESIGN.md:24-32`

**Interfaces:**
- Replaces the statement that Linux and Windows are “not guaranteed.”
- Keeps PV v1 and current distribution macOS-only.
- Establishes native compile validation as architecture work, not product support.

- [ ] **Step 1: Replace the stale platform-scope paragraph**

Update `DESIGN.md` under `## Platform Scope` so it states all of the following without changing the surrounding macOS v1 decisions:

```markdown
PV v1 targets macOS 13 and newer. Stabilizing the macOS application remains the immediate product priority.

Linux and Windows are committed subsequent platforms. During macOS stabilization, the installed application and runtime crates compile natively on macOS, Linux, and Windows so new system boundaries do not create unnecessary portability blockers.

Native compile support and explicit unsupported behavior do not make Linux or Windows supported distributions, add them to v1, or authorize publishing Linux or Windows binaries.
```

Keep the existing statement that application crates call the `platform` boundary, but revise “the v1 concrete host platform implementation is macOS-only” to clarify that unfinished Linux/Windows implementations return explicit unsupported results.

- [ ] **Step 2: Verify the design text against the approved spec**

Run:

```shell
rg -n "Platform Scope|committed subsequent|not guaranteed|unsupported" DESIGN.md
git diff --check
```

Expected: `DESIGN.md` contains the committed roadmap, no longer says support is “not guaranteed,” still says v1 is macOS-only, and `git diff --check` exits zero.

- [ ] **Step 3: Commit Task 1**

```shell
git add DESIGN.md
git commit -m "docs: record committed platform roadmap"
```

---

### Task 2: Add Typed Host Targets And Capabilities

**Files:**
- Create: `crates/platform/src/target.rs`
- Create: `crates/platform/src/capability.rs`
- Modify: `crates/platform/src/error.rs`
- Modify: `crates/platform/src/lib.rs`
- Modify: `crates/platform/src/trust.rs`
- Modify: `crates/platform/src/ca.rs`

**Interfaces:**
- Produces `PlatformTarget::{Macos, Linux, Windows}` and `PlatformTarget::current()`.
- Produces semantic `PlatformCapability` values needed in this slice: `BrowserHandoff`, `DaemonIpc`, `DaemonRegistration`, `ListenerInspection`, `LowPortFrontend`, `ProcessContainment`, `ResolverIntegration`, and `TrustStore`.
- Replaces `PlatformError::UnsupportedPlatform { feature }` with `PlatformError::Unsupported { capability, target }`.
- Produces `platform::require_capability(capability)` and `platform::unsupported(capability)`; neither performs runtime dispatch or host work.

- [ ] **Step 1: Write failing typed-error tests**

Replace `unsupported_platform_error_names_feature` in `crates/platform/src/lib.rs` tests with tests that assert the variant fields and rendered message:

```rust
#[test]
fn unsupported_error_names_capability_and_target() {
    let error = PlatformError::Unsupported {
        capability: PlatformCapability::DaemonRegistration,
        target: PlatformTarget::Linux,
    };

    assert!(matches!(
        &error,
        PlatformError::Unsupported {
            capability: PlatformCapability::DaemonRegistration,
            target: PlatformTarget::Linux,
        }
    ));
    assert_eq!(
        error.to_string(),
        "daemon registration is unsupported on linux"
    );
}

#[test]
fn capability_check_accepts_macos_and_rejects_windows() {
    assert!(require_capability_for(
        PlatformTarget::Macos,
        PlatformCapability::TrustStore,
    )
    .is_ok());

    let error = require_capability_for(
        PlatformTarget::Windows,
        PlatformCapability::TrustStore,
    );
    assert!(matches!(
        error,
        Err(PlatformError::Unsupported {
            capability: PlatformCapability::TrustStore,
            target: PlatformTarget::Windows,
        })
    ));
}
```

Keep `require_capability_for` crate-private; the public `require_capability` supplies `PlatformTarget::current()`.

- [ ] **Step 2: Run the tests to prove the new API is absent**

Run:

```shell
cargo test -p platform --lib unsupported --locked
```

Expected: compilation fails because `PlatformTarget`, `PlatformCapability`, and the new error variant do not exist yet.

- [ ] **Step 3: Implement the target and capability values**

In `target.rs`, define the three approved targets with `Display` values `macos`, `linux`, and `windows`. Implement `PlatformTarget::current()` using mutually exclusive `#[cfg(target_os = ...)]` functions for those three targets. For any other Rust target, return a typed `PlatformError::UnsupportedTarget { target: std::env::consts::OS }` from a fallible `PlatformTarget::current()`; do not add a fourth product target or a compile error.

Use this public shape:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformTarget {
    Macos,
    Linux,
    Windows,
}

impl PlatformTarget {
    pub fn current() -> Result<Self, PlatformError>;
    pub const fn as_str(self) -> &'static str;
}
```

In `capability.rs`, implement `Display` with the exact lower-case phrases named by each variant. `require_capability_for` returns `Ok(())` only for `Macos` in this foundation slice. `require_capability` obtains the current target, and `unsupported` constructs the typed error for the current target.

Do not imply that every capability will remain macOS-only; this support table is intentionally the starting implementation and will change when Linux/Windows backends are added.

- [ ] **Step 4: Migrate existing unsupported trust errors**

Replace every `PlatformError::UnsupportedPlatform { feature: ... }` in `trust.rs` with `PlatformError::Unsupported { capability: PlatformCapability::TrustStore, target: PlatformTarget::current()? }` or the shared `unsupported(PlatformCapability::TrustStore)` helper.

Update the exhaustive error classification in `ca.rs` for `Unsupported` and `UnsupportedTarget`. Do not change certificate generation, macOS Keychain behavior, or existing macOS trust snapshots.

- [ ] **Step 5: Export only the semantic values**

From `platform::lib`, publicly export `PlatformCapability`, `PlatformTarget`, and `require_capability`. Keep `require_capability_for` crate-private for tests. Export `unsupported` only if `daemon` needs it in Task 5; otherwise keep it crate-private and construct the error from the public types at the daemon boundary.

- [ ] **Step 6: Run focused and crate tests**

Run:

```shell
cargo test -p platform --lib unsupported --locked
cargo nextest run -p platform --locked
cargo fmt --all --check
git diff --check
```

Expected: all commands exit zero; the message is exactly `daemon registration is unsupported on linux`; existing macOS platform tests remain green.

- [ ] **Step 7: Commit Task 2**

```shell
git add crates/platform/src/target.rs crates/platform/src/capability.rs crates/platform/src/error.rs crates/platform/src/lib.rs crates/platform/src/trust.rs crates/platform/src/ca.rs
git commit -m "refactor(platform): add typed host capabilities"
```

---

### Task 3: Isolate Owner-Only Filesystem And Locking Policy

**Files:**
- Modify: `crates/state/src/error.rs`
- Modify: `crates/state/src/fs.rs`
- Modify: `crates/state/src/update_lock.rs`
- Modify: `crates/state/src/lib.rs`
- Modify: `crates/state/Cargo.toml`
- Modify: `crates/config/src/error.rs`
- Modify: `crates/config/src/filesystem.rs`
- Modify: `crates/config/src/lib.rs`

**Interfaces:**
- Produces `StateError::UnsupportedPlatform { capability, target }` with a typed `StateCapability::{OwnerOnlyFilesystem, FileLocking, SymbolicLinks}`.
- Produces `ConfigError::UnsupportedPlatform { capability, target }` with typed `ConfigCapability::PermissionPreservingWrite`.
- Keeps portable reads and path inspection available on Windows.
- Rejects protected writes before creating a directory, temporary file, lock file, or symlink.

- [ ] **Step 1: Add failing domain error tests**

Add unit tests beside the error definitions that construct the new typed variants and assert both fields and exact messages:

```text
owner-only filesystem is unsupported on windows
permission-preserving config writes are unsupported on windows
```

Also add a pure helper test for `unsupported_target_name()` (or the equivalent private helper) that accepts an injected `"windows"`; do not try to fake `cfg` in a test.

- [ ] **Step 2: Run focused tests and capture the red compile result**

Run:

```shell
cargo test -p state --lib unsupported --locked
cargo test -p config --lib unsupported --locked
```

Expected: compilation fails because the new variants and capability enums do not exist.

- [ ] **Step 3: Make state filesystem policy fail before side effects**

Remove the `compile_error!` from `state::fs`. Keep the existing Unix implementation unchanged behind `cfg(unix)`. Add non-Unix helpers that return `StateError::UnsupportedPlatform` and call them at the start of operations that promise owner-only behavior:

- `ensure_user_dir`
- `write_sensitive_file` through its parent/permission path
- `open_append_file`
- `secure_sensitive_file`
- `secure_executable_file`
- `inspect_layout`
- `inspect_database_files`
- `symlink_file`

Refactor `current_uid`, `owner_uid`, and `mode` only as much as required to return `Result` on every target. A non-Unix helper must not invent a UID or mode. Leave portable read-only helpers such as `read_to_string`, `path_exists`, `path_is_file`, and `read_dir_paths` functional.

The non-Unix path must return before `create_dir_all`, `std::fs::write`, `OpenOptions::create`, or temporary-file creation. Add a Windows-only unit test named `unsupported_owner_only_directory_does_not_create_path` that checks the requested path is still absent after `ensure_user_dir` returns unsupported; this test runs in native Windows CI, not on macOS.

- [ ] **Step 4: Isolate update locking**

Move the `rustix::fs::flock` calls behind `cfg(unix)` private functions. On non-Unix:

- `UpdateLock::acquire` returns `StateCapability::FileLocking` before creating the lock file;
- `require_no_update_in_progress` may still return `Ok(())` when the lock path does not exist, because no locking operation was requested;
- if an existing lock path requires inspection, return typed unsupported instead of treating it as unlocked.

Move `rustix` in `crates/state/Cargo.toml` to `[target.'cfg(unix)'.dependencies]`. Keep the workspace dependency and lockfile unchanged.

- [ ] **Step 5: Make config writes fail before temporary-file creation**

Remove the `compile_error!` from `config::filesystem`. Keep path discovery, canonicalization, directory checks, and reads portable. On non-Unix, make `file_mode` and `write_string_atomically_with_mode` return `ConfigCapability::PermissionPreservingWrite` before opening or creating any file.

Do not silently ignore the supplied Unix mode on Windows. Add a Windows-only test named `unsupported_permission_write_does_not_create_temporary_file` that proves `write_string_atomically_with_mode` leaves both the target and derived temporary path absent.

- [ ] **Step 6: Run focused macOS regressions**

Run:

```shell
cargo nextest run -p state -p config --locked
cargo fmt --all --check
cargo shear
git diff --check
```

Expected: all existing macOS state/config tests pass without snapshot changes; `cargo shear` accepts the target-gated dependency; no lockfile change exists.

- [ ] **Step 7: Commit Task 3**

```shell
git add crates/state/src/error.rs crates/state/src/fs.rs crates/state/src/update_lock.rs crates/state/src/lib.rs crates/state/Cargo.toml crates/config/src/error.rs crates/config/src/filesystem.rs crates/config/src/lib.rs
git commit -m "refactor(state): isolate host filesystem policy"
```

---

### Task 4: Reject Unsupported Resource Hosts Instead Of Selecting Darwin

**Files:**
- Modify: `crates/resources/src/error.rs`
- Modify: `crates/resources/src/platform.rs`
- Modify: `crates/resources/src/fs.rs`
- Modify: `crates/resources/src/install.rs`
- Modify: `crates/resources/src/lib.rs`

**Interfaces:**
- Produces `TargetPlatform::current() -> resources::Result<TargetPlatform>`.
- Preserves `DarwinArm64` and `DarwinAmd64` manifest identities.
- Produces `ResourcesError::UnsupportedHostCapability { capability, target }` for non-Unix permission and symlink operations.
- Never maps Linux/Windows architecture to a Darwin artifact platform.

- [ ] **Step 1: Write failing target-selection tests**

Extract a private pure helper `target_platform_for(os, arch)` and test this matrix in `resources::platform`:

| OS | architecture | expected |
|---|---|---|
| `macos` | `aarch64` | `DarwinArm64` |
| `macos` | `x86_64` | `DarwinAmd64` |
| `linux` | `x86_64` | `ResourcesError::UnsupportedPlatform { platform: "linux-x86_64" }` |
| `windows` | `x86_64` | `ResourcesError::UnsupportedPlatform { platform: "windows-x86_64" }` |

Use typed assertions. Add an `insta` debug snapshot for the full matrix only if the neighboring platform tests already use snapshots; otherwise typed equality is clearer for this domain error.

- [ ] **Step 2: Run the focused test and confirm the missing API**

Run:

```shell
cargo test -p resources --lib target_platform_for --locked
```

Expected: compilation fails because `TargetPlatform::current` and `target_platform_for` do not exist.

- [ ] **Step 3: Implement fallible current-target selection**

Implement:

```rust
impl TargetPlatform {
    pub fn current() -> Result<Self> {
        target_platform_for(std::env::consts::OS, std::env::consts::ARCH)
    }
}
```

Only the two existing Darwin combinations succeed. Use the current `ResourcesError::UnsupportedPlatform` for an unsupported artifact target; this is distinct from a missing filesystem capability.

- [ ] **Step 4: Type the existing non-Unix filesystem stubs**

Add a small `ResourceHostCapability::{OwnerOnlyFilesystem, SymbolicLinks}` enum and `ResourcesError::UnsupportedHostCapability { capability, target }`. Replace generic `ResourcesError::Filesystem` strings in the non-Unix branches of `fs.rs` and `install.rs` with the typed variant.

Ensure `write_atomically_with` reaches the non-Unix owner-only check before creating its temporary file. Ensure `symlink_dir` returns unsupported before modifying the link path.

- [ ] **Step 5: Run resource regressions**

Run:

```shell
cargo nextest run -p resources --locked
cargo fmt --all --check
git diff --check
```

Expected: all resource tests pass on macOS, the target matrix passes, and no manifest format or existing Darwin identity changes.

- [ ] **Step 6: Commit Task 4**

```shell
git add crates/resources/src/error.rs crates/resources/src/platform.rs crates/resources/src/fs.rs crates/resources/src/install.rs crates/resources/src/lib.rs
git commit -m "refactor(resources): reject unsupported host targets"
```

---

### Task 5: Hide Daemon IPC And Process-Group Mechanics Behind Compile-Time Modules

**Files:**
- Create: `crates/daemon/src/ipc/unsupported.rs`
- Modify: `crates/daemon/src/ipc/mod.rs`
- Modify: `crates/daemon/src/ipc/unix.rs`
- Modify: `crates/daemon/src/client.rs`
- Modify: `crates/daemon/src/supervisor.rs`
- Modify: `crates/daemon/src/gateway.rs`
- Modify: `crates/daemon/src/error.rs`
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/daemon/src/jobs.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/Cargo.toml`

**Interfaces:**
- Selects the existing Unix-socket transport only for `target_os = "macos"`.
- Adds shared `ipc::{LocalListener, LocalStream, bind, connect, prepare_endpoint, remove_endpoint}` operations.
- Linux and Windows IPC operations return `PlatformCapability::DaemonIpc` unsupported errors.
- Linux and Windows process-supervisor entrypoints return `PlatformCapability::ProcessContainment` unsupported errors before spawning or signaling.
- Managed Resource runtime catalogs obtain their artifact target through fallible `resources::TargetPlatform::current()` rather than architecture-only Darwin defaults.
- Keeps protocol framing generic over the private `LocalStream` type.

- [ ] **Step 1: Add a failing IPC facade test**

In `daemon::ipc` tests, add a pure support-selection test that asks the facade to validate Linux and asserts:

```rust
assert!(matches!(
    require_ipc_for(PlatformTarget::Linux),
    Err(DaemonError::Platform(PlatformError::Unsupported {
        capability: PlatformCapability::DaemonIpc,
        target: PlatformTarget::Linux,
    }))
));
```

Keep `require_ipc_for` crate-private. Native compilation of `unsupported.rs` plus the CLI unsupported smoke in Task 7 provide the target-selected coverage; do not make private IPC internals public solely for an integration test.

- [ ] **Step 2: Run the focused test and capture the red compile result**

Run:

```shell
cargo test -p daemon --lib ipc --locked
```

Expected: compilation fails because the IPC facade has no `connect` or target support helper.

- [ ] **Step 3: Convert the IPC module to macOS plus unsupported implementations**

Change `ipc/mod.rs` selection from `cfg(unix)` to:

```rust
#[cfg(target_os = "macos")]
mod unix;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod unsupported;
```

Re-export the same shared names from the selected module. In `unix.rs`, add `connect(paths)` around `UnixStream::connect` and keep existing endpoint cleanup behavior unchanged.

In `unsupported.rs`, define compile-only `LocalListener` and `LocalStream` types that satisfy all shared signatures. `LocalStream` may wrap `tokio::io::DuplexStream`; no unsupported operation may return a constructed stream or listener. `bind`, `connect`, `prepare_endpoint`, and `remove_endpoint` return typed unsupported. `LocalListener::accept` exists only so `server.rs` compiles and returns an `io::ErrorKind::Unsupported`; it is unreachable because `bind` cannot succeed.

Do not add Linux Unix sockets. That is deliberately deferred so Linux does not accidentally become a partially supported backend.

- [ ] **Step 4: Remove `UnixStream` from the client boundary**

Replace the direct `tokio::net::UnixStream` import and every `DaemonTransport<UnixStream>` in `client.rs` with `ipc::LocalStream`. Make `connect_transport` call `ipc::connect(paths)` inside the existing timeout.

Keep all NDJSON protocol timeouts, validation, event parsing, and public client functions unchanged.

- [ ] **Step 5: Isolate process-group mechanics**

Move the `rustix::process` import in `supervisor.rs` behind `cfg(target_os = "macos")`. Introduce a private semantic signal enum (`Reload`, `Terminate`, `Kill`) so shared supervisor methods do not mention `rustix::process::Signal`.

On macOS, map the semantic signal to the current rustix process-group calls without changing behavior. On Linux and Windows, guard these public behavior entrypoints before side effects:

- `ProcessSupervisor::start`
- `verify_ownership`
- `adopt`
- `adopt_recorded`
- `reload`
- `ManagedProcess::stop`
- `AdoptedProcess::stop`

Each returns a `PlatformCapability::ProcessContainment` unsupported error. `start` must reject before opening logs or spawning a child. Retain a best-effort direct `child.kill()` only in internal cleanup for a child already spawned by an earlier failure; cleanup is not a successful containment implementation.

Change process-group conditionals in `supervisor.rs` and `gateway.rs` from broad `cfg(unix)` to `cfg(target_os = "macos")`, including the `Pid`, `Signal`, and `kill_process_group` imports. Linux must use the unsupported path even though it is Unix. Do not redesign supervision or replace `/bin/ps` in this task; structured process inspection is a separate approved spike.

- [ ] **Step 6: Target-gate rustix**

Move `rustix` in `crates/daemon/Cargo.toml` to `[target.'cfg(target_os = "macos")'.dependencies]`. Keep platform/state ownership of their own rustix declarations separate.

- [ ] **Step 7: Propagate fallible Managed Resource targets**

In `managed_resources/mod.rs`, change `ManagedResourceRuntimeCatalog::production`, `without_adapters`, `without_adapters_with_manifest_url`, and `without_adapters_with_manifest_client` to return `Result<Self, DaemonError>`. Build `ManagedResourceInstallOptions` with `resources::TargetPlatform::current()?` and delete the architecture-only `current_target_platform()` helper.

Update production callers in `managed_resources/mod.rs`, `jobs.rs`, and `lib.rs` to propagate `?`. Update macOS tests in those modules to propagate the new `Result`; keep test-only catalogs that already receive an explicit `ManagedResourceInstallOptions` unchanged.

Search afterward:

```shell
rg -n "fn current_target_platform|cfg!\(target_arch" crates/daemon/src
```

Expected: no production daemon helper maps architecture alone to a Darwin platform.

- [ ] **Step 8: Run daemon regressions on macOS**

Run:

```shell
cargo nextest run -p daemon --lib --locked
cargo nextest run -p daemon --test supervisor_foundation --locked
cargo fmt --all --check
cargo shear
git diff --check
```

Expected: all existing daemon behavior passes on macOS, no supervisor snapshots change, `cargo shear` accepts the target-gated dependency, and `Cargo.lock` is unchanged.

- [ ] **Step 9: Commit Task 5**

```shell
git add crates/daemon/src/ipc/unsupported.rs crates/daemon/src/ipc/mod.rs crates/daemon/src/ipc/unix.rs crates/daemon/src/client.rs crates/daemon/src/supervisor.rs crates/daemon/src/gateway.rs crates/daemon/src/error.rs crates/daemon/src/managed_resources/mod.rs crates/daemon/src/jobs.rs crates/daemon/src/lib.rs crates/daemon/Cargo.toml
git commit -m "refactor(daemon): isolate host lifecycle mechanics"
```

---

### Task 6: Gate Host Commands And Use Fallible Artifact Targets

**Files:**
- Modify: `crates/platform/Cargo.toml`
- Modify: `crates/platform/src/launch_agent.rs`
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/commands/setup.rs`
- Modify: `crates/cli/src/commands/artifact_resource.rs`
- Modify: `crates/cli/src/commands/php.rs`
- Modify: `crates/cli/src/commands/composer.rs`

**Interfaces:**
- Produces a semantic `required_capability(&Command)` mapping at the CLI boundary.
- Rejects direct Linux/Windows host commands before update-lock inspection or filesystem creation.
- Replaces all CLI architecture-only Darwin defaults with `TargetPlatform::current()?`.
- Target-gates macOS-only dependencies without changing versions.

- [ ] **Step 1: Write failing command-capability tests**

In `cli::commands` tests, cover at least these mappings with typed equality:

| command | capability |
|---|---|
| `setup`, `uninstall` | `ResolverIntegration` (the first required setup capability) |
| `daemon:enable`, `daemon:disable`, `daemon:restart` | `DaemonRegistration` |
| `daemon:run` | `DaemonIpc` |
| `dns:status`, `dns:install`, `dns:uninstall` | `ResolverIntegration` |
| `ports:status`, `ports:install`, `ports:uninstall` | `LowPortFrontend` |
| `ca:status`, `ca:trust`, `ca:untrust` | `TrustStore` |
| `open`, `mailpit:open`, `mail:open`, `rustfs:open`, `s3:open` | `BrowserHandoff` |
| `status`, `doctor`, `update` | `DaemonRegistration` until the follow-up diagnostics/update capability plan |
| `completions` | no required host capability |

Do not add a blanket non-macOS guard. Help/version are handled by Clap before execution, and completions must reach its handler. `status` and `doctor` are temporarily fail-closed in this slice so they cannot invoke macOS inspection mechanisms on another host; the follow-up diagnostics plan replaces that gate with per-capability unsupported observations and continued collection.

- [ ] **Step 2: Run the focused CLI test and confirm the mapping is absent**

Run:

```shell
cargo test -p cli --lib required_capability --locked
```

Expected: compilation fails because `required_capability` does not exist.

- [ ] **Step 3: Gate semantic host commands before state access**

At the start of `commands::execute`, call a helper that:

1. obtains `required_capability(&cli.command)`;
2. calls `platform::require_capability` when one exists; and
3. returns the existing `ExecuteError::Platform` conversion on failure.

This check must run before `require_no_update_in_progress`, so `pv daemon:enable` on Windows reports daemon registration unsupported rather than creating or inspecting a lock/layout path.

Do not inspect `target_os` in CLI code and do not mention LaunchAgent, PF, Keychain, systemd, SCM, or named pipes in the mapping.

- [ ] **Step 4: Replace duplicated Darwin target defaults**

In `setup.rs`, `artifact_resource.rs`, `php.rs`, and `composer.rs`:

- change `target_platform(environment)` to return `Result<TargetPlatform, ExecuteError>`;
- preserve an injected `environment.target_platform()` for tests;
- otherwise call `TargetPlatform::current()?`;
- update `resource_commands` and direct callers to propagate `?`; and
- delete every local `current_target_platform()` that branches only on architecture.

Search afterward:

```shell
rg -n "fn current_target_platform|cfg!\(target_arch|DarwinArm64|DarwinAmd64" crates/cli/src
```

Expected: no production CLI helper maps architecture alone to a Darwin platform. Test fixtures may still name explicit Darwin variants.

- [ ] **Step 5: Guard direct macOS launch-domain code and dependencies**

In `platform::launch_agent`, put `rustix::process::getuid` behind `cfg(target_os = "macos")`. Every command capable of reaching LaunchAgent operations is already gated by the semantic CLI check; retain defensive typed unsupported returns in fallible launch-agent command functions where signatures permit it.

Move `security-framework` and `rustix` to `[target.'cfg(target_os = "macos")'.dependencies]` in `crates/platform/Cargo.toml`. Keep `plist` shared for this transitional slice; its public mechanism types are removed in the follow-up semantic-facade plan.

- [ ] **Step 6: Run CLI integration regressions**

Run:

```shell
cargo nextest run -p pv --test cli --locked \
  -E 'test(version_builds_and_runs_from_source) | test(daemon_run_is_hidden_from_top_level_help) | test(completions_generate_bash_script)'
cargo nextest run -p cli -p platform --locked
cargo fmt --all --check
cargo shear
git diff --check
```

Expected: existing portable command snapshots remain unchanged on macOS, command-capability tests pass, and the lockfile remains unchanged.

- [ ] **Step 7: Commit Task 6**

```shell
git add crates/platform/Cargo.toml crates/platform/src/launch_agent.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/setup.rs crates/cli/src/commands/artifact_resource.rs crates/cli/src/commands/php.rs crates/cli/src/commands/composer.rs
git commit -m "refactor(cli): gate unavailable host capabilities"
```

---

### Task 7: Add Native Linux And Windows Compile/Smoke Gates

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Retains the existing `macos-14` full Rust, lint, shellcheck, and nextest job.
- Adds native `ubuntu-latest` and `windows-latest` runtime-only jobs.
- Compiles only the installed runtime scope and never invokes release tooling or artifact publication.
- Runs the focused portable domain tests, including Windows no-side-effect stubs.
- Runs help, version, completions, and one direct unsupported command.

- [ ] **Step 1: Add the native runtime matrix**

Add a separate job after the existing macOS job:

```yaml
  runtime-portability:
    name: Runtime (${{ matrix.target }})
    strategy:
      fail-fast: false
      matrix:
        include:
          - runner: ubuntu-latest
            target: linux
          - runner: windows-latest
            target: windows
    runs-on: ${{ matrix.runner }}

    steps:
      - name: Checkout
        uses: actions/checkout@v6

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Cache Rust build
        uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32

      - name: Compile installed runtime
        run: >-
          cargo check --locked
          -p pv -p cli -p config -p daemon -p platform
          -p protocol -p resources -p self-update -p state

      - name: Build PV
        run: cargo build --locked -p pv

      - name: Test portable domain boundaries
        run: |
          cargo test --locked -p state -p config -p resources -p platform --lib unsupported
          cargo test --locked -p resources --lib target_platform_for
```

Do not include `--workspace`, `--all-targets`, or `pv-release`; Unix-specific macOS tests remain part of the supported macOS job until follow-up test portability work. The focused native tests must run `unsupported_owner_only_directory_does_not_create_path` and `unsupported_permission_write_does_not_create_temporary_file` on Windows.

- [ ] **Step 2: Add portable command smoke tests**

Use `shell: bash` for the smoke step on both native runners (GitHub’s Windows image provides Git Bash):

```yaml
      - name: Smoke portable commands
        shell: bash
        run: |
          set -euo pipefail
          pv_binary=target/debug/pv
          if [[ -x target/debug/pv.exe ]]; then
            pv_binary=target/debug/pv.exe
          fi
          pv_smoke_home=$(mktemp -d)
          export HOME="$pv_smoke_home"
          export USERPROFILE="$pv_smoke_home"
          "$pv_binary" --help >/dev/null
          "$pv_binary" --version >/dev/null
          "$pv_binary" completions bash >/dev/null
          test ! -e "$pv_smoke_home/.pv"
```

These commands must exit zero and must not create `~/.pv`.

- [ ] **Step 3: Add the direct unsupported smoke test**

Add:

```yaml
      - name: Smoke unsupported host capability
        shell: bash
        env:
          EXPECTED_TARGET: ${{ matrix.target }}
        run: |
          set -euo pipefail
          pv_binary=target/debug/pv
          if [[ -x target/debug/pv.exe ]]; then
            pv_binary=target/debug/pv.exe
          fi
          pv_smoke_home=$(mktemp -d)
          export HOME="$pv_smoke_home"
          export USERPROFILE="$pv_smoke_home"
          if output=$("$pv_binary" daemon:enable 2>&1); then
            printf '%s\n' "daemon:enable unexpectedly succeeded" >&2
            exit 1
          fi
          printf '%s\n' "$output" | grep -F \
            "daemon registration is unsupported on ${EXPECTED_TARGET}"
          test ! -e "$pv_smoke_home/.pv"
```

This proves capability-level failure and guards against both Windows owner-only-filesystem errors masking the command and Linux accidentally executing `launchctl`.

- [ ] **Step 4: Validate workflow syntax and local macOS behavior**

Run:

```shell
cargo check --locked -p pv -p cli -p config -p daemon -p platform -p protocol -p resources -p self-update -p state
cargo run --quiet --locked -p pv -- --help >/dev/null
cargo run --quiet --locked -p pv -- --version >/dev/null
cargo run --quiet --locked -p pv -- completions bash >/dev/null
git diff --check
```

Expected: all commands exit zero on macOS. Review `.github/workflows/ci.yml` and confirm there is no upload, release, publication, installer, or `pv-release` step in `runtime-portability`.

- [ ] **Step 5: Push the branch and inspect both native jobs**

After the task commit is pushed through the normal branch workflow, use GitHub Actions to confirm both matrix entries run natively. If either compile fails, fix only runtime-scope target leakage revealed by that compiler, add the exact affected file to this task’s commit, and rerun both jobs. Do not weaken the package list or skip a failing runtime crate.

Expected on completion: `Runtime (linux)` and `Runtime (windows)` both pass compile, build, portable smoke, and typed unsupported smoke steps. No artifacts are uploaded.

- [ ] **Step 6: Commit Task 7**

```shell
git add .github/workflows/ci.yml
git commit -m "ci: compile runtime on linux and windows"
```

---

### Task 8: Full macOS Regression And Scope Audit

**Files:**
- Verify all files changed by Tasks 1-7.
- Do not add implementation unless a verification command identifies a regression within this plan’s scope.

**Interfaces:**
- Proves the supported macOS product remains green.
- Proves the runtime scope has no compile errors or architecture-only Darwin fallback.
- Proves the plan did not expand into release tooling or functional Linux/Windows implementations.

- [ ] **Step 1: Run the repository’s complete supported checks**

Run exactly:

```shell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo shear
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh release/artifacts/recipes/redis/*.sh release/artifacts/recipes/mysql/*.sh release/artifacts/recipes/postgres/*.sh release/artifacts/recipes/mailpit/*.sh release/artifacts/recipes/rustfs/*.sh
cargo nextest run --workspace --all-features --locked
git diff --check
```

Expected: every command exits zero on macOS. No existing snapshot changes are pending.

- [ ] **Step 2: Audit forbidden fallbacks and compile blockers**

Run:

```shell
rg -n 'compile_error!\("PV v1 targets macOS|PV daemon IPC currently supports Unix' crates/state crates/config crates/daemon
rg -n 'fn current_target_platform|cfg!\(target_arch' crates/cli/src crates/daemon/src
rg -n 'tokio::net::UnixStream' crates/daemon/src/client.rs
rg -n 'security-framework|rustix' crates/platform/Cargo.toml crates/state/Cargo.toml crates/daemon/Cargo.toml
git diff --name-only 9601ef1..HEAD
git status --short
```

Expected:

- the first three searches return no matches;
- target-specific dependencies appear only in appropriate target dependency sections;
- changed files are limited to the runtime foundation, `DESIGN.md`, this plan, and CI;
- `Cargo.lock`, release tooling, and publication workflows are unchanged; and
- the worktree is clean after commits.

- [ ] **Step 3: Confirm native CI evidence**

Record the successful GitHub Actions run URL in the pull request or handoff notes. Confirm separately:

- existing macOS `Rust` job passed its full suite;
- `Runtime (linux)` passed;
- `Runtime (windows)` passed; and
- neither runtime job uploaded artifacts.

- [ ] **Step 4: Review against the approved scope**

Confirm this slice does **not** claim completion of the full portable-architecture program. The following remain required follow-up plans:

1. semantic capability facades and broad unsupported `status`/`doctor` observations;
2. `getrandom`, browser, structured process-inspection, and listener spikes;
3. lifecycle boundaries for IPC, containment, update activation, shims, resolver, low ports, trust stores, and daemon registration; and
4. cross-platform verification hardening once those semantic observations exist.

No extra commit is required for Task 8 if all verification passes without changes. If verification exposes an in-scope defect, use a focused Conventional Commit that describes that defect and rerun all affected checks.

---

## Completion Criteria

This implementation plan is complete only when:

1. `DESIGN.md` records macOS-first stabilization and committed subsequent Linux/Windows support.
2. The root application and every runtime crate in scope compile on native macOS, Linux, and Windows runners.
3. Top-level help, version output, and Bash completion generation exit zero on all three runners.
4. `pv daemon:enable` exits nonzero on Linux and Windows with the typed message naming daemon registration and the actual target.
5. Windows filesystem/config stubs return before creating protected paths or temporary files.
6. Linux is not routed through the macOS daemon IPC or host-control implementation merely because it is Unix.
7. No Linux/Windows call site defaults to `DarwinArm64` or `DarwinAmd64` based on architecture alone.
8. Existing macOS behavior, integration tests, snapshots, formatting, Clippy, cargo-shear, and shellcheck remain green.
9. `Cargo.lock`, release tooling, and publication workflows remain unchanged.
10. No Linux or Windows artifact is built for publication, uploaded, or described as supported.
