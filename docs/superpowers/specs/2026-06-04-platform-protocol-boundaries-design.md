# Platform And Protocol Boundaries Design

## Summary

PV v1 remains macOS-only, but application crates should no longer depend directly on macOS-specific host integration APIs. This refactor introduces two workspace boundaries:

- `protocol`: shared daemon wire types and protocol constants.
- `platform`: host OS integration APIs, with macOS as the only concrete v1 implementation.

The goal is to improve dependency direction while the workspace is still small. This is not a Linux or Windows implementation project.

## Goals

- Move daemon request, response, event, and protocol-version types out of `daemon` into a new `protocol` crate.
- Move the existing `macos` crate code into a new `platform` crate and delete `crates/macos`.
- Update app-facing crates, especially `cli`, to depend on `platform` instead of `macos`.
- Keep v1 behavior macOS-only and preserve the existing macOS setup, DNS, `pf`, CA, and LaunchAgent semantics.
- Update `DESIGN.md` and `IMPLEMENTATION.md` so the crate boundary is an explicit project decision.
- Define criteria for what belongs in `platform` versus what may stay in domain crates.

## Non-Goals

- Do not implement Linux or Windows support.
- Do not add machine-global owner metadata or takeover flows.
- Do not redesign DNS, `pf`, CA trust, LaunchAgent, daemon IPC, state, or artifact installation behavior.
- Do not move every `#[cfg(unix)]` block into `platform`.
- Do not rename public CLI commands or change user-facing command behavior.

## Architecture

The intended workspace shape is:

```text
crates/
  cli/
  config/
  daemon/
  platform/
  protocol/
  resources/
  state/
```

`protocol` owns only shared daemon communication contracts:

- protocol version constant
- request and command types
- response status and response envelope types
- job/progress event types
- lightweight framing/serialization helpers if they do not pull in daemon runtime behavior

`protocol` must not depend on `cli`, `daemon`, `platform`, `state`, `config`, or `resources`.

`platform` owns host OS integration:

- macOS resolver file semantics for `/etc/resolver/test`
- macOS `pf` anchor/reference rendering and inspection
- macOS CA file generation and System keychain inspection/trust behavior
- macOS LaunchAgent rendering, inspection, install, uninstall, and lifecycle integration
- host-level conflict detection for privileged integrations
- platform-specific constants for host integration paths

For this refactor, the existing `macos` implementation moves into `platform`. The old `crates/macos` workspace member is removed.

## Dependency Direction

Target dependency direction:

```text
cli
  -> protocol
  -> platform
  -> daemon client API only where needed
  -> state
  -> config

daemon
  -> protocol
  -> state
  -> config
  -> resources
  -> platform only if read-only host integration inspection is needed

platform
  -> state only where host integration needs existing path/state types

state, config, resources
  -> no platform dependency unless a later design decision proves it necessary
```

`cli` should not import macOS-specific implementation modules directly. `daemon` should not own privileged host integration mutation.

## Platform Boundary Criteria

Code belongs in `platform` when it decides, inspects, mutates, models, or names host OS integration.

Examples that belong in `platform`:

- `/etc/resolver/test` rendering, parsing, inspection, install, and uninstall helpers
- `pf` anchor and `/etc/pf.conf` reference rendering, parsing, inspection, install, and uninstall helpers
- System keychain trust inspection and mutation
- LaunchAgent plist rendering, ownership checks, install, uninstall, start, and stop helpers
- host OS constants such as privileged resolver, `pf`, keychain, and LaunchAgent paths
- future Linux or Windows host integration modules, if those become real project scope

Code may stay in its domain crate when OS-specific primitives are local mechanics for that crate's own responsibility.

Examples that may stay outside `platform`:

- `state` Unix permission and UID checks for PV-owned state files
- `resources` Unix symlink and file mode mechanics for artifact installs
- `config` Unix file mode mechanics for atomic `.env` writes
- `daemon` Unix socket IPC and Unix signal handling while v1 remains Unix/macOS-only
- tests guarded by `#[cfg(test)]`

Acceptance rule:

After the refactor, no crate except `platform` may import macOS host integration APIs or contain `target_os = "macos"` host integration logic. Remaining `#[cfg(unix)]` or `#[cfg(not(unix))]` usage outside `platform` must be reviewed and justified as local domain mechanics, not host integration policy.

## Unsupported Platforms

PV v1 remains macOS-only. The refactor may preserve compile errors for non-Unix or non-macOS cases where the current code already intentionally rejects unsupported platforms.

If an unsupported implementation is added, it must return clear typed errors and must not imply Linux or Windows support exists.

## Documentation Changes

`DESIGN.md` should say:

- v1 is still macOS-only.
- PV uses a host platform boundary so app crates do not depend directly on macOS implementation APIs.
- The only concrete v1 platform implementation is macOS.

`IMPLEMENTATION.md` should replace the old baseline `macos` crate shape with `platform` and `protocol`, and should record the boundary criteria above in the roadmap or crate ownership section.

## Testing And Verification

The implementation plan should include focused tests and checks:

- existing macOS integration snapshots continue to pass after moving code into `platform`
- daemon protocol serialization tests pass after extracting `protocol`
- CLI DNS, ports, CA, daemon, and setup-related tests pass against `platform`
- direct `macos::` imports are gone outside migrated platform tests or compatibility aliases, if any
- `cargo fmt --all -- --check`
- focused `cargo nextest` runs for affected crates/tests
- `git diff --check`

## Open Decisions

None for this refactor. Machine-global owner metadata is intentionally deferred.
