# pv

Local development server manager powered by [FrankenPHP](https://frankenphp.dev).

## Why

Running PHP projects locally shouldn't require juggling dnsmasq, Docker, and Traefik. `pv` replaces all of that with a single binary. Link a project directory and it's instantly available at `https://project.test` — with HTTPS, automatic PHP serving, and its own built-in DNS. No containers, no proxy chains, no config files to maintain.

Currently supports PHP projects (Laravel, generic PHP, static sites).

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

# Switch PHP versions
pv use 8.3
pv php list
```

`pv link` auto-detects your project type (Laravel, Laravel + Octane, generic PHP, static) and generates the right server configuration. Multiple PHP versions can run simultaneously — projects using a different version than the global default are automatically proxied to a dedicated process.
