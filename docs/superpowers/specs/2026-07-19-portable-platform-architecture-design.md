# Portable Platform Architecture Design

## Summary

PV will stabilize its current macOS application before implementing Linux and Windows product support, but Linux and Windows are committed subsequent platforms rather than hypothetical possibilities. The codebase will therefore adopt portable architectural boundaries now while preserving macOS behavior and release priority.

The installed PV application and its runtime crates will compile natively on macOS, Linux, and Windows. macOS remains the only supported and published platform during this phase. Linux and Windows builds will provide real portable commands and explicit typed `Unsupported` behavior for system capabilities that have not been implemented yet.

The implementation will use compile-time-selected operating-system modules behind small semantic APIs. It will not introduce one giant injected platform trait, split every operating system into a separate crate, or attempt to implement Linux and Windows system integration during macOS stabilization.

## Product Direction

The implementation must update `DESIGN.md` to record these decisions:

- PV v1 targets macOS 13 and newer.
- Stabilizing the macOS application remains the immediate product priority.
- Linux and Windows are committed subsequent platforms.
- New system boundaries must avoid unnecessary portability blockers.
- Compile support and explicit unsupported behavior do not constitute product support or authorize publishing Linux or Windows binaries.

This replaces the current statement that Linux and Windows support is not guaranteed. It does not add Linux or Windows to v1.

## Goals

- Preserve current macOS behavior while reorganizing system interaction behind portable semantic boundaries.
- Compile the installed application and runtime crates natively on macOS, Linux, and Windows.
- Let portable commands run on all three platforms.
- Return typed, capability-specific unsupported results for unfinished Linux and Windows system behavior.
- Keep operating-system mechanisms out of CLI and daemon orchestration.
- Replace brittle host interrogation and handoff mechanisms where a maintained Rust or native abstraction improves current macOS correctness and future portability.
- Establish native CI compile and smoke gates for the runtime application on all three platforms.
- Keep changes small enough to review, verify, and revert independently.

## Non-Goals

- Do not implement or advertise functional Linux or Windows product support in this phase.
- Do not publish Linux or Windows application artifacts.
- Do not make Linux or Windows part of PV v1.
- Do not port `pv-release`, artifact recipes, or maintainer-side release tooling as part of the runtime portability gate.
- Do not choose Linux resolver/service baselines or the Windows daemon/service model yet.
- Do not create a universal firewall, resolver, service-manager, trust-store, process, or filesystem implementation.
- Do not replace intentional managed child processes or supported operating-system control tools merely to reduce subprocess count.
- Do not introduce one large `Platform` trait or pass a platform trait object through the application.
- Do not create platform-specific crates unless the existing dependency graph proves that module-level isolation is insufficient.
- Do not combine the migration into one repository-wide rewrite.

## Runtime Workspace Scope

The portability gate covers the installed `pv` application and the runtime crates it depends on:

- root `pv` package
- `cli`
- `config`
- `daemon`
- `platform`
- `protocol`
- `resources`
- `self-update`
- `state`

`pv-release`, release recipes, artifact build hooks, signing tools, installer generation, and other maintainer-side release behavior are excluded. They may legitimately remain target-specific and require a separate portability design.

## Architecture

### Compile-Time Platform Facade

The application will use a compile-time facade with private per-operating-system implementations:

```text
cli / daemon / other orchestration
                |
                v
      semantic capability API
        |        |        |
        v        v        v
      macOS    Linux    Windows
```

Shared callers depend on semantic capabilities. Compile-time target selection routes each call to the matching implementation. There is no runtime operating-system switch and no dynamic dispatch requirement.

The facade should remain composed of focused modules such as:

- browser handoff
- process inspection
- listener inspection
- process containment
- daemon registration
- resolver integration
- low-port frontend
- trust-store integration
- owner-only filesystem behavior

The exact Rust module layout may follow the nearest existing module style, but these constraints apply:

