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
pv install

# Install a PHP version
pv php install 8.4

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

# Unlink a project
pv unlink my-app
```

### PHP version manager

`pv` is also a full PHP version manager. Install multiple versions side-by-side and switch between them instantly — no phpenv, phpbrew, or homebrew tap juggling.

```bash
# Install multiple versions
pv php install 8.3
pv php install 8.4
pv php install 8.5

# Switch the global default
pv use 8.4

# See what's installed
pv php list

# Remove a version
pv php remove 8.3
```

Per-project versions are supported too — drop a `.pv-php` file in your project root or let `pv` read the PHP constraint from `composer.json`. Multiple PHP versions run simultaneously, each project served by its own FrankenPHP process.

`pv link` auto-detects your project type (Laravel, Laravel + Octane, generic PHP, static) and generates the right server configuration automatically.
