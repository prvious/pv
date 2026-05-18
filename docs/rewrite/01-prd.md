# pv Rewrite PRD: Laravel-First Local Control Plane

## Problem Statement

Laravel developers who want fast native local development still have to juggle too many moving pieces: HTTPS `.test` domains, PHP versions, Composer, databases, Redis, mail capture, object storage, local DNS, certificates, and long-running processes. Existing tools hide useful details behind magic, rely on Docker or VM layers, or make failures hard to understand when a process dies or a project changes.

`pv` already proves the core direction, but the prototype-era architecture has grown around command workflows and service-specific implementations rather than a clear desired-state model. That makes the product harder to reason about, harder to extend, and easier to regress as the surface area grows.

The rewrite must turn `pv` into a Laravel-first local development control plane: a native, permanent replacement for Laravel Herd that makes the desired local environment explicit, reconciles the machine toward that state, and reports clearly when reality does not match.

## Solution

Rewrite `pv` around a Laravel-first desired-state model.

A Laravel developer installs `pv`, initializes or links a Laravel app, reviews an explicit `pv.yml`, and gets a complete local environment at `https://<app>.test` with a managed PHP runtime, Composer support, HTTPS certificates, DNS, and declared backing services. User commands describe what should exist. The daemon reconciles the machine until the real state matches that desired state.

The rewrite should use a control-plane architecture:

- Resources describe things `pv` manages, such as Laravel projects, PHP runtimes, Composer, databases, Redis, Mailpit, RustFS, and the web gateway.
- The desired-state store is the mutable authority for machine-owned state.
- Controllers reconcile one resource family at a time.
- The supervisor only runs processes and reports liveness; it does not understand Laravel, PHP, databases, or object storage.
- Observed status records what actually happened: running state, failures, ports, process IDs, log paths, and last reconcile time.

The product promise is:

> A Laravel developer can install `pv`, link a Laravel app, and get a complete local development environment that stays running, heals itself, and is easy to understand.

The MVP should be excellent for Laravel local development before it tries to be a general PHP or static-site platform. Generic PHP and static site support may return later, but they should not shape the rewrite architecture or MVP product decisions.

## User Stories