- public types and operations describe product capabilities rather than macOS mechanisms;
- operating-system commands, paths, configuration formats, native APIs, and evidence stay in private target implementations;
- target-specific dependencies are gated in the owning crate's `Cargo.toml` target sections; and
- application orchestration must not branch on PF, launchd, systemd, SCM, Keychain, CryptoAPI, named pipes, or similar mechanisms.

### No Giant Platform Trait

PV will not introduce an `Arc<dyn Platform>` or equivalent application-wide interface. Browser opening, process inspection, DNS integration, trust stores, daemon registration, and filesystem behavior have different callers, data, and lifecycle semantics. Combining them would create an oversized interface and an invasive dependency-injection refactor.

Existing focused test seams remain valid. A capability may use a narrow injected runner or inspector where tests require it.

### No Premature Platform Crates

PV will keep the current workspace structure initially. Private target modules provide sufficient compile-time isolation without creating `platform-macos`, `platform-linux`, and `platform-windows` crates. A new crate requires a concrete dependency-cycle or dependency-isolation need documented by the implementation plan.

## Capability And Error Model

### Semantic Public APIs

Application crates ask for product operations and observations, for example:

- inspect or repair daemon registration;
- inspect or install the low-port frontend;
- inspect resolver integration;
- inspect or mutate trust-store state;
- inspect a process identity;
- inspect local listeners; or
- open a URL for an interactive user.

They do not construct or interpret PF anchors, LaunchAgent plists, systemd units, Windows services, Keychain records, CryptoAPI stores, or named-pipe ACLs.

### Typed Unsupported Operations

An unavailable operation returns a typed error containing both the semantic capability and current target. It must not return success, a fabricated empty result, or a generic string that callers must parse.

Direct invocation of an unsupported system operation exits nonzero and identifies the unavailable capability. Unsupported behavior must be deterministic and testable.

### Diagnostic Observations

Read-only diagnostic capabilities can return an explicit unsupported observation so broad commands continue collecting other results. `pv status` and `pv doctor` must report required unsupported capabilities and exit nonzero rather than crash or describe the system as healthy.

Each capability owns the states meaningful to it. Modules may share vocabulary such as `Current`, `Missing`, `Stale`, `Conflict`, `Unreadable`, and `Unsupported`, but the design does not force unrelated subsystems into one generic state enum.

Operating-system evidence may be rendered in diagnostics without becoming orchestration policy. For example, a shared daemon-registration result may be stale while its private evidence describes a LaunchAgent plist, systemd unit, or Windows service mismatch.

## Code Ownership And Dependency Rules

The `platform` crate owns host integration, but it is not a dumping ground for every target-dependent primitive.

### `platform`

Owns:

- host process identity and containment primitives;
- listener and port-owner inspection;
- interactive desktop URL handoff;
- daemon registration mechanisms;
- host resolver integration;
- low-port frontend integration;
- trust-store inspection and mutation; and
- platform-specific host paths and configuration evidence needed by those capabilities.

### `state`

Owns its portable filesystem contract and private target implementations for:

- owner-only directories and files;
- permissions or access-control behavior;
- atomic file replacement; and
- file locking.

`state` must not depend on `platform`, because `platform` already depends on state paths and helpers. Target-specific filesystem mechanics may remain inside private `state` modules when they implement state's own policy rather than host integration.

### `daemon`

Owns daemon protocol-serving lifecycle and transport orchestration. Its local IPC boundary may select Unix sockets or Windows named pipes behind shared transport operations. Process ownership and containment primitives should be obtained from focused platform capabilities rather than embedded host interrogation commands.

### `self-update`

Owns target-specific application activation, executable replacement, rollback, and running-executable constraints. The current Unix symlink model must not be presented as the universal update contract.

### `resources`

Owns target naming, artifact format selection, executable layout, and resource-package validation. Runtime portability does not require release recipes to become portable in the same phase.

### `cli`

Owns interaction and orchestration only. It may decide how to explain an unsupported capability, but it must not know the operating-system mechanism underneath it.

