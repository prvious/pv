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

`pv link` auto-detects your project type (Laravel, Laravel + Octane, generic PHP, static) and generates the right server configuration automatically.

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