1. As a Laravel developer, I want to install one native CLI, so that I do not need Docker, a VM, Homebrew service juggling, dnsmasq, or Traefik to run local projects.
2. As a Laravel developer, I want to initialize a project with a generated `pv.yml`, so that the local environment contract is explicit and reviewable.
3. As a Laravel developer, I want `pv init` to detect a Laravel project, so that the generated project contract starts with useful Laravel defaults.
4. As a Laravel developer, I want `pv.yml` to declare the PHP version, so that the app runs on the same PHP line every time.
5. As a Laravel developer, I want `pv` to install and use managed PHP runtimes, so that my projects do not depend on system PHP.
6. As a Laravel developer, I want Composer to work through the managed PHP runtime, so that dependency installation is consistent with the project PHP version.
7. As a Laravel developer, I want to link a project and open it at `https://app.test`, so that local development feels like a real HTTPS app.
8. As a Laravel developer, I want `.test` hostnames to resolve automatically, so that I do not manually edit `/etc/hosts` for each project.
9. As a Laravel developer, I want local TLS certificates generated and trusted by `pv`, so that browser, Vite, and Laravel HTTPS workflows work without manual certificate setup.
10. As a Laravel developer, I want to declare aliases for a project, so that related hosts like admin or API subdomains work locally.
11. As a Laravel developer, I want project environment variables to come from `pv.yml`, so that `pv` does not silently infer or mutate my `.env`.
12. As a Laravel developer, I want pv-managed `.env` keys to be visibly labeled, so that I can tell which values are updated by `pv`.
13. As a Laravel developer, I want removing an env key from `pv.yml` to stop future pv updates, so that I can take ownership of that key manually.
14. As a Laravel developer, I want `pv link` to leave undeclared `.env` keys alone, so that project-specific configuration is not clobbered.
15. As a Laravel developer, I want to define setup commands in `pv.yml`, so that project bootstrap steps match my app instead of a hardcoded Laravel pipeline.
16. As a Laravel developer, I want setup commands to run from the project root, so that commands behave like I ran them myself.
17. As a Laravel developer, I want setup commands to fail fast, so that I see the first broken step instead of cascading errors.
18. As a Laravel developer, I want setup commands to run with the pinned PHP version on `PATH`, so that Composer and Artisan commands use the right runtime.
19. As a Laravel developer, I want to declare Postgres for a project, so that the daemon starts the correct database line when the project needs it.
20. As a Laravel developer, I want to declare MySQL for a project, so that apps using MySQL have a native managed database.
21. As a Laravel developer, I want to declare Redis for a project, so that cache, queue, and session workflows can use a native Redis process.
22. As a Laravel developer, I want to declare Mailpit for a project, so that outgoing mail is captured locally without external services.
23. As a Laravel developer, I want to declare RustFS/S3 for a project, so that object-storage workflows can run locally.
24. As a Laravel developer, I want each service binding to resolve to a stable version line, so that local data is not accidentally moved across incompatible service versions.
25. As a Laravel developer, I want missing service installs to produce clear errors, so that I know exactly which install command to run.
26. As a Laravel developer, I want database creation to be an explicit setup command, so that multi-database and custom migration workflows are possible.
27. As a Laravel developer, I want migrations to be my own setup command, so that projects with custom migration commands are supported.
28. As a Laravel developer, I want the daemon to start on login, so that linked projects and services recover without manual process juggling.
29. As a Laravel developer, I want the daemon to reconcile desired state, so that stopped or crashed managed processes are restarted when appropriate.
30. As a Laravel developer, I want status output to show desired state, running state, failures, and next actions, so that I can understand what happened without reading logs first.
31. As a Laravel developer, I want logs available for web and backing services, so that I can diagnose process failures.
32. As a Laravel developer, I want `pv stop`, `pv start`, and `pv restart` to affect declared project infrastructure predictably, so that lifecycle commands are safe to use during daily work.
33. As a Laravel developer, I want service commands to remain first-class, so that Postgres, MySQL, Redis, Mailpit, and RustFS are managed by commands that match their real behavior.
34. As a Laravel developer, I want `pv update` to refresh installed tools and service lines without surprising data migrations, so that updates are understandable.
35. As a Laravel developer, I want unsupported or post-MVP capabilities to be tracked explicitly, so that missing features are not half-implemented or hidden.
36. As a Laravel developer, I want `pv setup` to be a guided wrapper around real commands, so that scripted workflows and interactive workflows behave the same way.
37. As a Laravel developer, I want `pv open` to open the linked app, so that jumping into the browser is a first-class Laravel workflow.
38. As a Laravel developer, I want Laravel helper commands such as Artisan, database, mail, and S3 shortcuts to work through the project contract, so that common Laravel tasks are easy without hiding infrastructure.
39. As a maintainer, I want commands to request state changes instead of doing orchestration work directly, so that command behavior stays thin and testable.
40. As a maintainer, I want the desired-state store to be the only mutable authority for machine-owned state, so that there is one place to answer what should exist.
41. As a maintainer, I want controllers to reconcile one resource family at a time, so that behavior is isolated and debuggable.
42. As a maintainer, I want the supervisor to be a dumb process runner, so that process lifecycle code stays separate from product concepts.
43. As a maintainer, I want observed status to be persisted separately from desired state, so that failures are visible without corrupting the requested target state.
44. As a maintainer, I want resources to expose capabilities instead of inheriting from a generic service model, so that Postgres, Composer, PHP, Mailpit, RustFS, and the gateway can keep their real differences.
45. As a maintainer, I want install planning to support bounded parallel downloads and dependency-ordered installs, so that `pv install` and update workflows are fast but deterministic.
46. As a maintainer, I want state migrations once the product reaches GA, so that future state and filesystem changes are explicit instead of mysterious upgrades.
47. As a maintainer, I want the architecture to separate project config, desired state, observed status, resource controllers, host primitives, installation, supervision, and command UI, so that each subsystem has one clear responsibility.
48. As a maintainer, I want shared service mechanics extracted behind narrow interfaces, so that repeated lifecycle behavior can be tested without forcing every service into a shallow generic model.
49. As a maintainer, I want status and error output to use the project UI layer, so that CLI behavior stays consistent and scriptable.
50. As an automation user, I want commands to return errors and use stdout only for pipeable output, so that scripts can rely on exit codes and clean output.

