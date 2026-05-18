# Technical Breakdown: Epic 3 - Runtime, Daemon, And Resources

## Module Roles

| Module | Responsibility |
| --- | --- |
| `internal/resources/php` | PHP desired state, install integration, runtime paths, shims, status. |
| `internal/resources/composer` | Composer desired state, PHP dependency checks, shim behavior, blocked status. |
| `internal/control` | Daemon reconcile loop, controller dispatch, durable wake/signal behavior. |
| `internal/supervisor` | Process definitions, start/stop/check/readiness/log path/restart budget. |
| `internal/resources/mailpit` | Mailpit desired state, ports, readiness, env values, process definition. |
| `internal/resources/postgres` | Postgres version-line state, data/log paths, process definition, env values, database commands. |
| `internal/resources/mysql` | MySQL version-line state, init, socket/PID, privileges, process definition, env values, database commands. |
| `internal/resources/redis` | Redis version-line state, flags, data/log paths, readiness, env values. |
| `internal/resources/rustfs` | RustFS version state, credentials, API/console ports, S3 env values, redacted status. |

## Command Contracts

| Command | Behavior |
| --- | --- |
| `php:install <version>` | Writes PHP runtime desired state only. |
| `composer:install <version> --php <php-version>` | Writes Composer desired state with required managed PHP version. |
| `mailpit:install <version>` | Writes Mailpit desired state. |
| `postgres:install <major>` | Writes Postgres version-line desired state. |
| `mysql:install <version>` | Writes MySQL version-line desired state. |
| `redis:install <version>` | Writes Redis version-line desired state. |
| `rustfs:install <version>` | Writes RustFS desired state. |
| `postgres:db:create <name>` / `postgres:db:drop <name>` / `postgres:db:list` | Explicit Postgres database commands. |
| `mysql:db:create <name>` / `mysql:db:drop <name>` / `mysql:db:list` | Explicit MySQL database commands. |

Commands do not start long-running processes directly and do not infer project services from `.env`.

## Resource State Requirements

| Resource | Required observed states |
| --- | --- |
| PHP | missing install, pending, ready, failed. |
| Composer | missing runtime, blocked, pending, ready, failed. |
| Mailpit | missing install, stopped, running, crashed, restart-exhausted, failed. |
| Postgres | missing install, stopped, running, blocked, failed. |
| MySQL | missing install, stopped, running, blocked, failed. |
| Redis | missing install, stopped, running, crashed, failed. |
| RustFS | missing install, stopped, running, crashed, failed with redacted credentials. |

## Supervisor Boundary

The supervisor accepts process definitions and reports lifecycle facts. It must not mention PHP, Composer, Mailpit, Postgres, MySQL, Redis, RustFS, Laravel, or gateway in its public API or tests.

## Env Provider Requirements

- Env providers expose values only from declared desired state and observed resource facts.
- Env providers do not inspect project `.env` files.
- RustFS secret values are available to render declared env keys but are redacted from status and logs.

## Non-Goals

- No Laravel project contract parsing.
- No gateway route rendering.
- No worker, queue, or scheduler supervision.
- No expensive artifact workflows by default.
