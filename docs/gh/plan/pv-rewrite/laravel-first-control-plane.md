# Feature PRD: Laravel-First Local Control Plane

## Problem

Laravel developers need a native local development environment that handles
HTTPS `.test` domains, PHP versions, Composer, databases, Redis, mail capture,
object storage, local DNS, certificates, and long-running processes without
Docker, VMs, hidden magic, or brittle manual setup.

The prototype proved the product direction, but its architecture grew around
command workflows and service-specific implementation details. There are too
many partial sources of truth: project config, registry JSON, state JSON, config
paths, service packages, command wrappers, setup automation, daemon manager,
watcher, and supervisor.

## Solution

Rewrite `pv` as a Laravel-first desired-state control plane.

A Laravel developer installs `pv`, initializes or links a Laravel project,
reviews an explicit `pv.yml`, and gets a complete local environment at
`https://<app>.test` with managed PHP, Composer support, HTTPS certificates,
DNS, and declared backing services.

User commands describe what should exist. The daemon reconciles the machine
toward that desired state and records observed status when reality does not
match.

## Product Promise

A Laravel developer can install `pv`, link a Laravel app, and get a complete
local development environment that stays running, heals itself, and is easy to
understand.

## MVP Scope

- Active Go rewrite module at repository root.
- Legacy prototype isolated under `legacy/prototype`.
- Explicit `pv.yml` project contract.
- Managed PHP runtime and Composer support.
- Desired-state store and observed-status model.
- Controllers for resources.
- Resource-agnostic supervisor and daemon reconcile loop.
- Gateway `.test` HTTPS routing.
- Managed Postgres, MySQL, Redis, Mailpit, and RustFS resources.
- Project env rendering with managed labels and no hidden inference.
- Setup commands declared in `pv.yml`.
- Scriptable status output and clear next actions.
- Laravel helper commands for Artisan, database, mail, and S3 workflows.
- Explicit post-MVP backlog.

## Out Of Scope For MVP

- Laravel Octane.
- Generic PHP and static-site polish unless incidental.
- Docker or VM orchestration.
- Hidden service/env setup based on `.env` hints.
- Automatic cross-line data upgrades.
- Database dump/import tooling.
- Bucket migration tooling.
- Worker, queue, and scheduler supervision.
- Per-project PHP extension management.
- Per-project Xdebug management.
- Custom Caddy snippets.
- LAN sharing and mobile device access.
- Generic command runner beyond declared setup commands.
- Backward-compatible migration from prototype state layouts.
- Expensive artifact workflows unless explicitly requested.
- Rewriting the primary product in Rust, Zig, Node, or Bun.
- Heavy TUI-first setup.

## Success Criteria

- A new Laravel project can be initialized with a reviewable `pv.yml`.
- `pv link` records project desired state and does not infer services from
  `.env`.
- The daemon reconciles requested runtimes, tools, gateway routes, and backing
  services.
- `pv status` explains desired state, observed state, failures, logs, and next
  actions.
- Composer and Artisan run through the pinned PHP runtime.
- Declared services expose correct Laravel env values.
- The supervisor remains resource-agnostic.
- The store is the only mutable machine-owned authority.
- All MVP scope has issue coverage, implementation plans, and test strategies.
- The rewrite stack has a dedicated E2E validation epic with a required hermetic
  release gate.

## Primary Users

- Laravel developer: wants a native local environment with little ceremony.
- Maintainer: wants a testable architecture with clear ownership.
- Automation user: wants clean stdout, useful exit codes, and predictable state.

## Key Architecture Rule

```text
Commands do not do work. They request state changes.
Controllers do work.
The supervisor only runs processes.
The store is the authority.
Resources expose capabilities, not fake sameness.
Laravel is the primary product path.
```
