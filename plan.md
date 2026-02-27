# pv — Private Vault

A local development server manager powered by FrankenPHP. Think Laravel Herd, but built your way.

## The Problem

Running Laravel (and other) projects locally with Docker works, but introduces friction. Some tools and packages don't support Docker environments (browser testing plugins, debugging tools, etc.), and managing containers adds overhead for what should be simple local development.

## The Solution

`pv` replaces Docker for local development by managing a single FrankenPHP instance (which is Caddy with PHP built in) that serves all your projects under `.test` domains. You point it at a project directory and it handles the rest — detection, configuration, and serving.

`pv` includes its own embedded DNS server (using Go's `miekg/dns` library) that responds to all `*.test` queries with `127.0.0.1`. Combined with macOS's native `/etc/resolver/test` file (which tells the OS to route `.test` lookups to localhost), no external DNS tools like dnsmasq are needed. The result is: `pv link ~/code/my-app` → `my-app.test` is live, with HTTPS, instantly.

## How It Works

`pv` manages everything under `~/.pv/`:

```
~/.pv/
├── bin/                    # added to PATH — all managed binaries
│   ├── pv                  # the CLI itself
│   ├── php                 # managed PHP (shim that resolves version)
│   ├── frankenphp          # FrankenPHP binary (Caddy + PHP)
│   ├── composer
│   └── mago                # PHP linter/formatter
├── config/
│   ├── Caddyfile           # main Caddyfile (imports sites/*)
│   └── sites/              # per-project Caddy snippets
│       ├── my-app.caddy
│       └── api.caddy
├── logs/
│   └── caddy.log
└── data/
    └── registry.json       # linked projects and their detected types
```

When you `pv link` a project, the CLI:

1. Detects the project type (Laravel, Laravel+Octane, Node, static, etc.)
2. Generates a Caddy configuration snippet for that project
3. Saves it to `~/.pv/config/sites/`
4. Reloads FrankenPHP (graceful, zero-downtime)

The main `Caddyfile` simply imports all site snippets, so adding/removing projects is just adding/removing files and reloading.

## Project Detection

`pv` reads project files to determine the correct serving strategy:

| Signal                                                                                       | Detected As      | Serving Strategy                             |
| -------------------------------------------------------------------------------------------- | ---------------- | -------------------------------------------- |
| `composer.json` with `laravel/framework` + `laravel/octane` + `public/frankenphp-worker.php` | Laravel + Octane | FrankenPHP worker mode (app stays in memory) |
| `composer.json` with `laravel/framework`                                                     | Laravel          | FrankenPHP serves `public/`                  |
| `composer.json` (generic)                                                                    | PHP project      | FrankenPHP serves root or `public/`          |
| `package.json` with `dev` script                                                             | Node app         | Caddy reverse proxies to dev server port     |
| `index.html` at root                                                                         | Static site      | Caddy file server                            |

For Laravel + Octane projects, if the worker file doesn't exist yet, `pv` can bootstrap it by running `php artisan octane:install --server=frankenphp` automatically.

## Caddy Snippets (Generated)

**Laravel with Octane (worker mode):**

```caddyfile
my-app.test {
    root * /Users/clovis/code/my-app/public
    encode zstd gzip

    php_server {
        worker {
            file /Users/clovis/code/my-app/public/frankenphp-worker.php
            num 4
            watch /Users/clovis/code/my-app/**/*.php
        }
    }
}
```

The `watch` directive gives you automatic worker restarts on file changes — no manual `octane:reload` needed.

**Standard Laravel:**

```caddyfile
my-app.test {
    root * /Users/clovis/code/my-app/public
    encode zstd gzip

    php_server
}
```

## CLI Commands

```
pv install                        # first-time setup: download binaries, create dirs, trust CA
pv link [path] [--name=custom]    # register a project to be served
pv unlink [name]                  # remove a project
pv list                           # show all linked projects
pv start / stop / restart         # manage the FrankenPHP process
pv log [name]                     # tail logs
pv open [name]                    # open project in browser
pv status                         # show running state and linked site count
```

## PHP Version Management (Future)

FrankenPHP embeds PHP into the binary, so each PHP version requires its own FrankenPHP build. The architecture supports multiple versions:

```
~/.pv/php/
├── 8.4/frankenphp
├── 8.3/frankenphp
└── 8.2/frankenphp
```

For **CLI usage**, `~/.pv/bin/php` is a shim that resolves the right version by checking for a `.pv-php` file in the project directory (walking up to root), falling back to the global default.

For **serving**, multiple FrankenPHP instances run on internal ports, and a front Caddy proxy routes each project to the instance matching its PHP version.

```
pv php install 8.2               # download FrankenPHP build for PHP 8.2
pv php list                       # show installed versions
pv use php:8.2                    # set global default
pv use php:8.2 --project          # set for current project (writes .pv-php)
```

## Services (Future)

Databases, Redis, and other services would still need to run somewhere. A lightweight `pv services` layer could manage a small Docker Compose file dedicated to services only — keeping the benefit of native PHP execution while still containerizing stateful services.

## Tech Stack

- **CLI:** Go
- **Web server:** FrankenPHP (Caddy + embedded PHP)
- **DNS:** Embedded DNS server (via `miekg/dns` Go library) + macOS `/etc/resolver/test`

---

## Development Plan

### Phase 1 — CLI Skeleton

Set up the Go project with `cobra` and implement the core commands: `pv link [path]`, `pv unlink [name]`, `pv list`. These commands manage a `registry.json` file that tracks linked projects but don't yet generate any Caddy config.

### Phase 2 — Project Detection

Build the detection engine that reads project files (`composer.json`, `package.json`, `index.html`) and determines the project type and serving strategy. This runs automatically during `pv link` and stores results in the registry.

### Phase 3 — Caddyfile Generation

Create templates for each project type and generate per-project `.caddy` snippets into `~/.pv/config/sites/`. Write a main `Caddyfile` that imports all snippets. At this point you can manually start FrankenPHP against the generated config and sites will work.

### Phase 4 — FrankenPHP Lifecycle

Implement `pv start`, `pv stop`, `pv restart`. The CLI manages the FrankenPHP process, starts it with the generated Caddyfile, and handles graceful reloads when sites are linked/unlinked. Add `pv log` for tailing output.

### Phase 5 — First-Time Setup

Build `pv install` which downloads the FrankenPHP binary (and optionally Composer, Mago), creates the `~/.pv/` directory structure, writes `/etc/resolver/test` (requires sudo) so macOS routes `.test` DNS queries to the embedded DNS server, trusts Caddy's local CA for HTTPS, and prints instructions for adding `~/.pv/bin` to PATH.