### Conditional Compilation Rule

Target conditionals belong in narrow implementation modules owned by the relevant low-level crate. Application workflows should not contain target-specific policy branches. Remaining conditionals outside `platform` must implement a crate's local domain policy and must not leak host mechanisms to callers.

## Initial Linux And Windows Behavior

Linux and Windows application builds are compile and architecture validation targets, not supported distributions.

The following portable surfaces must work where their existing command shape applies:

- top-level help;
- version reporting; and
- shell-completion generation.

Pure shared parsing and protocol code must remain usable by their callers. Platform-dependent commands route through their capability boundary and return a typed unsupported result until implemented.

PV must not add a blanket non-macOS startup failure. Capability-level behavior proves that the application boundary is granular and allows portable commands to work. No Linux or Windows artifacts are published or promoted during this phase.

## Migration Sequence

### Phase 1: Product And Compile Foundation

- Update `DESIGN.md` with the committed platform direction and current support boundary.
- Introduce private macOS, Linux, and Windows implementation selection behind semantic APIs.
- Move current macOS behavior behind those APIs without changing observable macOS behavior.
- Add explicit Linux and Windows unsupported implementations.
- Remove or replace runtime compile errors and Unix-only public types that prevent the scoped application crates from compiling natively.
- Add the initial native Linux and Windows compile/smoke CI gates.

Phase 1 must not opportunistically implement Linux or Windows host integration.

### Phase 2: Host Helper Cleanup

Land each cleanup independently:

1. Replace MySQL and Postgres `/dev/urandom` reads with the existing `getrandom` dependency.
2. Replace the macOS-only browser handoff with a cross-platform browser capability, with `webbrowser` as the current preferred candidate.
3. Replace `/bin/ps` rendering and reparsing with structured process inspection, with `sysinfo` as the current preferred spike candidate.
4. Spike `listeners` against PV's required listener matrix and replace both `netstat-esr` and `/usr/sbin/netstat` only if the matrix passes.

A preferred candidate is not pre-approved regardless of evidence. Each spike has an explicit rejection path and must not leave both old and new production mechanisms active indefinitely.

### Phase 3: Lifecycle Boundary Isolation

Introduce or tighten semantic boundaries around:

- local daemon IPC;
- process containment and child-tree termination;
- application update activation and rollback;
- PHP and Composer command shims;
- daemon registration;
- resolver integration;
- low-port frontend integration; and
- trust stores.

The macOS implementations continue using the current supported mechanisms. Linux and Windows return typed unsupported results. `interprocess`, `process-wrap`, and similar libraries remain spike candidates rather than architectural requirements until PV's lifecycle tests prove they fit.

### Phase 4: Cross-Platform Verification Hardening

- Make native runtime compile checks required on macOS, Linux, and Windows.
- Keep the full supported behavior suite on macOS.
- Run portable-command smoke tests on Linux and Windows.
- Test direct unsupported command behavior and broad diagnostic collection.
- Audit orchestration crates for leaked target-specific mechanisms.

These phases define dependency and review order. They should be split into independently reviewable commits or pull requests rather than implemented as one patch.

## Candidate Acceptance Criteria

### `getrandom`

- Preserve credential length, encoding, and storage behavior.
- Remove direct random-device file access.
- Use the dependency already present in the daemon workspace package.

### `webbrowser`

- Preserve current HTTP and HTTPS URL behavior.
- Keep browser opening in the interactive CLI path rather than the daemon.
- Cover injected success and failure behavior.
- Complete one manual macOS desktop smoke test before removing the existing launcher.

### `sysinfo`

- Use only the feature set required for process inspection.
- Prefer the kernel-reported executable path over `cmd[0]`.
- Treat ordered structured arguments as supporting identity evidence, not sole proof.
- Store a separate operating-system-reported process-start token captured through the same inspector used during adoption.
- Do not treat the current human-readable runtime metadata `started_at` field as a process identity token.
- Cover executable symlinks, arguments containing spaces, ordered arguments, fast exits, permission failures, PID mismatch, and process-start-token mismatch.
- Preserve PV's ownership policy, readiness behavior, graceful timeout, restart backoff, and adoption decisions outside the dependency.

