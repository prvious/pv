# PR 11 pf Port Redirects And Port Commands Design

## Goal

Implement roadmap PR 11, covering `PV-052` and `PV-053`, without pulling in later setup, uninstall, LaunchAgent, Gateway runtime, or privileged system-mutation work.

PR 11 should deliver PV's non-privileged `pf` preparation and status surface. It should generate PV-owned `pf` artifacts, allocate the Gateway high ports those artifacts target, inspect prepared and system `pf` state read-only, and expose useful `pv ports:*` commands. It must not run `sudo`, invoke `pfctl`, or write real `/etc/pf.conf` or `/etc/pf.anchors` files.

## Scope

Roadmap PR 11 covers:

| Package | Purpose |
| --- | --- |
| `PV-052` | Implement `pf` config generation for loopback low-port redirects. |
| `PV-053` | Implement lower-level `pv ports:*` commands for status, install preparation, and uninstall preparation. |

This PR depends on the daemon and port allocator foundations from PR 4 and PR 5. It also follows the PR 10 DNS boundary: prepare and inspect now, leave real privileged mutation for PR 13 setup/system-integration work.

## Decisions

Use a prepared-config approach for PR 11. PV will render the complete PV anchor file and the PV-owned pf.conf anchor reference block under `~/.pv/config/pf/`, then report how those prepared artifacts compare with the system files. Later setup code can install the prepared artifacts with privileges.

Persist Gateway HTTP and HTTPS high ports now. The preferred ports are `48080` for HTTP and `48443` for HTTPS, with fallback through PV's shared high-port range. The Gateway runtime itself remains out of scope; PR 11 only decides and records the ports that future `pf` rules and Gateway startup must agree on.

Perform read-only low-port conflict checks in `ports:install`. If another process is already listening on loopback `80` or `443`, PV should fail before preparing install guidance. Process-name reporting can wait unless it is available through a simple, reliable helper.

IPv4 loopback is required for PR 11. IPv6 redirect generation can wait until PV can validate macOS `pf` IPv6 handling confidently and report any degradation clearly.

## Architecture

The `macos` crate should own `pf` domain logic:

- render PV's anchor file content;
- render the PV-owned `/etc/pf.conf` reference block;
- parse and inspect prepared and system `pf` files;
- classify each file or block as missing, current, stale, conflict, or unreadable.

The `state` crate should own persisted Gateway port assignment. PR 11 needs two distinct Gateway port identities, one for HTTP and one for HTTPS, because the current single `PortOwner::Gateway` identity can only store one row in the existing port-assignment table. The implementation should add narrowly named Gateway HTTP and HTTPS owners or an equivalent structured Gateway component identity, then expose matching port request helpers. Gateway assignments should be reused once present, even if the port is currently bound, matching the DNS behavior fixed in PR 10. New assignments should be created only when absent.

The CLI should add `ports:status`, `ports:install`, and `ports:uninstall`. These commands should use state and macOS helpers, support injected system paths in tests, and avoid privileged mutation.

## Generated Artifacts

PV should prepare both artifacts under `~/.pv/config/pf/`:

| Artifact | Purpose |
| --- | --- |
| prepared anchor file | The complete PV-owned anchor content intended for `/etc/pf.anchors/com.prvious.pv`. |
| prepared pf.conf reference block | The minimal PV-owned block intended for `/etc/pf.conf` so macOS loads PV's anchor. |

The anchor content should redirect loopback TCP port `80` to the stored Gateway HTTP port and loopback TCP port `443` to the stored Gateway HTTPS port. Both the anchor file and pf.conf reference block should carry clear PV ownership markers such as `# Managed by PV`.

Prepared files are disposable generated config. They live under `~/.pv/config/pf/` and may be overwritten by `ports:install` when the expected Gateway ports change or when stale prepared files are repaired.

## Command Behavior

`pv ports:install` should:

1. open `pv.db` from the injected home;
2. reuse existing Gateway HTTP and HTTPS assignments if present;
3. otherwise assign preferred ports `48080` and `48443`, falling back through `45000..=48999`;
4. check loopback `80` and `443` for active listeners before preparing install guidance;
5. render the prepared anchor and pf.conf reference files under `~/.pv/config/pf/`;
6. inspect system `pf` paths read-only;
7. exit non-zero with clear guidance that privileged installation is deferred.

