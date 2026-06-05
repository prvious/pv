# PR 13 System Integration Design

## Summary

PR 13 turns the prepared macOS integration work from PRs 10-12 into real foreground system mutation. It adds LaunchAgent lifecycle management, `pv setup`, safe `pv uninstall`, and System keychain trust/untrust for PV's local CA. The work stays behind the existing `platform` boundary so application crates do not directly own macOS APIs, shell commands, or privileged filesystem paths.

## Goals

- Replace daemon lifecycle stubs with LaunchAgent-backed `pv daemon:enable`, `pv daemon:disable`, and `pv daemon:restart`.
- Replace DNS, `pf`, and CA trust deferrals with safe foreground mutation for PV-owned system integrations.
- Add `pv setup [--yes] [--non-interactive] [--no-path]`.
- Add `pv uninstall [--prune] [--force]`.
- Preserve data by default during uninstall and require explicit force for destructive prune behavior.
- Extend the daemon client so setup can wait for foreground reconciliation job completion.
- Keep tests fully injected; tests must not mutate real `/etc`, `pf`, LaunchAgents, or the System keychain.

## Non-Goals

- Do not implement real Gateway, PHP, FrankenPHP, Composer, or backing Managed Resource adapters.
- Do not add Linux or Windows behavior.
- Do not create takeover flows for non-PV-owned resolver, `pf`, LaunchAgent, or keychain state.
- Do not change public command namespace style.
- Do not silently edit shell profiles outside PV's `PV ENV` block.

## Architecture

The implementation uses the existing split:

- `platform` owns host integration rendering, inspection, and mutation helpers.
- `cli` owns command orchestration, confirmation behavior, user output, and daemon client calls.
- `daemon` owns daemon process runtime and foreground reconciliation event streaming.
- `state` owns `~/.pv` paths, layout, database, and file permission helpers.
- `protocol` owns daemon wire events used by the extended client.

Privileged/system mutation is exposed through typed platform traits or small operation APIs so tests can assert behavior without running real system commands. Foreground commands perform mutation only after inspecting current state and only when the target is missing, stale, or PV-owned.

## Platform Mutations

DNS installs or repairs `/etc/resolver/test` from the prepared resolver config. It refuses non-PV-owned files. Uninstall removes only PV-owned resolver config.

`pf` installs or repairs PV's anchor and `pf.conf` reference. It refuses non-PV-owned anchor/reference conflicts and does not disable `pf` globally during uninstall.

CA trust creates or repairs local CA files, then installs trust for the current PV CA into the macOS System/Admin trust domain. Untrust removes PV CA trust from the System keychain while preserving local CA files. If the current trust is already absent, untrust succeeds.

LaunchAgent management renders `~/Library/LaunchAgents/com.prvious.pv.daemon.plist`, verifies PV ownership, loads/starts the daemon, and unloads/removes only PV-owned registration. The plist points to the current executable with `daemon:run`, enables `KeepAlive`, and writes stdout/stderr to PV-owned log files.

## Setup

`pv setup` performs the required bootstrap steps in order:

1. Ensure `~/.pv` layout and permissions.
2. Optionally install or repair the `PV ENV` shell profile block unless `--no-path` is used.
3. Install or repair DNS resolver config.
4. Install or repair `pf` redirects.
5. Ensure and trust the local CA.
6. Register and start the LaunchAgent-backed daemon.
7. Submit `reconcile system` and wait for `job_completed` or `job_failed`.

Because real Managed Resource adapters are deferred to later PRs, PR 13's setup waits on the existing system reconciliation scope but does not pretend default resource artifact installation is complete.

`--yes` accepts PV-owned confirmations. `--non-interactive` accepts PV-owned confirmations but fails when a prompt or macOS authentication would be required. `--no-path` skips shell profile mutation and prints manual shell integration guidance.

## Uninstall

`pv uninstall` stops/unregisters the LaunchAgent, removes PV-owned DNS and `pf` integrations, removes CA trust, removes the PV shell profile block when present, removes runtime/generated files, and preserves logs, `pv.db`, certificates, Composer home/cache, Managed Resource data, and Project `.env` blocks.

`pv uninstall --prune --force` also removes PV-owned state under `~/.pv` after preserving shell profile backups. `--prune` without `--force` requires an interactive confirmation and fails in non-interactive contexts.

## Testing

Tests should follow nearby `cli` and `platform` snapshot patterns:

- Platform tests cover LaunchAgent plist rendering/inspection and system operation planning.
- CLI tests cover daemon lifecycle, DNS/ports/CA mutation success and conflict paths, setup orchestration, uninstall preservation, prune confirmation, and help/completions.
- Daemon tests cover streaming client wait behavior for `job_completed` and `job_failed`.
- Integration tests under `it/` cover public command routing and help text.

Focused verification starts with affected crate tests, then formatting, diff checks, clippy, full workspace tests, and `cargo shear`.
