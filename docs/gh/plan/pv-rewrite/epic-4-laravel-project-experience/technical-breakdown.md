# Technical Breakdown: Epic 4 - Laravel Project Experience

## Module Roles

| Module | Responsibility |
| --- | --- |
| `internal/project` | `pv.yml` schema, parser, validation, project identity, and current-project resolution. |
| `internal/resources/laravel` | Laravel detection, defaults, deterministic contract generation, Laravel-specific helper behavior. |
| `internal/app` | Use cases for init, link, open, artisan, db, mail, and s3 commands. |
| `internal/projectenv` | Managed env parsing, rendering, backup, and merge behavior. |
| `internal/setup` | Ordered setup command execution with managed PATH and streaming output. |
| `internal/resources/gateway` | Gateway desired/observed state, route rendering, and process definition. |
| `internal/host` | DNS, TLS, browser, file, and process adapters. |

## `pv.yml` MVP Schema

The rewrite MVP uses this contract shape:

```yaml
version: 1
php: "8.4"
aliases: []
env:
  APP_URL: "{{ .site_url }}"
postgresql:
  version: "18"
  env:
    DB_CONNECTION: pgsql
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
setup:
  - cp .env.example .env
  - composer install
  - php artisan key:generate
```

Rules:

- `version: 1` is required for new rewrite contracts.
- `php` is required for Laravel contracts.
- Service keys are `postgresql`, `mysql`, `redis`, `mailpit`, and `rustfs`.
- `setup` is an ordered list of shell command strings.
- Each setup command runs in its own shell from the project root.
- Pinned managed PHP is prepended to `PATH` before setup commands run.
- `pv link` never adds setup commands that are not declared.

## Link Flow

1. Resolve and validate nearest `pv.yml`.
2. Reject unsupported version or unknown fields.
3. Record project desired state before daemon signal.
4. Resolve declared resource env providers.
5. Render only declared env keys.
6. Back up `.env` before mutation.
7. Update only pv-managed env entries and preserve user-authored lines.
8. Run declared setup commands in order and fail fast on first non-zero exit.
9. Signal daemon after durable project state is written.

## Gateway Flow

1. Create gateway desired state from linked project host, aliases, project path, and runtime reference.
2. Render deterministic FrankenPHP/Caddy route config.
3. Create or locate TLS material with SANs for primary host and aliases.
4. Apply DNS through host adapter.
5. Submit gateway process definition to supervisor.
6. Record route/gateway observed status for aggregate status.

## Helper Flow

1. Resolve current project.
2. Load durable project state.
3. Validate required declared resource or runtime exists.
4. Invoke the resource capability or managed PHP command.
5. Keep stdout/stderr behavior scriptable.

## Non-Goals

- No `.env` service inference.
- No hidden setup commands.
- No auto-creation of missing helper resources.
- No real DNS/TLS/browser mutation in unit tests.
