# PV Product Ideas

Top-level sections are ideas that are strong enough to navigate as product
directions. Parked maybes live in the appendix so they do not compete with the
main idea list.

## Table of Contents

- [Guided Project Init](#guided-project-init)
- [Project Hooks / Command Runner](#project-hooks--command-runner)
- [Curated Global Tools](#curated-global-tools)
- [Resource Shell Commands](#resource-shell-commands)
- [Resource Data Commands](#resource-data-commands)
- [Post-v1 Managed Resource Candidates](#post-v1-managed-resource-candidates)
- [TUI / `pv.test` Dashboard](#tui--pvtest-dashboard)
- [Richer CLI Presentation](#richer-cli-presentation)
- [ZDOTDIR-Aware Shell Integration](#zdotdir-aware-shell-integration)
- [Agent-Friendly CLI Contracts](#agent-friendly-cli-contracts)
- [Project TLS Placeholders](#project-tls-placeholders)
- [PHP Extension Profiles](#php-extension-profiles)
- [Auto-Reconcile Action Commands](#auto-reconcile-action-commands)
- [Project Details Command](#project-details-command)
- [Gateway Unknown Hostname Page](#gateway-unknown-hostname-page)
- [LAN Project Access](#lan-project-access)
- [Resource-Only Projects](#resource-only-projects)
- [Appendix: Parking Lot / Maybes](#appendix-parking-lot--maybes)
  - [Component-Specific Doctor Commands](#component-specific-doctor-commands)
  - [Project Command Sandboxing](#project-command-sandboxing)
  - [Worktree Environment Commands](#worktree-environment-commands)
  - [`.localhost` Project Hostnames](#localhost-project-hostnames)
  - [Targeted Runtime Restart](#targeted-runtime-restart)

## Guided Project Init

PV should probably include a guided Project config initializer in v1.

This contradicts the current `DESIGN.md`, which says:

> PV v1 does not include `pv init`, does not create sample Project config files
> during setup, and does not create Project config during `pv link`.

That design decision should be revisited. Herd already has `herd init`, DDEV has
`ddev config`, Laragon has Quick App, and FlyEnv users are asking for richer
Laravel project customization. If `pv.yml` is supposed to be the elegant,
team-shareable entrypoint, PV should help users create the first one.

### Product Shape

The command should be focused on existing directories:

```sh
pv init
pv init path/to/project
```

It should generate or update a `pv.yml` for a Project. It should not create a new
Laravel application, run `composer install`, run `pnpm install`, migrate
databases, or execute framework setup commands. Those actions belong to hooks or
user-owned scripts.

In other words:

- `pv init` helps write Project config.
- `pv link` registers and reconciles a Project.
- hooks run user-owned commands when the user opts into that idea.

This keeps PV from becoming a project scaffolder too early while still avoiding
the bad first-run UX of making users hand-write config from documentation.

### Interaction

The default experience should be interactive and conservative.

PV can inspect the directory and suggest values, but the user should confirm
what gets written. Useful detection could include:

- Laravel or generic PHP Project shape
- likely document root, such as `public`
- PHP track, defaulting to PV's default when uncertain
- whether `.env.example` or `.env` exists
- likely database/cache/mail/object-storage needs from existing env keys
- whether the Project should be served by the Gateway or marked `serve: false`

PV should show the proposed `pv.yml` before writing it, or at least print a clear
summary and the output path. Existing files should not be overwritten without
confirmation.

### Detection Scope

Framework detection belongs in `pv init`, not `pv link`.

`pv init` can inspect common Project files to make better suggestions:

- `composer.json` for Laravel, generic PHP shape, and package hints
- `artisan`, `bootstrap/app.php`, `config/app.php`, and `public/index.php` for
  Laravel confidence
- `package.json` for frontend tooling signals such as Vite, Next.js, or build
  scripts
- `.env.example` and `.env` for likely resource needs such as database, Redis,
  mail, object storage, and app URL mappings
- directory layout for document root and resource-only hints

Detection should produce suggestions, not hidden behavior. PV should explain the
detected shape, show the proposed config, and let the user confirm or edit.

### Migration-Assisted Init

PV should provide a migration path for Herd users through `pv init`, not through
a separate migration command namespace.

Possible command shape:

```sh
pv init --migrate herd
pv init path/to/project --migrate herd
```

This mode should still behave like guided init: inspect the existing Project,
translate what can be translated safely, show the proposed `pv.yml`, and ask
before writing.

The first supported migration target should be Herd. That is the clearest
competitive path and avoids designing a generic migration framework too early.
Other sources such as Valet or DDEV can wait until users ask.

Useful Herd migration inputs might include:

- existing hostname / site name
- PHP version when detectable
- document root
- common Laravel shape
- `.env` / `.env.example` resource hints
- future Herd team config if it becomes common enough to translate

This should not:

- import databases
- copy certificates
- run Composer, pnpm, Artisan, or framework commands
- rewrite application `.env` values outside PV's managed block
- attempt to fully emulate Herd behavior

Database movement belongs to the later Resource Data Commands idea. A good
future migration flow can tell users the next command to run, such as
`pv mysql:import`, but `pv init --migrate herd` should focus on generating the
PV Project config.

`pv init` should not:

- infer a PHP version from complex Composer constraints as a hard fact
- run framework commands
- install packages
- generate app secrets
- run migrations
- diagnose application correctness
- deeply customize framework-specific starter kits

`pv link` should stay simple. It may eventually print a hint when no `pv.yml`
exists, such as:

```text
No pv.yml found. Run `pv init` to generate one.
```

But `pv link` should not perform framework detection or write Project config.

### Non-Goals

This should not be a Laragon-style Quick App in v1.

Avoid:

- `laravel new`
- starter kit selection
- Breeze/Jetstream installation
- package manager execution
- migrations
- application secrets
- long-running project processes
- framework-specific deep customization

Those are useful later, but they cross into scaffolding and command execution.
For v1, the win is simply making `pv.yml` creation feel obvious and polished.

### Open Questions

- Should the command be `pv init`, `pv project:init`, or both?
- Should `pv link` suggest `pv init` when no Project config exists?
- Should `pv init` support non-interactive output for templates or docs, such as
  `pv init --print`?
- Should `pv init` be allowed to add hooks, or only resource/env declarations?
- Should `pv init` ever touch `.env`, or should it only write `pv.yml`?

## Project Hooks / Command Runner

PV could support a simple Project hook runner in `pv.yml` so a Project can declare
the commands needed to become usable after linking.

This is intentionally a product idea, not a spec. The current `DESIGN.md` says PV
v1 does not automatically run package manager or Laravel application commands
during `pv link`. This idea would deliberately change that behavior.

Example:

```yaml
php: "8.4"

env:
  APP_URL: "${project_url}"

postgres:
  version: "8.0"
  allocations:
    laravel-db:
      env:
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
        DB_PORT: "${port}"

hooks:
  prepare:
    - test -f .env || cp .env.example .env

  setup:
    - composer install
    - pnpm install
    - php artisan key:generate
    - x-migrate
```

### Product Shape

`hooks.prepare` runs before PV mutates Project state.

PV still has to read and parse `pv.yml` first so it knows the hook exists. After
that, `prepare` should run before PV records the Project in `pv.db`, provisions
Managed Resources, creates Resource allocations, or writes the PV-managed `.env`
block.

This gives users a clean place for local Project file prep, such as copying
`.env.example` to `.env`. If `prepare` fails, `pv link` stops before PV has
anything meaningful to roll back.

`hooks.setup` runs immediately after PV renders the PV-managed `.env` block.

This is the place for dependency installation, app keys, migrations, local asset
build steps, and custom framework setup commands. PV should not classify these
commands or infer risk from their names. `php artisan migrate`, `x-migrate`,
`just setup`, and `./bin/bootstrap` are all just raw user-owned commands.

### Lifecycle

The intended `pv link` flow:

1. Read and parse Project config.
2. Run `hooks.prepare`.
3. Record/link the Project as desired state.
4. Reconcile Managed Resources and Resource allocations.
5. Render the PV-managed `.env` block.
6. Run `hooks.setup`.
7. Finish Project serving reconciliation.

If `pv link` runs before `pv setup`, `prepare` can still run immediately because
it does not need PV infrastructure. `setup` should be deferred until `pv setup`
starts the daemon, reconciles the Project, and renders `.env`.

When `pv setup` later handles Projects that were linked before setup, it should
run deferred `setup` hooks per Project. A failed Project hook should not stop
other Projects from reconciling or running their own hooks.

The Project config watcher should not run hooks. Editing `pv.yml` should trigger
normal reconciliation only, not arbitrary shell commands.

### Execution Contract

Hooks are deliberately dumb:

- Commands run sequentially.
- Commands fail fast within a Project.
- PV does not diff hook definitions.
- PV does not track whether command text changed.
- PV does not decide whether a command is necessary.
- The user owns idempotency.
- PV owns ordering, environment, output, logging, and failure reporting.

Re-running `pv link` should run the hooks again. If users do not want that, they
can skip hooks for that invocation.

Likely escape hatch:

```sh
pv link --skip-hooks
```

### Failure Behavior

If `prepare` fails:

- Stop immediately.
- Do not record/link the Project.
- Do not reconcile resources.
- Do not write `.env`.
- Exit non-zero.

If `setup` fails:

- Stop at the first failed command.
- Keep the Project linked.
- Keep reconciled Managed Resources and Resource allocations.
- Keep the rendered `.env` block.
- Mark the Project degraded or failed for hook execution.
- Exit non-zero for the foreground command that ran the hook.
- Log stdout/stderr and show the failed command clearly.

For `pv setup` with multiple previously linked Projects, hook failures are scoped
per Project. PV should continue processing other Projects and exit non-zero at
the end if any deferred hook failed.

### Shell Choice

Run each hook command through:

```sh
/bin/sh -c '<command>'
```

Do not use the user's `$SHELL`, do not use a login shell, and do not source shell
profiles by default. This keeps behavior deterministic and avoids aliases,
functions, shell plugins, and unrelated startup side effects.

Each command should run with:

- working directory set to the Project root
- PV shims prepended to `PATH`
- PV-managed Composer environment values set explicitly
- the normal inherited environment otherwise

Users who need another shell can opt in inside their own command:

```yaml
hooks:
  setup:
    - zsh -lc 'source ~/.zshrc && custom-command'
```

For anything complex, Projects should prefer a script:

```yaml
hooks:
  setup:
    - ./bin/pv-setup
```

### Names Considered

Preferred names:

- `prepare`: before PV mutates Project state
- `setup`: after PV renders `.env`

Names that felt too implementation-focused:

- `before_env`
- `after_env`
- `before_resources`
- `after_resources`
- `after_gateway`

Names that may be useful later but should not be part of the first version:

- `ready`: after the Project hostname is expected to be reachable
- `cleanup`: around unlink or explicit cleanup flows

### Open Questions

- Does PV need a small deferred-hook marker so repeated `pv setup` does not run
  deferred `setup` hooks forever?
- Should there be a separate command to rerun only hooks, such as
  `pv project:setup`, or is `pv link` enough?
- Should hook status appear in `pv list`, `pv status`, both, or only logs?
- Should hook command output be stored in the normal daemon job logs, a
  Project-specific hook log, or both?

## Curated Global Tools

PV could offer a curated local development tool installer for global CLI tools
that are useful in Laravel/PHP workflows. The first candidate is the Laravel
Installer.

This should not be modeled as a normal Managed Resource artifact. Composer is
already a PV-managed artifact, but Composer global packages live in the
user-facing Composer home:

```text
~/.pv/composer/
  composer.json
  composer.lock
  vendor/bin/
```

PV already includes `~/.pv/composer/vendor/bin` in `pv env`, so Composer global
tool binaries are already exposed on `PATH` when users opt into PV shell
integration. For example, installing `laravel/installer` globally should expose
the `laravel` binary without PV creating an extra shim.

### Product Shape

This is a quality-of-life feature, not core infrastructure.

The most useful interface is an interactive picker:

```sh
pv tools
```

Possible UI shape:

```text
PV tools

[ ] Laravel Installer     laravel/installer
[ ] Laravel Pint          laravel/pint
[ ] PHPStan               phpstan/phpstan
[ ] Pest                  pestphp/pest

Install selected tools? [Y/n]
```

Scriptable commands can still exist for documentation, automation, and tests:

```sh
pv tools:list
pv tools:install laravel
```

The picker is important because the feature is mostly about discovery and
curation. If PV only exposes `pv tools:install <tool>`, it is not much better
than telling users to run `composer global require <package>` themselves.

### Registry

The registry should be structured, not a set of arbitrary shell commands.

Preferred shape:

```yaml
laravel:
  manager: composer
  package: laravel/installer
  binaries:
    - laravel
```

Avoid this shape:

```yaml
laravel:
  manager: composer
  package: laravel/installer
  install: composer global require laravel/installer
  update: composer global require laravel/installer
```

Raw command strings look flexible, but they turn the registry into a command
execution system. If the registry ever becomes remote, that gets especially
dangerous. A structured registry lets PV derive safe manager-specific behavior
for install, update, remove, status, and diagnostics.

For Composer-managed tools, PV can derive commands such as:

```sh
composer global require laravel/installer
composer global update laravel/installer --with-dependencies
composer global remove laravel/installer
```

The `binaries` field is still useful even though Composer handles installation.
PV can use it to show users what command they will get and to verify that the
expected binary exists after installation.

### Tool Metadata

The curated tools registry can expose helpful metadata without becoming a
plugin or add-on command system.

The interactive picker and `tools:list` output should eventually be able to
show:

- package name
- manager
- installed binaries
- install location
- update path
- trust boundary

For example:

```text
Laravel Installer
Package: laravel/installer
Manager: Composer
Binary: laravel
Installs into: ~/.pv/composer/vendor/bin
Updates with: pv update
Trust: third-party Composer package
```

Do not use this as a way to register arbitrary commands. PV should show what it
is managing and how, but it should not become an add-on/plugin command runner.

### Updates

PV should not silently update tools in the background.

Composer/npm-style packages can change behavior, run scripts/plugins, or become
compromised upstream. Background updates would make executable code change
without an explicit user action, which is not a good default.

Instead, installed curated tools should update during:

```sh
pv update
```

That keeps the maintenance story simple:

> Anything PV installed can be updated by running `pv update`.

Tool-specific update commands may be added later if useful, but they are not the
core value of the feature.

### Boundaries

Curated tools are different from Managed Resources.

Managed Resources are installed artifacts that PV provisions, starts,
reconciles, supervises, or uses as local runtime infrastructure. Curated tools
are global CLI packages installed through an existing package manager such as
Composer.

For a first version, Composer is the only manager worth supporting. Future
managers such as pnpm could reuse the same concept if PV later supports them.

### Open Questions

- Should PV track selected curated tools in `pv.db`, or infer installed tools
  from Composer global state?
- Should `pv tools:list` show all curated tools, installed tools only, or both?
- Should `pv tools` support uninstalling from the same picker, or only
  installing?
- Should installed curated tools participate in `pv uninstall --prune`, or is
  preserving `~/.pv/composer` already enough?

## Resource Shell Commands

PV should strongly consider shell commands for SQL Managed Resources in v1.

Competitor research showed that users value fast access to local databases and
resource tooling. PV already has `pv project:env`, `pv mailpit:open` /
`pv mail:open`, and `pv rustfs:open` / `pv s3:open`, so this idea should not
duplicate those commands.

The missing ergonomic piece is opening the correct database client with the
correct PV-managed connection details.

### Namespace

Do not introduce a generic `db:*` namespace.

PV already has explicit Managed Resource namespaces, and `db` is ambiguous. It
could mean MySQL, Postgres, Redis, SQLite, or some future database-like Managed
Resource. Commands should stay under the resource they operate on:

```sh
pv mysql:shell
pv mysql:shell 8.4

pv postgres:shell
pv postgres:shell 18

pv pg:shell
pv pg:shell 18
```

The optional positional track can resolve the same way other resource commands
resolve tracks. If omitted, PV can use the manifest default track or the only
installed/running track, depending on the final command design.

The exact resolution rules need a real design pass. The core product idea is the
command, not the track-resolution contract.

### Product Shape

`mysql:shell` should launch the PV-managed MySQL client for the selected MySQL
track with the correct host, port, username, and password.

`postgres:shell` / `pg:shell` should launch the PV-managed `psql` client for the
selected Postgres track with the correct host, port, username, and password.

The first version can be resource-track oriented rather than Project-allocation
oriented. It is enough to connect users to the running local database server.
Later, PV can consider Project-aware selection for a specific Resource
allocation database.

### Boundaries

These commands are interactive convenience commands. They should not:

- print secrets in broad status output
- rotate credentials
- create or delete databases
- run migrations
- inspect application schemas
- create a generic `db:*` namespace

`pv project:env` remains the explicit command that prints generated Project env
values, including secrets.

### Open Questions

- Should `mysql:shell` / `postgres:shell` connect to the server default database,
  or try to infer the current Project's first SQL Resource allocation?
- If the current Project has multiple SQL allocations, should PV show a picker
  or require an explicit allocation selector?
- Should there be a non-interactive flag to print the equivalent native client
  command without executing it?
- Should PV expose SQL shell commands before or after Project-aware allocation
  selection exists?

## Resource Data Commands

PV should keep database import/export/snapshot/restore/backups as a compelling
post-v1 or v1.1 idea.

DDEV's database tooling is a major product advantage, and Laragon/FlyEnv users
also ask for backup and admin workflows. This is worth copying eventually, but
it is bigger than a small shell command because restore/import operations are
destructive and each resource has different semantics.

Possible future commands:

```sh
pv mysql:export
pv mysql:import
pv mysql:snapshot
pv mysql:restore
pv mysql:backups
pv mysql:clone

pv postgres:export
pv postgres:import
pv postgres:snapshot
pv postgres:restore
pv postgres:backups
pv postgres:clone

pv pg:export
pv pg:import
pv pg:snapshot
pv pg:restore
pv pg:backups
pv pg:clone
```

These should stay under explicit resource namespaces. Do not use `db:*`.

### Resource Cloning

Resource cloning is not confirmed yet, but it is stronger than a random maybe.

The compelling use case is worktree and agent workflows. A developer or agent
could have several linked worktrees based on `main`, each with its own Project
allocation. PV could clone the main Project's database into each worktree's
database so every checkout starts with useful local data without hand-written
dump/import commands.

This should not become `pv project:clone`. Worktrees already use normal
`pv link`; cloning is about copying backing Resource data between explicit
allocations.

Possible command shape:

```sh
pv postgres:clone <source> <target>
pv mysql:clone <source> <target>
```

The hard part is architecture, not naming. Cloning may be slow depending on
database size, disk speed, engine behavior, and whether PV can use an efficient
local snapshot path or has to fall back to dump/restore. A real design should
think through progress output, cancellation, confirmation before overwriting
target data, whether to snapshot the target first, and how source/target
selectors resolve to Project Resource allocations.

### Why This Is Later

The design needs to answer:

- whether operations target a whole Managed Resource track or a Project Resource
  allocation
- how clone source/target selectors resolve across linked Projects and
  allocations
- whether PV snapshots before destructive restore/import
- file formats and compression
- whether clone is implemented as dump/restore, engine-native copy, or a
  resource-specific fast path
- MySQL vs Postgres differences
- backup retention and naming
- whether Redis and RustFS need equivalent backup concepts
- how to avoid surprising users who expect PV to preserve local data by default

This idea is captivating, but it needs a deliberate design before it becomes a
command surface.

## Post-v1 Managed Resource Candidates

PV should have room to grow beyond the initial Laravel-first v1 Managed Resource
set.

The current v1 set is already broad enough: PHP/FrankenPHP, MySQL, Postgres,
Redis, Composer, Mailpit, and RustFS. Future resources should be added
deliberately, one at a time, after the resource adapter/artifact pipeline is
boring.

Strong future candidates:

- Meilisearch
- Typesense
- Valkey
- ClickHouse

Other possible candidates:

- MariaDB
- MongoDB
- OpenSearch / Elasticsearch
- Redis Stack or Redis module-aware variants, only if Redis 8 does not cover the
  practical local-dev need

### Product Shape

Do not create a generic module marketplace.

Each supported resource should be a first-class PV resource with:

- a PV-owned artifact recipe or wrapped upstream artifact
- manifest tracks and update behavior
- install/update/uninstall/list commands
- daemon runtime adapter
- readiness checks
- logs
- status/doctor coverage
- Project config support
- env placeholder contract
- clear data retention and prune behavior

Command namespaces should stay explicit:

```sh
pv meilisearch:install
pv meilisearch:list

pv typesense:install
pv typesense:list

pv valkey:install
pv valkey:list

pv clickhouse:install
pv clickhouse:list
```

Avoid a vague `service:*`, `module:*`, or `resource:*` public namespace for
normal user workflows.

### Likely Order

Meilisearch and Typesense are probably the best early additions. They are common
in Laravel apps through Scout/search workflows and have clear local-development
value.

Valkey is useful if users want a Redis-compatible alternative or if Redis
licensing/community pressure keeps mattering. Since PV already targets Redis 8,
Valkey should be demand-driven rather than automatic.

ClickHouse is interesting for analytics-heavy apps, but it is a larger product
surface than search services. Add it only after the common web-app resources are
solid.

MariaDB and MongoDB can wait for explicit demand. OpenSearch/Elasticsearch are
heavy enough that PV should be careful before taking them on.

### Allocation Questions

Not every resource needs Project allocations in the first version.

Search services may only need resource-level env values at first:

```yaml
meilisearch:
  version: "latest"
  env:
    MEILISEARCH_HOST: "${url}"
    MEILISEARCH_KEY: "${key}"
```

Later, PV can decide whether a resource needs Project-specific objects such as
indexes, databases, users, tokens, or namespaces.

The important rule is the same as the v1 resources: PV should not pretend it
manages application data semantics. It can create local infrastructure and basic
access credentials, but application schemas, indexes, migrations, and data
seeding remain user-owned.

## TUI / `pv.test` Dashboard

PV should consider a dashboard experience after the CLI foundation is stable.

Competitor research points in this direction: Herd has a GUI Site Manager,
Laragon's tray/menu workflow is a major part of its appeal, FlyEnv is heavily
GUI-driven, and DDEV's community pushed toward a TUI before a full GUI.

The likely order should be:

1. TUI first.
2. `pv.test` internal web dashboard later.
3. Native GUI/menu bar much later, if ever.

### TUI First

A terminal dashboard fits PV's CLI-first product shape better than a native GUI.
It could become a fast way to inspect and act on local state without adding a
large desktop application surface.

Possible command shapes:

```sh
pv
pv dashboard
```

The exact command name needs a later design pass.

Useful TUI content:

- linked Projects
- Project hostnames
- PHP tracks
- Managed Resource health and ports
- recent jobs
- config/env/hook failures
- quick actions for opening Projects, Mailpit, RustFS, and logs

The TUI should be a view over existing daemon/state APIs. It should not require
special polling hacks or a separate state model.

### TUI Log Viewer

PV could also offer a focused terminal log viewer before a full dashboard.

This is different from making normal `pv logs` richer. The plain command should
stay simple, pipeable, and script-friendly. A TUI log viewer is for the human
case where someone wants to watch several PV-owned streams at once without
opening multiple terminal tabs.

Possible command shape:

```sh
pv logs --tui
pv logs --tui --all
pv logs --tui --gateway
pv logs --tui --resource mysql --track 8.0
```

The command should reuse the existing `pv logs` source selection model instead
of inventing a separate logs product.

Useful behavior:

- split panes or tabs for daemon, LaunchAgent, Gateway, workers, and Managed
  Resources
- source labels and severity coloring
- pause/resume following
- search/filter within visible logs
- quick source toggles
- clear empty-state messages when a selected log does not exist yet

Boundaries:

- read-only
- no daemon control from the log viewer
- no mutation or auto-reconcile
- no secret scraping or Project `.env` display
- normal `pv logs`, `pv logs --follow`, and `pv logs --all` remain the stable
  non-TUI interface

### `pv.test` Later

`DESIGN.md` already reserves `pv.test` for PV diagnostics or a future internal
UI. That makes it a natural place for a local web dashboard later.

The first version should probably be read-only or mostly read-only:

- Projects
- Managed Resources
- Mailpit/RustFS links
- logs
- diagnostics
- generated env preview
- recent jobs

Mutating actions from a browser UI are a much bigger product and security
surface, even locally. They should wait until the CLI behavior is already
boring and well understood.

### Native GUI

A native desktop GUI or menu bar app should be deferred hard.

It may become useful later, but it is expensive and risks pulling PV toward the
same broad surface area as Herd/FlyEnv before the CLI control plane is excellent.

## Richer CLI Presentation

PV should have a cleaner, richer, more minimal CLI presentation across important
commands.

This is separate from a TUI or dashboard. It applies to normal command output:

```sh
pv setup
pv status
pv doctor
pv list
pv jobs
pv mysql:list
pv postgres:list
```

Current command output is functional, but it can be noisy and flat. For example,
`pv setup` prints every step as plain lines, `pv doctor` prints every passing
check, and `pv status` reads like a raw state dump. That is good for tests and
debugging, but not necessarily a polished product experience.

### Product Feel

PV should feel calm, minimal, and precise.

The default human output should emphasize:

- what happened
- what needs attention
- the next command to run
- paths and details only when useful

It should avoid making successful flows feel like logs.

Possible presentation direction:

- group related lines into compact sections
- hide successful low-level details by default
- show warnings/failures before verbose successful checks
- use color and symbols in TTY output, while preserving clear words
- keep `NO_COLOR` and `--no-color` deterministic
- provide detail flags where needed, such as `--verbose`
- keep JSON output stable and boring for scripts

### Command-Specific Notes

`pv setup` should feel like a guided installer, not a log dump. It can show a few
major phases and then a concise success summary.

`pv doctor` should probably default to failed/warning checks plus a compact
summary. Passing checks can be collapsed unless `--verbose` is used.

`pv status` should read like a dashboard summary: overall health first, then the
few things the user should care about. It should not duplicate every detail that
belongs in `pv doctor`, `pv list`, or resource-specific list commands.

`pv list` and resource list commands should be easy to scan. Tables should be
compact, aligned, and avoid noisy placeholder values when possible.

Project selectors should also feel polished.

This mainly affects `pv open` when run outside a linked Project, and later the
TUI/dashboard command surfaces. Once users have many linked Projects, a plain
numbered list becomes tedious even if it is deterministic.

Useful selector polish:

- searchable / fuzzy filtering by primary hostname and path
- compact health indicators for failed or degraded Projects
- additional hostname count or hint without making each hostname a separate row
- recent Projects can influence initial ordering if PV can derive that without
  adding user-managed metadata
- keyboard navigation in TUI contexts

Avoid persistent Project favorites, folders, or arbitrary groups for now. They
add metadata and product surface without solving a core PV problem.

Terminal hyperlinks are another worthwhile polish pass.

When output is going to an interactive terminal that supports OSC 8 links, PV
can make obvious URLs and paths clickable while keeping the visible text normal
and copyable.

Good candidates:

- Project URLs
- Mailpit/RustFS dashboard URLs
- log directories and log files
- Project config paths
- generated Gateway/worker config paths when shown for diagnostics

Boundaries:

- never in JSON
- never for secrets, DSNs, or generated env values
- disabled for non-TTY output
- disabled by `NO_COLOR` / `--no-color`, or by a later plain-output flag
- implemented in the shared output layer rather than one command at a time

### Boundaries

This is presentation polish, not a behavior change.

The underlying command contracts should stay intact:

- no hidden mutations
- JSON output remains scriptable
- secrets remain redacted outside explicit commands such as `pv project:env`
- plain output remains usable without color
- tests should continue to snapshot deterministic output

## ZDOTDIR-Aware Shell Integration

PV should respect `$ZDOTDIR` for zsh shell profile integration.

This is small v1 setup polish, not a new product surface. Today the design says
PV edits `~/.zprofile` for zsh, and the current implementation follows that.
That works for the default macOS zsh setup, but zsh users can move their startup
files by setting `$ZDOTDIR`. For those users, writing `~/.zprofile` can appear to
succeed while new terminals never load PV's shell integration.

### Product Shape

For zsh only:

- if `$ZDOTDIR` is set to a sane absolute directory, use
  `$ZDOTDIR/.zprofile`
- otherwise keep using `~/.zprofile`
- still edit only one detected shell profile file
- keep the same confirmation, backup, reporting, `--yes`, `--non-interactive`,
  and `--no-path` behavior
- keep the same manual shell integration fallback when the target cannot be
  determined or edited

This should apply to both the generated installer and `pv setup`, because both
can create or repair the `PV ENV` shell block.

### Boundaries

Do not turn this into broad shell-profile discovery.

PV should not scan and mutate several files such as `.zshrc`, `.zlogin`, and
`.profile`. The existing design rule still matters: PV edits one detected shell
profile file and keeps the mutation clearly delimited inside the `PV ENV` block.

Do not add completion installation as part of this. Completions remain explicit
through `pv completions <shell>`.

## Agent-Friendly CLI Contracts

PV should stay friendly to AI agents and automation by improving its existing
CLI contracts, not by becoming an AI tool manager.

The direction for now is:

- keep expanding reliable `--json` output where it is useful
- make JSON stdout quiet and machine-readable
- keep warnings, repair hints, and human presentation out of JSON stdout
- improve existing commands instead of adding a dedicated agent context command
- do not add an MCP server yet
- do not manage Codex, Claude, OpenCode, API keys, model selection, or agent
  installs

Current PV already has useful machine-readable surfaces:

```sh
pv project:env --json
pv list --json
pv status --json
pv jobs --json
pv mysql:list --json
pv postgres:list --json
pv redis:list --json
pv rustfs:list --json
```

That is enough of a foundation for now. If agents need more context later, the
first move should be to improve the existing command shapes and schemas before
adding new AI-specific product surface.

### Non-Goals

Avoid an agent-specific tool registry.

In this context, "agent-specific tool registry" means PV keeping a catalog of
known AI tools such as Codex, Claude, OpenCode, Cursor agents, or MCP servers,
then exposing install commands, capabilities, binary paths, API key setup, model
configuration, or tool manifests for those agents.

That would pull PV away from its local development control-plane job. PV should
make project/resource state easy for any tool to read, but it should not become
the owner of the agent ecosystem.

## Project TLS Placeholders

Status: accepted into `DESIGN.md`. PV should expose stable Project TLS file
paths as env placeholders instead of building framework-specific integrations
for every frontend tool.

The first version should expose only primary-hostname TLS material:

```yaml
env:
  VITE_DEV_SERVER_KEY: "${tls_key}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
  PV_TLS_CA: "${tls_ca}"
```

The concrete placeholders are:

- `${tls_key}`: path to the Project primary-hostname TLS private key
- `${tls_cert}`: path to the Project primary-hostname TLS certificate chain
- `${tls_ca}`: path to PV's local CA certificate

`${tls_ca}` should point only to the CA certificate. PV must never expose the CA
private key through Project env placeholders.

### Product Shape

This keeps PV's side simple:

- PV owns creating or exporting stable certificate files for the Project's
  primary hostname.
- env rendering exposes those file paths.
- users map the placeholders into whatever their frontend or dev server
  expects.

PV should not create first-party plugins for Vite, Rspack, Webpack, Laravel Mix,
or every other JavaScript dev server. Laravel's Vite integration can already
read env-style key/cert paths when users map them. Other tools can use small
config snippets in their own config files.

This is the right level of abstraction: PV provides trusted local TLS material;
the application decides how its dev tooling consumes it.

### Important Caveat

Do not make Caddy/FrankenPHP's internal certificate storage part of PV's public
contract.

The current design says the Gateway uses PV's local CA and Caddy/FrankenPHP
generates Project certificates as needed. For placeholders, PV needs a deliberate
export or generation path with stable filenames under PV-owned storage, such as
a future `~/.pv/certificates/projects/...` layout.

The exact storage path, whether stable files are symlinks or exported copies,
and when reconciliation forces or refreshes certificate material can wait for
implementation design. The env contract should be stable from the user's point
of view.

### Boundaries

This should not turn into:

- automatic edits to `vite.config.*`
- automatic edits to `webpack.mix.js`
- frontend build tool detection
- JS dev server process management
- JS dev server path-proxying through the Gateway
- wildcard certificate or wildcard routing support in the first version
- a PV plugin ecosystem for frontend TLS

If a Project wants custom behavior, it can use normal env mappings and its own
config.

## PHP Extension Profiles

PV should not support arbitrary user-loaded PHP `.so` files, but PV-managed
optional shared extensions look promising.

The current v1 design deliberately avoids PHP extension management. PHP and
FrankenPHP artifacts use a fixed compiled-in extension set, no `phpize`, no
PECL, no dynamic extension installation, and one build flavor per PHP track.
This idea would be a later design pass that reopens that decision carefully.

### Product Direction

PV owns the extension modules.

Users should not download random `.so` files, point PV at them, or compile
extensions locally. If PV supports Xdebug, Imagick, or another optional
extension, PV should build, publish, validate, and smoke-test that module.

PV then enables extensions by generating ini overlays.

For CLI PHP, the PV `php` shim already controls ini discovery with `PHPRC` and
`PHP_INI_SCAN_DIR`. A future extension system could append a PV-generated
profile `conf.d` directory to that scan path.

For FrankenPHP, PV would start a worker process for the required PHP track plus
extension profile. Since PHP extensions load at process startup, changing the
enabled extension set means reloading or restarting the affected worker group.

Example generated Xdebug ini:

```ini
zend_extension=/Users/me/.pv/resources/php/8.4/releases/8.4.22-pv1/modules/xdebug.so
xdebug.mode=debug,develop
xdebug.client_host=127.0.0.1
```

Normal extensions would use `extension=...`; Zend extensions such as Xdebug use
`zend_extension=...`.

### Activation Models

Two user-facing activation models seem useful.

Track-global toggle:

```sh
pv php:extension enable xdebug --track 8.4
pv php:extension disable xdebug --track 8.4
```

This is simple and probably good enough for some users, but it affects every
CLI command and every Project worker using that PHP track.

Project/runtime profile:

```yaml
php: "8.4"
php_extensions:
  - xdebug
```

or:

```yaml
php: "8.4"
php_profile: debug
```

Internally, PV can turn that into separate runtime groups:

```text
php-8.4
php-8.4+xdebug
```

That gives Project-level process isolation without a VM or container. A Project
using Xdebug routes to the Xdebug-enabled FrankenPHP worker, and the `php` shim
inside that Project uses the same ini overlay for CLI commands.

### Packaging Options

The packaging choice can stay open for now.

Bundled optional modules:

```text
~/.pv/resources/php/8.4/releases/8.4.22-pv1/
  bin/php
  modules/xdebug.so
  modules/imagick.so
```

This is the simplest model for a small curated set, especially Xdebug.

Separate extension artifacts:

```text
php-extension:xdebug:8.4
php-extension:imagick:8.4
```

This may be better for larger extensions, extensions with native dependency
surfaces, or extensions that update on a different cadence from PHP itself. The
hard requirement is that each extension artifact is tied to the exact PHP patch
version, platform, ZTS mode, and PV build revision it was built against.

Compiled-in optional extensions are less attractive for toggles. They are fine
for the default fixed extension set, but not for Xdebug-style opt-in behavior.

### Boundaries

Avoid:

- arbitrary user-provided `.so` loading
- local extension compilation
- `phpize` and `php-config` as product features
- PECL installation as product behavior
- per-Project custom ini as a broad general feature
- CLI and browser extension drift for the same Project/runtime profile

PV should keep the default PHP track clean and boring. Optional extensions
should be opt-in and visible in status output.

### Open Questions

- Is Xdebug important enough for v1, or should this stay post-v1?
- Should the Project config key be `php_extensions`, `php_profile`, or both?
- Should track-global extension toggles exist, or should PV only expose
  Project/runtime profiles?
- Should optional modules be bundled into the PHP artifact first, or published
  as separate extension artifacts from day one?
- How should `pv status`, `pv list`, and `pv php:list` display active extension
  profiles?

## Auto-Reconcile Action Commands

PV should consider auto-reconciling before action commands after v1.

Current design keeps dashboard/open commands read-only. That is a good v1
boundary, because it keeps command behavior obvious while the daemon and
resource lifecycle are still settling.

Post-v1, the UX can get smoother. If a user asks PV to do an action that needs a
Project or Resource runtime, PV can reconcile the relevant desired state first,
wait for readiness, then perform the action.

Possible commands:

```sh
pv open
pv mailpit:open
pv mail:open
pv rustfs:open
pv s3:open
pv mysql:shell
pv postgres:shell
pv pg:shell
```

This means a stopped-but-linked Project could come back up when the user runs
`pv open`, and a demanded Mailpit/RustFS runtime could start before opening the
dashboard.

### Product Shape

This should apply to action commands only.

Good candidates:

- open a Project URL
- open a Resource dashboard
- enter an interactive Resource shell
- maybe tail logs for a specific Project or Resource if the runtime is demanded

Bad candidates:

- `pv status`
- `pv list`
- `pv doctor`
- `pv jobs`
- `pv *:list`
- JSON/status commands in general

Read/status commands should stay observational. They should report current
state, not mutate it.

If the daemon is running, the action command can request the narrowest useful
reconciliation scope, wait for completion, then continue. If the daemon is not
running, it should fail clearly or suggest `pv setup` / `pv daemon:restart`
rather than silently starting system integrations.

### Boundaries

Avoid making every command magical.

Auto-reconcile should not:

- run Project hooks
- install unrelated default resources
- start resources that no linked Project demands
- mutate Project config
- hide reconciliation failures
- make `--json` commands perform surprise mutations

If a runtime cannot become ready, the command should show the failed
reconciliation result and point to logs/doctor output.

## Project Details Command

PV should probably add a focused Project details command.

`pv list` is broad and compact. `pv status` is whole-system and explicitly not
scoped to the current Project. `pv project:env` prints generated env values,
including secrets, and should stay focused on that job.

There is still a useful gap:

```sh
pv project:status
pv project:status acme.test
pv project:status --json
```

The command should show what PV believes about one Project, resolving from the
current directory by default and accepting a hostname argument when provided.

Possible output shape:

```text
Project: acme.test
Path: /Users/me/Code/acme
Config: pv.yml
PHP: 8.4
Document root: public
URLs:
  https://acme.test
  https://api.acme.test
Serving: running via worker php-8.4
Env: current
Resources:
  postgres 18  app      ready
  redis 8.8    cache    ready
  mailpit 1    mail     ready
Logs:
  pv logs --worker 8.4
  pv logs --resource postgres --track 18
```

This should be read-only. It should not reconcile, start resources, rewrite
env, install artifacts, or run hooks.

### Boundaries

Do not print secrets.

If users need actual generated env values, the explicit command remains:

```sh
pv project:env
```

`project:status --json` should be stable and agent-friendly, but still avoid
secrets. It can include Project identity, path, config status, resolved PHP
track, document root, hostnames, env observed state, resource demand/allocation
status, relevant runtime subjects, and useful log command hints.

This is likely v1-worthy because it makes PV's Project model visible without
adding new lifecycle behavior.

## Gateway Unknown Hostname Page

PV should make the Gateway 404 page nicer before or soon after v1.

The current design already says unknown `.test` hostnames should return a simple
self-contained HTML response explaining that no PV Project is linked for the
hostname. This is worth treating as product polish rather than leaving the
Gateway fallback as a generic "running" response.

When a user visits an unlinked hostname such as `https://whatever.test`, PV can
use the page to reinforce the routing model:

- DNS catches `.test`
- the Gateway is running
- no linked Project owns this hostname yet

The page should stay small and static. It should not become a dashboard, daemon
control UI, or secret-bearing status page.

Useful content:

- the requested hostname
- `pv link --hostname <hostname>`
- `pv list`
- `pv open`
- maybe known linked Projects if the generated Gateway config can include them
  safely
- a more specific missing-Project message when a hostname belongs to a linked
  Project whose path no longer exists

This belongs to Gateway UX, not Project config. It should not add new hostname
rules, wildcard routing, dashboard routes, or arbitrary local domains.

## LAN Project Access

PV should support local network access to selected Projects after v1, likely in
v1.1.

This is separate from public tunneling. LAN access is for phones, tablets, and
other devices on the same trusted local network. Tunneling through Cloudflare,
frp, ngrok-style services, or PV-hosted sharing can wait until later, maybe v2.

The current v1 design intentionally binds Gateway, Project workers, DNS, and
backing Managed Resources to loopback. That should remain the safe default.
LAN access should be explicit and scoped.

Possible command shape:

```sh
pv lan acme.test
pv lan
pv lan:stop acme.test
pv lan:list
```

The command should expose one selected Project, not every linked Project and not
the whole PV stack.

### Product Shape

The simplest useful version is probably a per-Project LAN listener:

```text
http://192.168.1.25:48123
```

PV can allocate a high port, bind that listener on the chosen LAN interface, and
route all traffic for that listener to the selected Project internally. The
phone does not need to resolve `.test`, install PV's CA, or send a special Host
header.

Internally, the LAN listener can still proxy to the normal Project route using
the Project's primary hostname as the upstream Host. From the user's point of
view, they get a simple LAN URL.

This avoids requiring users to:

- configure router DNS
- point a phone at PV's DNS resolver
- install the PV local CA on the phone
- use a public tunnel for same-network testing

HTTPS can come later. Plain HTTP on a private LAN is probably acceptable for the
first version, as long as the command is explicit about what is being exposed.

### Boundaries

LAN access should not expose:

- backing Managed Resource ports
- Mailpit or RustFS dashboards unless explicitly requested later
- every linked Project by default
- the PV daemon socket or control plane
- public internet tunnels

PV should show the active LAN URL clearly and make stopping it obvious.

The feature should also handle common rough edges:

- multiple network interfaces
- changing Wi-Fi IP addresses
- port conflicts
- macOS firewall prompts
- sleeping/waking laptops
- clear status output for active LAN shares

Possible later polish:

- QR code output for mobile testing
- interface selection
- temporary expiry
- optional allowlist or one-time token
- HTTPS after the certificate/trust story is designed

Public tunneling should be a separate idea. It needs provider accounts, public
exposure warnings, auth, rate limits, and abuse considerations.

## Resource-Only Projects

PV could support Projects that use PV-managed resources without being served by
the Gateway.

This would let non-PHP and non-Laravel Projects use PV for local infrastructure
such as Postgres, MySQL, Redis, Mailpit, or RustFS while running their own app
server. For example, a Next.js Project could use PV for Postgres and still run:

```sh
pnpm dev
```

PV would own the backing resources and env rendering. The framework's own dev
server would remain user-owned.

Example:

```yaml
serve: false

postgres:
  version: "18"
  allocations:
    app:
      env:
        DATABASE_URL: "postgresql://${username}:${password}@${host}:${port}/${database}"

redis:
  allocations:
    cache:
      env:
        REDIS_URL: "redis://${host}:${port}"
```

### Product Shape

`serve: false` means the Project is linked and reconciled, but PV does not create
a Gateway route or try to serve the Project at a `.test` hostname.

`pv link` for a resource-only Project should:

- register the directory as a Project
- install and start requested Managed Resource tracks
- create Resource allocations
- render configured env values
- skip Gateway route generation for that Project
- show the Project as resource-only in status/list output

PV should not try to run `next dev`, `pnpm dev`, Rails, Go, Python, or other app
servers in the first version of this idea. That is a separate process
orchestration problem.

### Project Slug

Resource-only Projects should not need fake `.test` hostnames just to get stable
Resource allocation names.

Instead, PV should assign a stored Project slug when the Project is first linked.
The default slug is derived from the Project directory basename:

```text
/Users/me/Code/appointment -> appointment
```

If the slug already exists in PV state, PV assigns the next available suffix:

```text
appointment
appointment-1
appointment-2
```

PV should store the assigned slug in `pv.db` and never change it automatically,
even if the directory is later renamed. This keeps generated resource names
stable.

The assigned slug is used as the readable prefix for generated Resource
allocation names:

```text
appointment_app       # SQL database name
appointment_1_app     # SQL database name when slug is appointment-1
appointment-cache-    # Redis prefix
appointment-uploads   # RustFS bucket
```

SQL resource names convert hyphens to underscores. Redis prefixes and RustFS
buckets keep DNS-style hyphens.

Collision checks should happen against PV state, not by scanning the underlying
database, Redis, or object storage directly. Underlying resource objects may
include old or orphaned data that PV deliberately preserves.

### Served Projects

Served Projects can keep using their Project hostname as the user-facing label
and resource-name prefix for now.

The Project slug idea may later be unified across all Projects, but the first
reason to introduce it is resource-only Projects where a hostname would be fake.

### Open Questions

- Should served Projects also get a stored Project slug, or only resource-only
  Projects?
- Should `pv list` show both Project hostname and Project slug when both exist?
- Should resource-only Projects support `pv open`, or should `pv open` clearly
  say that the Project is not served by PV?
- Should env rendering still default to `.env`, or should resource-only Projects
  support an `env_file` key such as `.env.local`?

## Appendix: Parking Lot / Maybes

These are ideas that seem useful, but are not yet strong enough to treat as a
product direction. Promote them only if user feedback, support pain, or PV's own
development experience proves the need.

### Component-Specific Doctor Commands

PV already has `pv doctor` in the v1 design as a deeper read-only diagnostic for
setup, DNS, ports, CA, daemon, Gateway, manifest cache, and common conflicts.

The maybe is a more focused diagnostic family for specific components:

```sh
pv dns:doctor
pv daemon:doctor
pv php:doctor
pv gateway:doctor
pv resource:doctor mysql
```

Competitor research shows users often struggle when a broad "something is
broken" status does not identify the exact layer that failed. DDEV's focused
diagnostic commands are the strongest example here.

This should stay parked for now. Add it only if the broad `pv doctor` output
gets too noisy, users repeatedly ask for narrower checks, or PV development
itself needs component-level debugging commands.

If added later, these commands should stay read-only and actionable. They should
suggest repair commands instead of mutating privileged system state or restarting
processes automatically.

### Project Command Sandboxing

PV could eventually offer a guarded command runner for risky Project commands,
but this is a far-future maybe rather than a near-term product direction.

The motivating problem is real: AI agents, package manager scripts, and
compromised dependencies can be destructive when they run with normal user
access. A command such as `pnpm install`, `composer install`, or an AI agent
session can read dotfiles, cloud credentials, SSH keys, and unrelated Projects
unless something constrains it.

The PV-shaped version would be generic, not AI-specific:

```sh
pv sandbox run -- pnpm install
pv sandbox run -- composer install
pv sandbox run -- codex
pv sandbox shell
```

Potentially, hooks could opt into the same execution mode later:

```yaml
sandbox: true

hooks:
  setup:
    - pnpm install
```

The honest first version would only promise damage reduction, not perfect
supply-chain security. It might restrict filesystem access to the Project, PV
shims, required runtime artifacts, and explicitly allowed cache directories.
Broad claims like "safe npm install" would be wrong unless PV also controls
network access, credentials, and exfiltration paths.

For macOS, a lightweight native implementation might use Apple's Seatbelt
sandboxing through `sandbox-exec`, but that tool is deprecated and the profile
surface is not a great product foundation. A stronger version would require a
VM or microVM/container boundary, which is much closer to a separate product
than a small PV feature.

Keep this parked unless user demand is strong or PV's hook runner creates enough
risk that a first-party guardrail becomes necessary.

### Worktree Environment Commands

PV does not need first-class Git worktree commands right now.

The existing product answer should be:

```sh
pv link
```

Each Git worktree is just another directory. Since PV identifies Projects by
canonical absolute path and requires unique Project hostnames, linking a
worktree naturally creates a separate Project with its own hostname and Resource
allocations.

This keeps PV out of Git-specific workflows and avoids adding command surface
such as:

```sh
pv worktree:link
pv project:clone
```

Those would mostly duplicate `pv link`.

The one useful bit of polish to keep in mind is collision UX. If a hostname was
auto-derived from the directory name and already exists, PV could eventually
suggest or assign the next available name. If the user explicitly passed
`--hostname`, collisions should keep failing hard.

Do not promote this into a feature unless users repeatedly struggle to link
multiple checkouts of the same app.

### `.localhost` Project Hostnames

PV should stay `.test`-first.

The current design intentionally rejects non-`.test` Project hostnames and
wildcards. That keeps DNS, Gateway routing, certificates, docs, and user mental
models much simpler.

Revisit `.localhost` only if real OAuth/provider workflows complain about
`.test` callback URLs. Google or similar providers may be more willing to accept
`localhost`-style redirect URIs than arbitrary local TLDs. If that becomes a
support issue, PV could consider allowing explicit `.localhost` Project
hostnames as a narrow compatibility escape hatch.

This should not become broad custom domain support:

- no arbitrary local TLDs
- no wildcard Project routing
- no routing real user-owned domains
- no per-team domain policy system
- no extra resolver suffixes unless there is a concrete provider problem

If added later, `.localhost` should follow the same explicit hostname collision,
certificate, Gateway, and `hostnames:` rules as `.test`.

### Targeted Runtime Restart

PV already has the broad product shape of `pv restart`: restart PV-managed
runtime processes and reconcile desired state.

The maybe is allowing `pv restart` to target one PV-owned runtime instead of
restarting the whole local stack.

Possible shape:

```sh
pv restart
pv restart mysql
pv restart mysql 8.0
pv restart postgres 18
pv restart pg 18
pv restart redis
pv restart mailpit
pv restart rustfs
pv restart php 8.4
pv restart gateway
```

This should stay under `pv restart` rather than adding separate commands such
as `pv mysql:restart` or `pv postgres:restart`. The resource name remains
explicit, but the verb stays in one obvious place.

Useful when a single local runtime is degraded or weird and the user wants the
equivalent of "restart Redis" without bouncing every Project and Resource.

Boundaries:

- restart only PV-owned runtime processes after ownership verification
- stream daemon job progress and fail if readiness fails
- no Project hooks
- no Project config mutation
- no privileged system repair
- no generic `db` target
- if the daemon is down, suggest `pv daemon:restart` or `pv setup`

Avoid `pv restart <project>` until the semantics are clear. A Project can share
a PHP worker with other Projects, so "restart this Project" may not map cleanly
to one runtime process.