### `listeners`

- Detect IPv4 loopback, IPv6 loopback, IPv4 wildcard, and IPv6 wildcard listeners relevant to PV.
- Report TCP listen state and the owning process evidence when the operating system makes it available.
- Define deterministic behavior for permission limitations and processes that exit during inspection.
- Pass fixtures on supported macOS versions and both supported macOS architectures before replacing production inspection.
- Remove both `netstat-esr` and `/usr/sbin/netstat` together after acceptance; do not add a third permanent source.

### Lifecycle Candidates

`process-wrap`, `interprocess`, standard-library file locks, and other portability candidates must preserve PV's product semantics rather than dictate them. Their spikes must evaluate:

- child survival versus termination when the daemon exits;
- safe process-tree termination;
- ownership verification and adoption;
- graceful shutdown followed by bounded force termination;
- endpoint owner access and peer identity;
- stale endpoint cleanup;
- NDJSON protocol preservation;
- update rollback; and
- current macOS behavior.

Crate adoption is deferred if the required behavior depends on Linux or Windows product decisions intentionally outside this phase.

## Verification Strategy

### macOS Behavior Gate

macOS remains the supported behavior gate. Every migration slice must:

- run the relevant existing integration and snapshot tests before and after the change;
- preserve snapshots unless a user-facing change is separately approved;
- prefer integration tests and nearby `insta` snapshot patterns;
- run formatting and Clippy using the commands in `CONTRIBUTING.md`;
- run `cargo shear` when dependency declarations change; and
- run `git diff --check` before completion.

### Linux And Windows Compile/Smoke Gate

Native Linux and Windows CI runners must:

- compile the scoped installed application and runtime crates;
- run top-level help, version, and completion-generation smoke tests;
- invoke at least one platform-dependent command and assert a typed unsupported error with a nonzero exit; and
- run a diagnostic smoke test showing that unsupported required capabilities are reported without a panic and result in a nonzero exit.

These jobs do not publish artifacts and are not evidence of supported product behavior.

### Target-Specific Test Location

Pure semantic behavior should be tested once in shared modules. Operating-system mechanisms and evidence parsing should be tested in their target implementation. Integration tests should exercise the public capability or command boundary so callers cannot accidentally depend on private mechanism types.

## Completion Criteria

The portable-architecture program is complete when all of the following are true:

1. `DESIGN.md` records macOS stabilization first and committed Linux and Windows support afterward.
2. The installed application and runtime crates compile natively on macOS, Linux, and Windows.
3. Help, version, and completion generation work on all three targets.
4. Unimplemented capabilities fail explicitly through typed results.
5. Broad diagnostics report required unsupported capabilities and exit nonzero without crashing.
6. Existing supported macOS behavior and its test suite remain green.
7. Operating-system mechanisms no longer leak through application orchestration APIs.
8. Host-helper replacements meet their fixture acceptance criteria before old production paths are removed.
9. Linux and Windows artifacts are neither published nor advertised as supported.
10. The work lands in independently reviewable and revertible slices.

## Deferred Platform Implementation Decisions

The following decisions are deliberately deferred until PV begins functional Linux and Windows implementation after macOS stabilization:

- supported Linux distributions and whether the first baseline requires systemd and systemd-resolved;
- the Windows daemon model: SCM service, per-user process, or another startup mechanism;
- Windows DNS port and elevation behavior;
- Linux and Windows trust-store support boundaries;
- Windows application activation and rollback mechanics;
- Linux and Windows artifact formats;
- PHP and Composer launcher behavior across application updates; and
- whether child processes survive daemon failure or terminate with the daemon on each platform.

These decisions do not block the compile-time facade, typed unsupported behavior, macOS implementation isolation, or native compile/smoke gates defined here.
