# pv.yml as explicit project config — drop the auto-wiring

**Status:** Draft
**Date:** 2026-05-10

## Background

`pv link` today runs two automation steps that silently mutate a project's `.env`:

1. **`DetectServicesStep`** (`internal/automation/steps/detect_services.go`) scans the project's existing `.env` for hints:
   - `DB_CONNECTION=pgsql` → bind the highest-installed Postgres major
   - `DB_CONNECTION=mysql` → bind the highest-installed MySQL
   - Laravel project + Redis installed → bind Redis unconditionally
   - `MAIL_HOST` contains `localhost`/`127.0.0.1` → bind mailpit
   - `AWS_ENDPOINT` contains `localhost`/`127.0.0.1` → bind rustfs

2. **`laravel.DetectServicesStep`** (`internal/laravel/env.go`) then writes Laravel-shaped env keys back into `.env` via `MergeDotEnv`:
   - `DB_HOST=127.0.0.1`, `DB_PORT=…`, `DB_DATABASE=<projectName>`, `DB_USERNAME=root`, `DB_PASSWORD=""`
   - "Smart Laravel vars" — `CACHE_STORE=redis`, `SESSION_DRIVER=redis`, `QUEUE_CONNECTION=redis`, `FILESYSTEM_DISK=s3`, `MAIL_MAILER=smtp` — flipped based on which services were bound

In parallel, the rest of the `pv link` pipeline runs a hardcoded sequence of project-setup steps that the user cannot disable, reorder, or extend: `CopyEnvStep`, `ComposerInstallStep`, `GenerateKeyStep`, `InstallOctaneStep`, `CreateDatabaseStep`, `RunMigrationsStep`, `SetAppURLStep`, `SetViteTLSStep`.

This whole model has three concrete problems:

- **Silent mutation of `.env`.** The user can't tell which keys are pv-managed and which are theirs. Editing a pv-managed key gets clobbered on next link.
- **Laravel-shaped assumption with no escape hatch.** Key names are hardcoded. Non-Laravel projects, projects with renamed connections, and multi-tenant projects with prefixed keys can't be supported.
- **Fragile triggers.** Auto-detect relies on `.env` already containing the right hints. A fresh project, a project with a renamed `DB_CONNECTION`, or a project that derives DB config from another source gets nothing wired.

A real-world break: a Laravel project with a custom `php artisan x-migrate` command that handles multi-database setup. pv's hardcoded `RunMigrationsStep` runs `migrate`, not `x-migrate`. pv's `CreateDatabaseStep` creates one database when the project needs several. The user has no way to opt out short of monkey-patching pv.

The fix is to make pv.yml the single source of truth for what a project declares about itself, and to stop pv from doing anything to the project that wasn't asked for.

## Goals

1. pv.yml is the contract between a project and pv. Nothing about a project's setup is inferred at runtime; everything is declared.
2. The user picks env-var names. pv supplies values via templates (e.g., `{{ .host }}`, `{{ .port }}`, `{{ .password }}`).
3. The user owns the project-setup pipeline. `pv link` runs a list of shell commands the user wrote; pv does not pick what to run.
4. pv still owns infrastructure: PHP binary, service supervision, Caddy site block, TLS certs, DNS. The user does not configure those.
5. A migration tool, `pv init`, generates a sensible default pv.yml per project type. The magic that used to live in `DetectServicesStep` becomes scaffolding the user reviews and commits, not runtime behavior.

## Non-goals

This spec covers only the env-wiring and link-pipeline redesign. The following are deferred to future specs (the schema leaves room but does not implement them):

- Per-project PHP extensions list. Requires changes to the `static-php-cli` build set and a php.ini per-project model.
- Per-project Xdebug toggle with trigger mode.
- Worker / queue / scheduler supervision. The community case is real but small; users run `php artisan queue:work` themselves via `concurrently` or similar.
- A generic command runner (`pv run <name>`). Composer scripts already cover this for PHP projects.
- MariaDB as a first-class service. The schema may reserve the key; implementation is its own spec.
- LAN sharing / mobile device access (separate feature).
- Custom Caddy snippets per project (Magento/WordPress).
- Per-project php.ini settings beyond extension toggles.

## Schema

The full top-level shape of pv.yml after this change. Every key except `php:` is optional.

