# Post-MVP Backlog

This backlog records capabilities deliberately excluded from the rewrite MVP. These entries are visible product scope, not hidden implementation obligations.

| Capability | Deferral reason | Reconsideration trigger |
| --- | --- | --- |
| Laravel Octane | MVP focuses on standard Laravel HTTP serving through FrankenPHP gateway behavior. | At least two active projects need Octane-specific local behavior after standard gateway support is stable. |
| Generic PHP polish | Laravel is the primary product path for the rewrite. | Laravel MVP ships and generic PHP users report missing workflow parity. |
| Static-site polish | Static sites do not drive the Laravel-first control-plane architecture. | A maintained static workflow needs pv-managed HTTPS plus setup semantics. |
| Docker or VM orchestration | The rewrite is native local orchestration, not container orchestration. | Product direction changes to support isolated container development environments. |
| Hidden service/env setup from `.env` hints | Hidden inference caused prototype drift and `.env` clobbering. | Do not reconsider for MVP; any future helper must remain explicit and reviewable. |
| Automatic cross-line data upgrades | The MVP does not promise data migration across service major lines. | A supported upgrade story is planned for a specific resource family. |
| Database dump/import tooling | MVP database scope is install, run, status, env, and explicit create/drop/list commands. | Users need repeatable project seed or migration workflows that cannot be handled by existing database tools. |
| Bucket migration tooling | MVP RustFS scope is local S3 compatibility and env values. | Users need local bucket copy/backup workflows after RustFS resource behavior is stable. |
| Worker, queue, and scheduler supervision | MVP supervises infrastructure resources, not Laravel app worker processes. | Multiple Laravel projects need first-class queue or scheduler lifecycle management. |
| Per-project PHP extension management | Requires a deeper PHP build and php.ini model than MVP runtime installation. | A stable extension catalog and per-project php.ini strategy are designed. |
| Per-project Xdebug management | Debugger toggles add runtime configuration complexity outside MVP. | PHP runtime model supports per-project php.ini safely. |
| Custom Caddy snippets | Gateway MVP owns deterministic route rendering and TLS behavior. | A real Laravel workflow cannot be represented by the standard route model. |
| LAN sharing and mobile device access | MVP targets local `.test` development on the host machine. | Users need device testing after local HTTPS routing is reliable. |
| Generic command runner | Composer scripts and project setup commands cover MVP needs. | A repeated non-setup workflow cannot be expressed through Composer, Artisan, or explicit helpers. |
| Prototype state migration | The rewrite is pre-GA and does not preserve prototype state layouts. | The project reaches a compatibility-support phase with persisted user state to migrate. |
| Expensive artifact workflows by default | Artifact builds/download-heavy validation are unsafe to run casually. | A specific artifact family is affected and the user explicitly asks to run it. |
| Rust, Zig, Node, or Bun rewrite of pv | The rewrite stays in Go for repository logic and OS orchestration. | Product ownership explicitly changes the implementation language strategy. |
| Heavy TUI-first setup | MVP prioritizes scriptable commands and clear stderr/stdout behavior. | CLI flows are stable and a TUI adds value without replacing scriptability. |
| Store migration checksums | Schema version and applied migration records are enough for MVP planning. | SQLite migrations become complex enough that integrity tracking materially reduces risk. |

## Promotion Rule

A backlog item can move into MVP only after a new planning change names the owning epic or feature, adds acceptance criteria, adds test coverage, and updates the issue hierarchy.