## Implementation Decisions

- The rewrite remains a Go CLI, but Fang is not a locked dependency for the new architecture. Default to a minimal, scriptable command layer and add CLI presentation helpers only when they clearly improve behavior without creating a UI-first product path.
- Go remains the right implementation language for the primary product because `pv` is mostly OS orchestration: processes, files, ports, downloads, signals, launchd/systemd integration, DNS, TLS, state, daemons, and cross-platform release builds.
- Rewriting the primary product in Rust, Zig, Node, or Bun is not part of this PRD. Those languages may be useful for isolated artifact or tooling work later, but the CLI/daemon rewrite should stay in Go.
- Simplicity is a product and architecture requirement. Remove libraries, wrappers, command surfaces, abstractions, prompts, and styling that do not carry clear user or maintenance value.
- A dependency is justified only when it removes more complexity than it adds. Convenience alone is not enough for the rewrite.
- The MVP is Laravel-first. Generic PHP and static-site support are deferred unless they naturally fall out of the Laravel implementation without shaping it.
- Backward compatibility with prototype installs is not required. Prototype testers can uninstall and reinstall between versions.
- `pv.yml` remains the project-level contract. It should be explicit, reviewable, and suitable for committing to a project.
- YAML is for human-authored project contracts. Machine-owned mutable state should move toward SQLite. Remote artifact metadata may remain generated JSON because it is not user-facing.
- Machine-owned state remains separate from human-authored project config. Runtime state, desired state, observed status, logs, data directories, and registry records should not be hidden inside project config.
- The desired-state store is the mutable authority for what should exist. Commands validate input and write desired state; controllers do the orchestration work.
- User commands request desired state. The daemon reconciles actual machine state toward desired state and records/report failures.
- The daemon should be understandable as a small local control plane: resources, desired state, controllers, supervisor, observed status, and reconciliation.
- Observed status is distinct from desired state and should record running state, failures, port, process ID, log path, last error, and last reconcile time where relevant.
- Controllers own one resource family at a time. They translate desired state into installed artifacts, generated config, supervised processes, route changes, and observed status.
- The supervisor is intentionally dumb. It starts, stops, probes, and restarts processes, but it does not know Laravel, PHP, Postgres, Redis, Mailpit, RustFS, or Caddy semantics.
- Public command groups stay service-specific. Postgres, MySQL, Redis, Mailpit, and RustFS remain first-class concepts with their own command groups and aliases where already established.
- Do not reintroduce a generic `service:*` namespace. It hides real differences between databases, caches, mail capture, object storage, and web serving.
- Resources should be modeled by capabilities rather than fake inheritance. Useful capabilities include installable, runnable, stateful, exposes env, has database commands, has HTTP console, has CLI shim, and depends on runtime.
- PHP is a runtime, not just a tool. It also provides CLI shims and supports project execution.
- Composer is a tool that depends on a PHP runtime.
- FrankenPHP is gateway infrastructure for web projects, not a user-managed backing service like Mailpit.
- Service versions use stable version-line identities. Moving across incompatible lines is explicit and does not imply data migration.
- Managed service binaries live under explicit service-specific version roots. User PATH entries remain shims and symlinks only.
- The filesystem layout should stop growing organically. Real binaries should not live ambiguously at the top level, and services should not invent special-case storage paths.
- PHP runtimes remain versioned and managed by `pv`, with project version resolution driven by `pv.yml`.
- Composer support should be integrated with managed PHP instead of depending on system PHP.
- The install/update flow should use an install planner: validate a plan, download artifacts in bounded parallelism, install in dependency order, expose shims atomically, then signal reconciliation.
- `pv setup` should be a thin guided wrapper around real commands and desired-state changes, not a separate UI-first experience.
- The default UX should be scriptable first and polished second. Rich terminal output is welcome only when it does not create a different product path from the CLI.
- Do not carry Fang forward from the prototype by default. It is likely unnecessary friction for the rewrite's simple command model. If the rewrite later wants Fang or another command presentation wrapper, that should require a deliberate architecture decision with a concrete reason.
- Before new rewrite code is introduced at the repository root, move the current prototype implementation into `legacy/prototype/` as a buildable Go module. This keeps old `cmd/`, `internal/`, and related package names available for reference without colliding with the new root-level architecture.
- The project contract module owns parsing, validation, defaults, and template rendering for `pv.yml`.
- The init-generation module owns project detection and generated `pv.yml` content, with Laravel as the primary template.
- The project environment module owns `.env` parsing, managed labels, backups, and merge semantics.
- The setup-runner module owns user-declared setup commands, working directory behavior, environment propagation, PHP PATH pinning, and fail-fast execution.
- The site gateway module owns `.test` hosts, aliases, Caddy configuration, DNS integration, and TLS certificate wiring.
- The PHP runtime module owns PHP version installation, version resolution, shims, and per-version serving needs.
- The tool module owns Composer and other developer-tool installation, update, uninstall, and PATH exposure behavior.
- The managed-service lifecycle modules own install, update, uninstall, wanted state, installed state, process definitions, readiness, logs, template variables, and service-specific behavior.
- Shared service helpers should extract mechanics only: state persistence, artifact installation skeletons, wait/readiness helpers, command choreography, alias registration, and daemon reconciliation descriptors.
- Service-specific modules should keep service behavior explicit: database initialization and create/drop behavior, Redis process options, Mailpit SMTP/web behavior, RustFS credentials/routes, and version-line policies.
- The registry module records linked projects and resolved service bindings, using concrete version-line identities rather than empty or moving aliases.
- The status model should report desired state, observed process state, failure state, and next action in a form that is readable by humans and stable enough for automation.
- Commands that mutate daemon-observed state should signal the daemon after persistent state changes are complete. Read-only commands should not signal.
- `pv link` should not infer services from `.env`, write undeclared env keys, copy `.env.example`, run Composer, generate keys, run migrations, install Octane, create databases, or set Vite/TLS variables unless those actions are declared in `pv.yml`.
- `pv unlink` should remove project registration and web routing, but it should not edit `.env`, drop databases, delete buckets, or remove `pv.yml`.
- Database and bucket creation are explicit user actions, available as service-specific commands and usable from setup commands.
- Status output and new user-facing command output should use the existing UI helpers and stderr unless the command is intentionally pipeable.
- Before GA, state and storage can break freely. At GA, introduce explicit state schema versioning, contract versioning, and forward-only migrations recorded in the store.
- Detailed rewrite architecture lives in `docs/rewrite/02-architecture.md`. This PRD defines the product scope and locked decisions; the architecture document explains the control-plane model and implementation boundaries.

