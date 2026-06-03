# PR 10 DNS Resolver And Resolver Config Design

## Goal

Implement roadmap PR 10, covering `PV-050` and `PV-051`, without pulling in later LaunchAgent, setup, or privileged system-mutation work.

PR 10 should deliver the internal `.test` DNS resolver and useful `pv dns:*` command surfaces. It should prepare and inspect resolver configuration, but it must not run `sudo`, register or start a LaunchAgent, or mutate the real `/etc/resolver/test` file.

## Scope

Roadmap PR 10 covers:

| Package | Purpose |
| --- | --- |
| `PV-050` | Implement the internal DNS resolver. |
| `PV-051` | Implement `/etc/resolver/test` inspection and lower-level `pv dns:*` commands. |

This PR depends on the existing daemon foundation and port allocator from PR 4 and PR 5. The existing `PortOwner::Dns` / `PortRequest::dns` state model should remain the source of truth for the chosen DNS port.

## Decisions

Use `hickory-proto` for DNS wire parsing and response encoding. Do not use `hickory-server` for this slice; its higher-level server and resolver machinery is broader than PV's narrow internal `.test` resolver needs.

Leave LaunchAgent registration, daemon-start enforcement, `pv setup`, and real foreground `sudo` mutation for PR 13 and related system-integration work. In PR 10, command output should be honest that privileged installation or removal is deferred.

Read the real `/etc/resolver/test` file read-only for `pv dns:status`. Status should be useful immediately, but it must not create, repair, remove, or rewrite system resolver files.

## Architecture

The daemon should gain a small DNS subsystem. On daemon startup, it opens state, allocates or reuses the DNS resolver port, and starts both UDP and TCP loopback listeners. The preferred port is `35353`; fallback candidates come from the shared high-port range already used by PV runtime services. If no candidate is available, daemon startup should fail with a typed daemon error.

The resolver should live inside the daemon process as an internal task. It should not be a child process, Managed Resource, or external resolver. Shutdown should stop accepting DNS traffic along with the existing daemon shutdown path.

The `state` crate should continue owning the persisted port assignment. If a helper path is needed for the prepared resolver config, it should be added to `PvPaths`, for example `~/.pv/config/resolver/test`.

The `macos` crate should own resolver-file content rules and read-only inspection of `/etc/resolver/test`. That keeps macOS integration details out of the CLI and leaves a clear seam for later privileged mutation. The implementation should support injected resolver paths in tests so tests never touch the real `/etc`.

The CLI should add `dns:status`, `dns:install`, and `dns:uninstall` routing. These commands should use the state and macOS helpers but should not start the daemon or run privileged commands.

## Resolver Behavior

For each DNS request, preserve the transaction ID, copy the query section, set response mode, mark the response authoritative, and use a TTL of 5 seconds for answers.

Supported answers:

| Query | Response |
| --- | --- |
| `.test` `A` | `NOERROR` with one `127.0.0.1` answer. |
| `.test` `AAAA` | `NOERROR` with one `::1` answer. |
| `.test` other record type | `NOERROR` with no answers. |
| Non-`.test` any record type | `NOERROR` with no answers. |

The resolver must not proxy non-`.test` queries upstream. Malformed UDP or TCP DNS messages should not crash the resolver task. They may be dropped and logged.

TCP handling should use standard DNS-over-TCP framing: a two-byte length prefix on input and output. UDP responses are tiny and should fit comfortably in normal DNS packet sizes.

## Command Behavior

`pv dns:install` should allocate or reuse the DNS port and write PV's prepared resolver config under `~/.pv/config/resolver/test`. The prepared file should include a PV ownership marker such as `# Managed by PV`, `nameserver 127.0.0.1`, and `port <chosen-port>`. It should also make clear that this is the prepared source for `/etc/resolver/test`.

Because real privileged installation is deferred, `pv dns:install` should exit non-zero after preparing the config and print clear guidance that installing into `/etc/resolver/test` will be handled by later setup/system-integration work.

`pv dns:status` should be read-only. It should report PV's prepared resolver config state and the real `/etc/resolver/test` state separately. System resolver states should include:

- missing;
- PV-owned and current;
- PV-owned but stale;
- present but not PV-owned;
- unreadable.

`pv dns:status` must not allocate a port, create the prepared config, start the daemon, run `sudo`, or mutate any file.

`pv dns:uninstall` should remove only PV's prepared resolver config under `~/.pv/config/resolver/test`. If a real PV-owned `/etc/resolver/test` still exists, it should exit non-zero and explain that privileged removal is deferred. If a non-PV-owned system resolver file exists, it should report the conflict and leave it alone.

These commands should use human output only for PR 10. JSON output can wait for the later `PV-095` JSON outputs package.

## Error Handling

Domain and integration helpers should use typed errors. Avoid string parsing to distinguish cases such as malformed DNS packets, no available DNS port, unreadable resolver file, non-PV-owned resolver conflict, and stale PV-owned resolver config.

Use `anyhow` only at test or top-level orchestration boundaries. CLI command handlers can convert typed errors into user-facing messages through the existing `ExecuteError` path.

Daemon startup should fail if the DNS listeners cannot be started on any candidate port. Individual malformed DNS requests should be handled per-request without taking down the daemon.

## Testing

Prefer integration-style tests in the touched crates and snapshots for user-facing command output.

Daemon tests should cover:

- UDP `.test` `A` returns `127.0.0.1` with TTL 5;
- UDP `.test` `AAAA` returns `::1` with TTL 5;
- UDP `.test` unsupported record type returns `NOERROR` with no answers;
- non-`.test` returns `NOERROR` with no answers;
- malformed UDP input does not crash the daemon;
- TCP DNS framing works for a `.test` `A` query;
- preferred-port collision falls back to a high port and persists the DNS assignment.

CLI and macOS integration tests should cover:

- `dns:install` writes the prepared resolver config under injected `HOME`, prints deferred privileged-install guidance, and exits non-zero;
- `dns:status` reports missing prepared config and missing system resolver without creating anything;
- `dns:status` reports prepared config plus system missing;
- `dns:status` reports PV-owned current, PV-owned stale, non-PV-owned conflict, and unreadable system resolver file through injected resolver paths;
- `dns:uninstall` removes only the prepared config and reports when real system resolver removal is deferred.

State tests should add focused coverage that the DNS port owner persists and reuses preferred and fallback assignments if nearby snapshots do not already prove that behavior.

Verification should use focused commands first, for example specific `cargo nextest run -E 'test(...)'` invocations and focused `cargo insta test --accept --test-runner nextest -- ...` runs for snapshots. Do not update all dependencies in the lockfile; if a lockfile update is needed for `hickory-proto`, keep it limited to that dependency set.

## Non-Goals

The following are intentionally out of scope for PR 10:

- registering, starting, stopping, or repairing the LaunchAgent;
- implementing `pv setup` or `pv uninstall`;
- running `sudo`;
- mutating the real `/etc/resolver/test`;
- managing `pf` rules or CA trust;
- resolving Gateway or Project routing decisions;
- proxying or recursively resolving non-`.test` DNS queries;
- JSON output for `pv dns:*`.