```yaml
php: 8.4

aliases:
  - admin.myapp.test
  - api.myapp.test

env:
  APP_URL: "{{ .site_url }}"
  APP_NAME: "MyApp"
  SANCTUM_STATEFUL_DOMAINS: "{{ .site_host }}"
  VITE_DEV_SERVER_KEY: "{{ .tls_key_path }}"
  VITE_DEV_SERVER_CERT: "{{ .tls_cert_path }}"

postgresql:
  version: 18
  env:
    DB_CONNECTION: pgsql
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
    DB_USERNAME: "{{ .username }}"
    DB_PASSWORD: "{{ .password }}"

mysql:
  version: 8.0
  env:
    DB_CONNECTION: mysql
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
    DB_USERNAME: "{{ .username }}"
    DB_PASSWORD: "{{ .password }}"

redis:
  env:
    REDIS_HOST: "{{ .host }}"
    REDIS_PORT: "{{ .port }}"

mailpit:
  env:
    MAIL_HOST: "{{ .smtp_host }}"
    MAIL_PORT: "{{ .smtp_port }}"

rustfs:
  env:
    AWS_ENDPOINT: "{{ .endpoint }}"
    AWS_ACCESS_KEY_ID: "{{ .access_key }}"
    AWS_SECRET_ACCESS_KEY: "{{ .secret_key }}"
    AWS_USE_PATH_STYLE_ENDPOINT: "{{ .use_path_style }}"

setup:
  - cp .env.example .env
  - composer install
  - php artisan key:generate
  - php artisan x-migrate
  - bun install
  - bun run build
```

### Schema notes

- `aliases:` — extra `.test` hostnames that resolve to this project. pv mints TLS certs for them automatically; they appear as SANs in the same Caddy site block as the primary host.
- `env:` (top-level) — env keys the user wants pv to write into `.env`. Values may be plain strings or Go templates referencing project-level template vars (`{{ .site_url }}`, `{{ .site_host }}`, `{{ .tls_cert_path }}`, `{{ .tls_key_path }}`).
- `<service>:` — a service block declares a binding. `version:` is required for `postgresql` and `mysql` (multiple versions can coexist on one machine); it is implicit for `redis`, `mailpit`, `rustfs` (pv ships one bundled version at a time). A service block with no `env:` map is valid: it declares pv should supervise the service for this project but writes no env keys.
- `setup:` — a list of shell commands. Each command runs from the project root, with the pinned PHP version on PATH (existing per-directory shim), inheriting the project's `.env` as it stands after env templating. Execution is fail-fast: the first non-zero exit aborts the rest.

### Database and bucket names are user-controlled

`pv link` does **not** create databases or S3 buckets. There is no `database:` or `bucket:` field on service blocks. Users who want a database or bucket created at link time call pv's standalone commands from their `setup:`:

```yaml
setup:
  - pv postgres:db:create my_app
  - pv s3:bucket:create my-uploads
  - composer install
  - php artisan migrate
```

This is the answer to the multi-database use case (e.g., `x-migrate` projects): pv exposes the capability, the user composes it however they want.

## Template variables

### Project-level (top-level `env:`)

| Variable | Value | Source |
|---|---|---|
| `.site_url` | `https://<project>.test` | pv-assigned site URL for the primary host |
| `.site_host` | `<project>.test` | pv-assigned hostname (no scheme) |
| `.tls_cert_path` | absolute path to the pv-issued `.pem` | pv local CA |
| `.tls_key_path` | absolute path to the pv-issued key | pv local CA |

### Postgres (`postgresql.env`)