`pv ports:status` should be read-only. It must not allocate ports, create files, remove files, run `pfctl`, or mutate state. It should report the prepared anchor, prepared pf.conf reference, system anchor, and system pf.conf reference independently.

`pv ports:uninstall` should remove only the prepared `pf` artifacts under `~/.pv/config/pf/`. If PV-owned system `pf` files or reference blocks remain, it should exit non-zero and explain that privileged removal is deferred. If non-PV-owned system content exists, it should report the conflict and leave it alone.

These commands should use human output only for PR 11. JSON output can wait for the later `PV-095` JSON outputs package.

## Status States

`ports:status` should report four independent state groups:

- prepared anchor config;
- prepared pf.conf reference block;
- system anchor file;
- system pf.conf reference block.

Prepared states should include missing, current, stale, and unreadable. System states should include missing, current, stale, conflict, and unreadable.

For ownership, PV only treats system content as PV-owned when the ownership marker is present and the content has an expected shape. Missing files are safe. PV-owned but mismatched content is stale. Existing non-PV content is a conflict. Unreadable paths are reported as unreadable and left alone.

## Error Handling

Domain and integration helpers should use typed errors where callers need to distinguish behavior. Avoid parsing user-facing strings to distinguish low-port conflicts, unreadable files, non-PV-owned system content, stale PV-owned system content, and no available Gateway port.

Use `anyhow` only at test and top-level orchestration boundaries. CLI command handlers can convert typed errors into user-facing messages through the existing `ExecuteError` path.

Gateway HTTP and HTTPS port assignment should avoid leaving partial state. PR 11 should add a narrow transactional helper that assigns both Gateway ports as one operation, using the distinct Gateway HTTP and HTTPS identities.

Command output should avoid unsafe manual instructions, including raw `sudo` or `pfctl` snippets. Future-work comments may mark the later privileged mutation hooks for PR 13/setup, but they should name the deferred action precisely.

## Testing

Prefer integration-style tests in touched crates and snapshots for user-facing command output.

State tests should cover:

- Gateway HTTP prefers `48080`;
- Gateway HTTPS prefers `48443`;
- fallback works when preferred ports are unavailable;
- persisted assignments are reused;
- HTTP and HTTPS assignments are stored under distinct structured owners;
- HTTP and HTTPS assignment does not leave partial state when one side cannot be assigned.

macOS tests should cover:

- rendered anchor content snapshot;
- rendered pf.conf reference block snapshot;
- inspection of missing, current, stale, conflict, and unreadable anchor files;
- inspection of missing, current, stale, conflict, and unreadable pf.conf reference state.

CLI integration tests should cover:

- `ports:install` writes prepared artifacts under injected `HOME`, prints deferred privileged-install guidance, and exits non-zero;
- `ports:install` fails on injected low-port conflicts before preparing install guidance;
- `ports:status` reports prepared and system `pf` states without creating files or allocating ports;
- `ports:uninstall` removes only prepared artifacts and reports when real system removal is deferred;
- command output does not include unsafe `sudo` or `pfctl` command snippets.

Low-port conflict tests should use an injectable availability checker or test seam instead of binding real low ports.

Verification should use focused commands first, for example specific `cargo nextest run -E 'test(...)'` invocations and focused `cargo insta test --accept --test-runner nextest -- ...` runs for snapshots. Before opening the PR, run the full workspace tests plus formatting, clippy, dependency, snapshot, and diff hygiene checks.

## Non-Goals

The following are intentionally out of scope for PR 11:

- writing `/etc/pf.conf`;
- writing `/etc/pf.anchors/com.prvious.pv`;
- invoking `pfctl`;
- running `sudo`;
- starting, configuring, or validating the real Gateway runtime;
- implementing `pv setup` or `pv uninstall`;
- registering, starting, stopping, or repairing the LaunchAgent;
- daemon health integration for stale `pf` state;
- whole-system `pv status` or `pv doctor`;
- CA trust or DNS resolver changes;
- JSON output for `pv ports:*`.
