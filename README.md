# pv

Local development server manager powered by [FrankenPHP](https://frankenphp.dev).

## Why

Running PHP projects locally shouldn't require juggling dnsmasq, Docker, and Traefik. `pv` replaces all of that with a single binary. Link a project directory and it's instantly available at `https://project.test` — with HTTPS, automatic PHP serving, and its own built-in DNS. No containers, no proxy chains, no config files to maintain.

Currently supports PHP projects (Laravel, generic PHP, static sites).

## What it sets up

`pv install` gets you a complete local PHP environment in one shot:

- **FrankenPHP** — the server (Caddy + embedded PHP, no Apache/Nginx)
- **PHP** — managed per-version, no homebrew/apt needed
- **Composer** — ready to go
- **Mago** — PHP linter/formatter

## Usage

```bash
# Install pv and set up the environment
pv install                # non-interactive
pv setup                  # interactive wizard

# Install a PHP version
pv php:install 8.4

# Link a project — it's now live at https://my-app.test
pv link ~/code/my-app

# Start the server
pv start

# See what's linked
pv list

# Manage the server
pv status
pv stop
pv restart
pv log

# Unlink a project
pv unlink my-app
```

### PHP version manager

`pv` is also a full PHP version manager. Install multiple versions side-by-side and switch between them instantly — no phpenv, phpbrew, or homebrew tap juggling.

```bash
# Install multiple versions
pv php:install 8.3
pv php:install 8.4
pv php:install 8.5

# Switch the global default
pv php:use 8.4

# See what's installed
pv php:list

# Remove a version
pv php:remove 8.3
```

Per-project versions are supported too — add a `pv.yml` file with `php: "8.4"` in your project root or let `pv` read the PHP constraint from `composer.json`. Multiple PHP versions run simultaneously, each project served by its own FrankenPHP process.

`pv init` detects your project type (Laravel, Laravel + Octane, generic PHP, static) and writes a default `pv.yml` you review and commit; `pv link` then reads `pv.yml` and serves the project.

### Tool management

Each managed tool has a consistent set of commands:

```bash
# Download a tool to private storage
pv mago:download

# Expose/remove from PATH
pv mago:path
pv mago:path --remove

# Install (download + expose)
pv mago:install

# Update to latest version
pv mago:update

# Uninstall (remove binary + PATH entry)
pv mago:uninstall
```

This pattern applies to all tools: `php`, `mago`, `composer`.

### Backing services

All backing services run as native binaries supervised by the pv daemon — no Docker, no VM. Each service has its own first-class command group:

```bash
# Databases
pv postgres:install 17
pv postgres:start

pv mysql:install 8.4
pv mysql:start 8.4

pv redis:install
pv redis:start

# Mail (Mailpit) — `mail:*` is an alias for `mailpit:*`
pv mailpit:install
pv mailpit:start

# S3 (RustFS) — `s3:*` is an alias for `rustfs:*`
pv rustfs:install
pv rustfs:start
```

Each service exposes the standard lifecycle (`install`, `uninstall`, `update`, `start`, `stop`, `restart`, `status`, `logs`).

### Daemon mode

Run pv as a background service that starts on login:

```bash
pv daemon:enable     # Install + start daemon
pv daemon:disable    # Stop + uninstall daemon
pv daemon:restart    # Restart the daemon
```

### Update & uninstall

```bash
pv update            # Self-update pv + all tools
pv update --no-self-update  # Only update tools
pv uninstall         # Complete removal with guided cleanup
```

## Migrating from pre-pv.yml versions

If you used pv before this release, your projects worked via auto-detection plus a hardcoded setup pipeline. Both are gone. **`pv.yml` is now the project's contract with pv.**

To migrate an existing linked project:

```bash
cd /path/to/your/project
pv init               # detects project type; writes pv.yml with sensible defaults
# review the generated file — adjust services, env, setup as needed
git add pv.yml && git commit -m "Add pv.yml"
pv link               # relinks with the new contract
```

`pv init` writes a `pv.yml` with:

- The project's PHP version (from `composer.json` `require.php` when parseable, otherwise your global default)
- A `postgresql:` or `mysql:` block when the matching engine is installed (use `--mysql` to prefer mysql when both are installed)
- A `setup:` block with `cp .env.example .env`, optional `pv <engine>:db:create <name>`, `composer install`, `php artisan key:generate`, and `php artisan migrate` when a Laravel project has a generated database block (just `composer install` for generic PHP, empty for static sites)
- An `aliases:` block with a commented example hostname you can uncomment or replace

Common adjustments after `pv init`:

- **No database**: remove the `postgresql:` / `mysql:` block and the `pv <engine>:db:create` + `php artisan migrate` lines from `setup:`.
- **Custom migrate command**: replace `php artisan migrate` with whatever your team uses (e.g., `php artisan x-migrate` for multi-database setups).
- **Custom env keys**: add to the top-level `env:` block or per-service `env:` map. Values can be plain strings or templates. Top-level `env:` exposes project vars like `{{ .site_url }}` and `{{ .tls_cert_path }}`; per-service `env:` maps expose service vars like `{{ .host }}` and `{{ .port }}`. See [the spec](docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md) for the full template variable reference.
- **Aliases**: uncomment the `aliases:` line and add hostnames; each alias gets its own TLS cert.

## What's no longer automatic

The following used to happen invisibly during `pv link` and related commands. After this release, they only happen if you declare them in `pv.yml`:

- **Service binding from `.env` hints** (e.g., `DB_CONNECTION=pgsql` no longer auto-binds postgres) — declare the service in `pv.yml` instead.
- **Laravel-shaped env writes** (`DB_HOST`, `DB_PORT`, `CACHE_STORE`, `SESSION_DRIVER`, `QUEUE_CONNECTION`, `FILESYSTEM_DISK`, `MAIL_MAILER`) — declare the keys in `pv.yml`'s `env:` blocks; service connection values come from per-service template variables like `{{ .host }}` and `{{ .port }}`.
- **`.env.example` → `.env` copy** — put `cp .env.example .env` in `setup:`.
- **Composer install, `key:generate`, migrations, Octane install** — put each in `setup:`.
- **Database creation** — call `pv postgres:db:create <name>` (or `mysql`) from `setup:`.
- **Retroactive env writes on service install** (e.g., installing postgres no longer modifies existing projects' `.env`) — re-run `pv link` in each project to refresh env values from your pv.yml's templates.
- **`APP_URL` and Vite TLS env vars** — declare in pv.yml's top-level `env:`:

  ```yaml
  env:
    APP_URL: "{{ .site_url }}"
    VITE_DEV_SERVER_KEY: "{{ .tls_key_path }}"
    VITE_DEV_SERVER_CERT: "{{ .tls_cert_path }}"
  ```

The trade-off: a one-time `pv init` per project in exchange for never seeing a mystery `.env` write again.

## Architecture

```
~/.pv/
├── bin/                        # User PATH — shims and symlinks only
│   ├── php                     # Shim (version resolution)
│   ├── composer                # Shim (wraps PHAR with PHP)
│   ├── frankenphp              # Symlink → ~/.pv/php/{ver}/frankenphp
│   └── mago                    # Symlink → ~/.pv/internal/bin/mago
├── internal/bin/               # Private storage — real binaries
│   ├── mago
│   └── composer.phar
├── pv.yml                      # Global settings (TLD, default PHP)
├── config/                     # Server configuration
│   ├── Caddyfile
│   ├── sites/                  # Per-project Caddyfile includes
│   └── sites-{ver}/           # Per-version site configs
├── data/                       # Registry, PID file
├── logs/                       # Server logs
└── php/                        # Versioned PHP binaries
    └── {ver}/frankenphp + php
```

### Multi-version PHP

The main FrankenPHP process (global PHP version) serves on :443/:80. Projects using a different PHP version are proxied to secondary FrankenPHP processes running on high ports (`8000 + major*100 + minor*10`, e.g., PHP 8.3 → port 8830).

Version resolution: `pv.yml` `php` field → `composer.json` `require.php` → global default.

### Source layout

```
main.go              # Entry point
cmd/                 # Core/orchestrator commands + thin registration shims
internal/
  commands/          # Grouped tool/service/daemon commands
    php/             # php:install, php:use, php:list, etc.
    mago/            # mago:install, mago:update, etc.
    composer/        # composer:install, composer:update, etc.
    postgres/        # postgres:install, postgres:start, etc.
    mysql/           # mysql:install, mysql:start, etc.
    redis/           # redis:install, redis:start, etc.
    rustfs/          # rustfs:* / s3:* (object storage)
    mailpit/         # mailpit:* / mail:* (SMTP catcher)
    daemon/          # daemon:enable, daemon:disable, etc.
  tools/             # Tool abstraction (exposure, shims, symlinks)
  config/            # Path helpers, settings
  registry/          # Project registry
  phpenv/            # PHP version management
  caddy/             # Caddyfile generation
  server/            # Process management, DNS
  daemon/            # macOS launchd integration
  supervisor/        # Native binary supervision (start/stop/health)
  binaries/          # Binary download helpers
  selfupdate/        # pv self-update
  postgres/, mysql/, redis/  # Per-database lifecycle helpers
  services/          # Binary-service registry (mail, s3)
  detection/         # Project type detection
  setup/             # Prerequisites, shell config
  ui/                # Terminal UI (lipgloss)
```
