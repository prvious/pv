# PV User Guide

PV is a macOS local development control plane for PHP Projects. It installs the PV app, configures `.test` routing, runs a per-user daemon, and manages PHP/FrankenPHP, Composer, MySQL, Postgres, Redis, Mailpit, and RustFS artifacts from PV manifests.

## Install

Install PV from the stable installer URL published with the release candidate:

```shell
curl -fsSL <installer-url> | bash
```

The installer downloads the PV binary, verifies its SHA-256 checksum, installs it under `~/.pv/bin/releases/<version>/pv`, updates the active `~/.pv/bin/pv` symlink, and runs `pv setup` unless `--no-setup` is used.

Installer flags mirror setup behavior:

- `--yes` accepts PV prompts, but macOS may still ask for administrator credentials.
- `--non-interactive` disables prompts and fails if confirmation, shell profile editing, or macOS authentication is required.
- `--no-setup` installs only the PV binary.
- `--no-path` skips automatic shell profile edits.

## Setup

Run setup after installation or whenever system integration needs repair:

```shell
pv setup
```

`pv setup` creates the PV state layout, optionally repairs the PV-managed shell profile block, installs `.test` DNS resolver configuration, installs PV-owned `pf` loopback redirects for ports `80` and `443`, trusts the PV local CA in the macOS System keychain, registers and starts the per-user LaunchAgent daemon, and records desired default Managed Resources.

Fresh setup fetches the Managed Resource artifact manifest before planning default resources. If the fetch fails and a cached manifest exists, PV may use the cached manifest with a warning. If no cached manifest exists, setup fails before default-resource planning and remains safe to rerun.

Current default setup tracks are PHP/FrankenPHP `8.5`, Composer `2`, MySQL `8.4`, Postgres `18`, Redis `8.8`, Mailpit `1`, and RustFS `1`. Setup installs the desired default resources but does not start backing services until a linked Project needs them.

## Shell Integration

`pv env` prints shell code for PV-managed binaries and Composer paths:

```shell
eval "$(pv env --shell zsh)"
```

Supported shells are `zsh`, `bash`, and `fish`. PV does not auto-install shell completions; generate them with:

```shell
pv completions zsh
```

## Link, Open, And List

Link a Project from the current directory:

```shell
pv link
```

Link another directory or choose a hostname:

```shell
pv link ~/Code/acme --hostname acme.test
```

`pv link` records desired Project state and requests daemon reconciliation. It can run before setup, but the Project is not reachable until setup completes.

Open a linked Project:

```shell
pv open
pv open acme.test
```

List linked Projects:

```shell
pv list
pv list --json
```

Unlinking removes the Project from PV desired state without deleting the Project directory or data:

```shell
pv unlink
pv unlink acme.test
```

## Project Config: `pv.yml`

PV reads Project config from `pv.yml` or `pv.yaml` in the Project root. Documentation should prefer `pv.yml`. If both files exist, PV reports a config conflict.

Example:

```yaml
php: "8.5"
document_root: public
hostnames:
  - api.acme.test
env:
  APP_URL: "${project_url}"
mysql:
  version: "8.4"
  allocations:
    app:
      env:
        DB_CONNECTION: mysql
        DB_HOST: "${host}"
        DB_PORT: "${port}"
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
redis:
  version: "8.8"
  allocations:
    cache:
      env:
        REDIS_HOST: "${host}"
        REDIS_PORT: "${port}"
        REDIS_PREFIX: "${prefix}"
```

Project config accepts YAML anchors, aliases, and merge keys as YAML syntax. PV resolves them before validating keys and values. Unknown keys that remain after YAML resolution fail validation.

### PHP Extensions

The `php` key may be a scalar version or an object:

```yaml
php:
  version: 8.4
  extensions:
    - redis
    - xdebug
```

If `version` is omitted, PV uses the configured default PHP track:

```yaml
php:
  extensions:
    - xdebug
```

PV loads bundled optional extensions that are available in the installed PHP artifact. Unknown extension names are ignored and reported as warnings.

Preview rendered environment values without editing `.env`:

```shell
pv project:env
pv project:env acme.test --json
```

`pv project:env` prints generated secrets because it is the explicit Project environment command.

## Managed Resources

PV manages runtime artifacts as desired state. Install, update, list, and uninstall commands are available per resource:

```shell
pv php:install 8.5
pv mysql:install 8.4
pv postgres:list --json
pv redis:update
pv mailpit:open
pv rustfs:open
```

Aliases are available for common resource names:

- `pg:*` aliases `postgres:*`.
- `mail:*` aliases `mailpit:*`.
- `s3:*` aliases `rustfs:*`.

Resource uninstall preserves data by default. Use `--prune` to delete resource data and `--force` to bypass active-use guards or non-interactive prune confirmation:

```shell
pv mysql:uninstall 8.4 --prune --force
```

If a Project still declares a forced-uninstalled track, reconciliation fails until you remove that declaration or explicitly reinstall the track.

## Update

Preview app and installed Managed Resource updates:

```shell
pv update --check
pv update --check --json
```

`pv update --check` requires the PV daemon. If the daemon is unavailable, run `pv daemon:restart` or rerun `pv setup`.

Apply updates:

```shell
pv update
```

`pv update` updates the PV app first when a newer app release is available, restarts or reconnects to the daemon, then updates installed Managed Resource tracks. It does not install new default tracks simply because they exist; setup owns default desired installs.

## Diagnostics

Use these commands to inspect PV state:

```shell
pv status
pv status --json
pv doctor
pv logs --all
pv jobs --json
```

Targeted commands are also available for specific system integrations:

```shell
pv daemon:restart
pv dns:status
pv ports:status
pv ca:status
```

The real artifact workflow/resource matrix is opt-in for development and release validation with `PV_E2E_REAL_ARTIFACTS=1` and an artifact manifest URL. The privileged macOS RC workflow can run by manual dispatch or through Real Artifact E2E with `privileged_rc=true`, and uploads evidence.

## Uninstall

Safe uninstall preserves user data:

```shell
pv uninstall
```

By default, PV removes the daemon registration, DNS resolver config, `pf` redirects, local CA trust, PV app binaries, shims, runtime metadata, sockets, generated config, and download cache. It preserves logs, `pv.db`, certificates, Composer home/cache, Managed Resource data, and Project `.env` blocks.

Remove all PV-owned state:

```shell
pv uninstall --prune --force
```

`--prune` requires confirmation unless `--force` is provided.