## Testing Decisions

- Tests should verify external behavior and contracts rather than implementation details. A good test describes what a user command, parser, reconciler, or lifecycle API promises, not which private helper happened to run.
- Tests that touch pv state must isolate `HOME` with a temporary directory.
- Cobra tests should build a fresh command tree per test and should not mutate package-level command state.
- Command tests should focus on scriptable CLI behavior and should not assume Fang-owned formatting, spacing, or error presentation in the rewrite.
- Registry tests should continue to load, mutate, save, and reload because registry changes are in-memory until saved.
- Project contract tests should cover schema parsing, defaults, validation, service blocks, aliases, env templates, setup commands, and unsupported configuration errors.
- Init-generation tests should cover Laravel detection, generated Laravel defaults, refusal to overwrite existing config, forced overwrite, and non-Laravel fallback behavior when included.
- Project environment tests should cover managed labels, updating declared keys, preserving non-pv keys, backup creation, and leaving removed declarations untouched.
- Setup-runner tests should cover command ordering, working directory, environment propagation, PHP PATH pinning, stdout/stderr streaming behavior, and fail-fast errors.
- Site gateway tests should cover primary host generation, aliases, certificate subject alternative names, Caddy configuration, and deterministic routing output.
- PHP runtime tests should cover version resolution from the project contract, missing-version behavior, shim behavior, and per-version serving behavior.
- Managed-service tests should cover default version resolution, version validation, artifact URL/name resolution, installed-version detection, wanted-state persistence, process definitions, readiness checks, logs, update semantics, and uninstall behavior.
- Daemon reconciliation tests should cover desired-running, desired-stopped, missing install, crashed process, restart-budget exhaustion, stale wanted entries, and status snapshots.
- Command-layer tests should cover first-class service commands, hidden aliases, user-facing errors, daemon signaling after mutations, and no signaling for read-only commands.
- Store tests should cover desired-state persistence, observed-status persistence, locking, schema versioning, and forward-only migrations once the SQLite store lands.
- Installer planner tests should cover bounded parallel download scheduling, dependency-ordered install execution, atomic shim exposure, failure behavior, and daemon signaling after successful state changes.
- Supervisor tests should continue to prove that supervision is resource-agnostic: process start, stop, readiness, crash restart, restart budget, and log handling.
- Capability/resource tests should prove each resource advertises and implements only the behavior it actually supports rather than satisfying a generic service interface.
- E2E tests should cover real binaries, network behavior, DNS, HTTPS, daemon startup, and a fresh Laravel app served at `https://<app>.test`.
- Prior art already exists across command tests, config tests, project env tests, service lifecycle tests, supervisor tests, registry tests, Caddy tests, daemon tests, and e2e scripts. The rewrite should preserve that style while expanding coverage around the new desired-state contract.
- Before handing off Go changes, run the repository's full verification set: formatting, vetting, build, and tests.