| Variable | Value |
|---|---|
| `.host` | `127.0.0.1` |
| `.port` | pv-allocated port for the requested major version |
| `.username` | default `postgres` |
| `.password` | default `postgres` |
| `.version` | full version string (e.g. `18.1`) |
| `.dsn` | `postgresql://<user>:<pass>@<host>:<port>` (no database — pv doesn't pick one) |

### MySQL (`mysql.env`)

| Variable | Value |
|---|---|
| `.host` | `127.0.0.1` |
| `.port` | pv-allocated port for the requested version |
| `.username` | default `root` |
| `.password` | default empty |
| `.version` | full version string |
| `.dsn` | `mysql://<user>:<pass>@<host>:<port>` |

### Redis (`redis.env`)

| Variable | Value |
|---|---|
| `.host` | `127.0.0.1` |
| `.port` | pv-allocated |
| `.password` | empty in dev |
| `.url` | `redis://<host>:<port>` |

### Mailpit (`mailpit.env`)

| Variable | Value |
|---|---|
| `.smtp_host` | `127.0.0.1` |
| `.smtp_port` | `1025` |
| `.http_host` | `127.0.0.1` |
| `.http_port` | `8025` |

### RustFS / S3 (`rustfs.env`)

| Variable | Value |
|---|---|
| `.endpoint` | `http://127.0.0.1:<port>` |
| `.access_key` | pv-generated |
| `.secret_key` | pv-generated |
| `.region` | `us-east-1` |
| `.use_path_style` | `true` |

## Runtime behavior

### `pv link`

After this change, `pv link` runs the following ordered steps. The list is final — there is no auto-detect path, no fallback heuristic.

1. **Resolve PHP version** from pv.yml `php:`. If not installed, run `pv php:install <version>` (existing behavior).
2. **Start declared services.** For each service block in pv.yml, ensure it's installed at the requested version. If not installed, `pv link` errors with a clear message pointing at `pv <service>:install <version>`. No silent installs.
3. **Render env templates.** Top-level `env:` resolves against project-level vars; per-service `env:` resolves against service-specific vars. Values that aren't templates are passed through as literal strings.
4. **Merge into `.env`.** Rendered keys are written to the project's `.env` via the existing `MergeDotEnv`. A `.pv-backup` is written before any modification (current behavior, retained). Keys not declared in pv.yml are not touched.
5. **Wire Caddy site.** The primary host + every alias gets a SAN entry in one Caddy site block. pv mints a cert for each.
6. **Run `setup:` commands.** Each command runs in order, fail-fast, from the project root, with pinned PHP on PATH. The first non-zero exit aborts.
7. **Register / update registry. Signal daemon.** (Existing behavior, unchanged.)

### What `pv link` no longer does

- Does not scan `.env` for service hints.
- Does not write any env key that isn't declared in pv.yml.
- Does not copy `.env.example` to `.env` (users add `cp .env.example .env` to `setup:` if they want that).
- Does not run `composer install`, `php artisan key:generate`, `php artisan migrate`, or install Octane. These belong in `setup:`.
- Does not create databases or S3 buckets. Users call `pv postgres:db:create` / `pv s3:bucket:create` from `setup:`.
- Does not set `APP_URL` or Vite TLS env vars implicitly. These are declared in top-level `env:` via templates.
- Does not flip "smart Laravel vars" (`CACHE_STORE`, `SESSION_DRIVER`, `QUEUE_CONNECTION`, `FILESYSTEM_DISK`, `MAIL_MAILER`). Users set those themselves.

### `pv init`

`pv init` is the migration tool. Given a project directory, it detects the project type and writes a sensible default pv.yml.

Detection rules (file presence at the project root):

| Marker | Type |
|---|---|
| `artisan` | Laravel |
| `please` | Statamic |
| `bin/console` | Symfony |
| `composer.json` (and none of the above) | generic PHP |
| `package.json` only | Node |

For Laravel, the generated pv.yml includes:

- `php:` set from `composer.json` `require.php` (if present) or the global default
- A `postgresql:` (or `mysql:`) block with the standard Laravel-shaped `env:` mapping (`DB_CONNECTION`, `DB_HOST`, etc.)
- A top-level `env:` block with `APP_URL: "{{ .site_url }}"`
- A `setup:` block:
  ```yaml
  setup:
    - cp .env.example .env
    - composer install
    - php artisan key:generate
    - php artisan migrate
  ```

Other types get analogous templates. `pv init` refuses to overwrite an existing pv.yml without `--force`.

### Conflict policy

- pv.yml is the source of truth while a key remains declared there. Templated env values overwrite existing `.env` values on every `pv link` and are labeled with `# pv-managed`. A user who edits a currently declared pv-managed key in `.env` will see their edit clobbered on next link; the right place to change that value is pv.yml (either the literal, or by overriding the template result).
- `.pv-backup` is written before any merge (current `MergeDotEnv` behavior).
- Keys not declared in pv.yml are untouched. Comments and blank lines are preserved (current behavior).

### `pv unlink` does not touch `.env`

Unlink is administrative — it removes the project from pv's registry and tears down the Caddy site for it. It does **not** clean up the project's `.env`, drop databases or buckets, or remove pv.yml. Reasons:

- Unlink is often temporary (the project is being archived, moved, or will be re-linked later). Clobbering env values forces the user to re-derive them on next link.
- Env values written by pv are still valid strings; they just point at services that aren't running for this project. A re-link refreshes them from pv.yml anyway.
- Anything destructive must be invoked explicitly. Users who want to drop databases / buckets call `pv postgres:db:drop` etc. themselves; users who want to clear pv-managed env keys remove them from pv.yml, relink to stop future updates, then edit or remove the existing `.env` lines manually.

### Removing a key from pv.yml — managed labels

PR 6 labels keys pv writes from pv.yml with an adjacent comment:

```dotenv
# pv-managed
APP_URL=https://myapp.test
```

Removing a key from pv.yml does not delete the existing `.env` line. It stops future pv updates for that key; the user owns cleanup.

### Service installed check

If pv.yml declares a service version that isn't installed, `pv link` errors:

```
postgresql 18 is declared in pv.yml but not installed.
Run: pv postgres:install 18
```

No silent install. The user opts into binary installs explicitly.

### One version per service per project

A project's pv.yml can declare exactly one version of any given service. Multiple Postgres versions in one project is not supported. Users who run multiple Postgres versions on their machine pick one per project; switching between them is a pv.yml edit + re-link.

## What's removed

Code-level deletions land in PR 5. Exact file paths are verified during implementation; this is the surface as of writing:

| File / step | Disposition |
|---|---|
| `internal/automation/steps/detect_services.go` | Delete. The whole heuristic auto-detect dies. |
| `internal/laravel/env.go` `DetectServicesStep`, `UpdateProjectEnvForMysql`, `UpdateProjectEnvForPostgres`, `UpdateProjectEnvForRedis`, `UpdateProjectEnvForBinaryService`, smart Laravel vars logic | Delete. Connection strings and smart vars are now user-declared. `ApplyFallbacks` survives for explicit uninstall fallback hooks. |
| `internal/automation/steps/copy_env.go` (`CopyEnvStep`) | Delete. Users add `cp .env.example .env` to `setup:`. |
| `internal/automation/steps/composer_install.go` (`ComposerInstallStep`) | Delete. Moves to `setup:`. |
| `internal/automation/steps/generate_key.go` (`GenerateKeyStep`) | Delete. Moves to `setup:`. |
| `internal/automation/steps/install_octane.go` (`InstallOctaneStep`) | Delete. Moves to `setup:`. |
| `internal/automation/steps/create_database.go` (`CreateDatabaseStep`) | Delete. Capability extracted to `pv postgres:db:create` / `pv mysql:db:create`. |
| `internal/automation/steps/run_migrations.go` (`RunMigrationsStep`) | Delete. Moves to `setup:`. |
| `internal/automation/steps/set_app_url.go` (`SetAppURLStep`) | Delete. Declared via top-level `env:` template. |
| `internal/automation/steps/set_vite_tls.go` (`SetViteTLSStep`) | Delete. Declared via top-level `env:` template. |
| `cmd/setup.go` (the `pv setup` command) | No PR 5 change. The command is the interactive setup wizard, not a hardcoded project pipeline runner. |

The `internal/automation/pipeline.go` itself shrinks to just the steps pv owns (resolve PHP, start services, render+merge env, wire Caddy, run user `setup:`).

## Rollout plan

Six PRs, additive then breaking. PRs 1–4 land on `main` individually; each is shippable, and the old pipeline keeps working through them. PR 5 flips the switch. PR 6 polishes.

### PR 1 — pv.yml schema + template engine (parse-only)

Pure additive. No runtime behavior change.

- Extend `internal/config/pvyml.go` with the full schema: `aliases []string`, `env map[string]string`, per-service structs (`Postgresql`, `Mysql`, `Redis`, `Mailpit`, `Rustfs`, each with `Version` where applicable and `Env map[string]string`), `setup []string`.
- New package `internal/projectconfig/template` (name finalized in implementation) with:
  - A Go-template renderer that takes a template string + a map and returns the rendered string.
  - Per-service var-map producers: `PostgresVars(version)`, `MysqlVars(version)`, `RedisVars()`, `MailpitVars()`, `RustfsVars()`, and a project-level `ProjectVars(projectName, host, certPath, keyPath)`.
- Unit tests covering schema parsing of every block type and template rendering of every documented variable.
- `pv link` and `pv setup` do not read the new fields yet.

### PR 2 — `pv link` honors `services:`, `env:`, `aliases:` from pv.yml

- When pv.yml declares one or more service blocks, bind those services for the project (replacing what the auto-detect step would have inferred). Auto-detect remains as a fallback when no service blocks are declared, so existing users are not broken.
- Render top-level `env:` and per-service `env:` against template vars. Merge results into `.env` via `MergeDotEnv`.
- Add `aliases:` support to Caddy site generation: primary host + every alias appear as SANs in one site block, all certed by pv's local CA.
- Tests for: service-block-honored path, env-template-rendering path, alias-cert path, fallback-to-auto-detect path.

### PR 3 — `setup:` runner + standalone db/bucket commands

- Add `pv postgres:db:create <name>` / `pv postgres:db:drop <name>`, `pv mysql:db:create <name>` / `pv mysql:db:drop <name>`, `pv s3:bucket:create <name>` / `pv s3:bucket:drop <name>`. These reuse the logic extracted from `CreateDatabaseStep`. They are useful both inside `setup:` blocks and standalone.
- Implement the `setup:` runner: iterate the command list, exec each via shell with project root as cwd and pinned PHP on PATH, fail-fast on the first non-zero exit code, stream stdout/stderr to the user.
- When a project's pv.yml has a `setup:` block, skip the hardcoded pipeline steps (`ComposerInstallStep` et al.) for that project. When no `setup:` block is present, the hardcoded steps still run (compat).
- Tests for: fail-fast behavior, env propagation, PHP version pinning, db/bucket command idempotency.

### PR 4 — `pv init`

- Implement `pv init` as a new top-level command.
- Project type detection per the rules in this spec (artisan / please / bin/console / composer.json / package.json).
- Per-type pv.yml generation. Laravel template includes the standard `postgresql` (or `mysql`, by flag) block with Laravel-shaped env, top-level `APP_URL: "{{ .site_url }}"`, and a `setup:` block with the four standard commands.
- `--force` to overwrite an existing pv.yml. Default behavior is refuse + error.
- Unit tests per project type. E2E test: fresh Laravel skeleton + `pv init` + `pv link` produces a working project.

### PR 5 — Breaking: delete the magic

This is the cut-over. `pv.yml` becomes mandatory for `pv link`.

- Delete all the files listed in the "What's removed" table above.
- Leave `cmd/setup.go` alone; it is the interactive setup wizard, not a hardcoded project pipeline runner.
- `pv link` without a `pv.yml` errors:
  ```
  no pv.yml found at <project>.
  Run: pv init
  ```
- README + CLAUDE.md updates. Add a migration guide ("if you were on pv ≤ X, run `pv init` in each linked project, review the generated pv.yml, commit it") to README.
- E2E tests updated to cover the new flow. Old auto-detect e2e tests deleted.

### PR 6 — `.env` managed labels

Makes pv.yml-driven `.env` writes visible without hidden state or automatic cleanup.

- Label keys written from pv.yml with an adjacent `# pv-managed` comment.
- On `pv link`, update currently declared pv.yml env keys and preserve/add their labels.
- Do not delete keys that disappear from pv.yml; removing a key from pv.yml stops future pv updates and leaves cleanup to the user.
- Tests for: add key + relink writes the label, update existing key adds or preserves the label, removing a key from pv.yml does not delete it, manually-edited non-pv keys are never touched.

## Open questions

The following are deliberately not nailed down in this spec and are settled inside the relevant PR's implementation:

1. **Exact `pv init` output per project type** beyond Laravel. Decided in PR 4 alongside detection tests.
2. **Whether `pv setup` keeps its own command after PR 5**, or is merged into `pv link --force-rerun-setup`. Decided in PR 5; preserving the command is the lower-risk default.