## Out of Scope

- Laravel Octane support is out of scope for MVP.
- Generic PHP and static-site polish are out of scope for MVP unless they are incidental and do not shape the Laravel-first architecture.
- Docker, VM orchestration, and container-based service management are out of scope.
- Hidden service or env setup based on `.env` hints is out of scope.
- Automatic cross-line data upgrades or migrations for Postgres, MySQL, Redis, Mailpit, or RustFS are out of scope.
- Database dump/import tooling is out of scope for MVP.
- Bucket migration tooling is out of scope for MVP.
- Worker, queue, and scheduler supervision are out of scope for MVP.
- Per-project PHP extension management is out of scope for MVP.
- Per-project Xdebug management is out of scope for MVP.
- Custom Caddy snippets and advanced framework-specific web-server tuning are out of scope for MVP.
- LAN sharing and mobile-device access are out of scope for MVP.
- A generic command runner beyond declared setup commands is out of scope.
- Backward-compatible migration from prototype state layouts is out of scope.
- Publishing all artifact families or running expensive artifact workflows by default is out of scope.
- Rewriting the primary CLI/daemon in Rust, Zig, Node, or Bun is out of scope.
- A heavy TUI-first setup experience is out of scope; guided setup should wrap the same commands and state transitions as scriptable CLI use.
- Carrying the prototype's Fang-based command presentation into the rewrite by default is out of scope.

## Further Notes

The core product sentence is:

> `pv` is a Laravel-first local desired-state control plane.

The implementation should optimize for clarity, reliability, and long-term architecture over preserving prototype-era decisions. The project is still pre-GA, so removing weak abstractions is acceptable when it produces a simpler model.

The simplicity test for every dependency or abstraction is:

> Does this make the product easier to understand, operate, test, and maintain than the smallest direct implementation?

If the answer is not clearly yes, leave it out.

The most important architectural boundary is between explicit service behavior and shared lifecycle mechanics. Avoid fake sameness. Databases, HTTP tools, PHP runtimes, CLI tools, and the web gateway should be modeled by what they actually do.

The architecture target is:

> Commands do not do work. They request state changes.
> Controllers do work.
> The supervisor only runs processes.
> The store is the authority.
> Resources expose capabilities, not fake sameness.
> Laravel is the primary product path.

The post-MVP backlog should explicitly track omitted capabilities so they are not forgotten or partially implemented inside MVP work.
