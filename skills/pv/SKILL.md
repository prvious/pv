---
name: pv
description: Work in a PHP or Laravel project served locally by PV, the `pv` command-line tool that serves `.test` sites over HTTPS and runs PHP, MySQL, Postgres, Redis, Mailpit and S3-compatible storage. Use it to find a project's URL and databases, wait for a new Git worktree to be ready, change PHP or services through `pv.yml`, verify a change over HTTPS, or work out why `php`, `composer`, a `.test` URL or a database connection misbehaves.
---

# Working with PV

This skill is published by the PV project. It describes how PV runs the project you are working in, so you can use the environment the developer already has instead of building another.

A project uses PV when it has a `pv.yml` (or `pv.yaml`) at its root, or when `pv list --json` lists its directory. PV calls each linked directory a **Project**. A Project is served at `https://<hostname>`, gets its own databases, Redis key prefix and storage buckets, and has PV-managed values in its `.env`.

## Ground rules

- **The site is already served.** Don't run `php artisan serve`, `valet`, Docker or another web server. PV serves the Project at its `.test` URL.
- **Use PV's PHP and Composer.** `command -v php` should print `~/.pv/bin/php`. If it doesn't, the shell didn't load PV's `PATH`. Call `~/.pv/bin/php`, `~/.pv/bin/composer` and `~/.pv/bin/pv` by full path instead of using another PHP.
- **Don't approve hooks.** You may add `hooks` to `pv.yml`, but only the developer can approve them, by running `pv hooks:trust` in a terminal. Tell them which commands need approval.
- **Don't delete data.** Never run `pv uninstall`, or any `:uninstall` with `--prune` or `--force`. Removing a Git worktree makes PV delete that worktree's databases, Redis keys and buckets; ask before you run `git worktree remove`.
- **Keep secrets out of the conversation.** `pv project:env` prints passwords and keys. Read only the key you need (see below), and never paste the whole output.
- **Leave the PV block in `.env` alone.** PV owns the lines between `# >>> PV MANAGED` and `# <<< PV MANAGED`, and comments out values it overrides with `# pv: `. Change `pv.yml` instead.

## Find the Project, its URL and its databases

```sh
pv list --json
```

Each entry in `projects` has `path`, `hostname`, `slug`, `mode`, `php`, `resources`, `env` and `status`.

1. Pick the entry whose `path` is your working directory or its nearest parent. Inside a Git worktree, use only an entry whose `path` is inside that worktree. If none exists yet, read [worktrees](references/worktrees.md) before running other `pv` commands there.
2. The URL is `https://<hostname>`. A `resource-only` Project has `hostname: null` and serves nothing.
3. `status: "unknown"` is normal. `missing` means the directory is gone, and `config-invalid` means `pv.yml` doesn't validate. `env: "rendered"` means PV wrote the Project's `.env` values. `pending`, `failed` or `warning` mean it hasn't, or wrote them with a problem.

To get one value, such as the database name, without printing secrets:

```sh
pv project:env --json | jq -r '.DB_DATABASE'
```

This prints only keys that `pv.yml` maps. See [databases and services](references/databases.md) to connect a client, read caught mail, or find a Redis prefix or bucket.

## Change PHP or services

Edit `pv.yml`. PV watches it and applies the change on its own: it installs missing PHP or service versions, creates databases and renders `.env`. If the new file is invalid, PV keeps serving the last valid config, and `pv list --json` shows `config-invalid`.

```yaml
php: "8.4"
root: public
env:
  APP_URL: "${url}"
postgres:
  version: "18"
  allocations:
    app:
      env:
        DB_CONNECTION: pgsql
        DB_HOST: "${host}"
        DB_PORT: "${port}"
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
```

`pv php:use 8.4` writes the PHP version into `pv.yml` for you. After any change, check `pv list --json` until `env` is `rendered`. Then read the value you need, as above, to confirm what PV created.

## Git worktrees

PV links each Git worktree of a linked Project automatically and gives it its own hostname and databases. The hostname comes from the worktree's directory name, so look it up rather than guessing. Read [worktrees](references/worktrees.md) to wait for a worktree to be ready and to understand cleanup.

## Verify a change

Requests to `https://<hostname>` from PHP or Node need PV's certificate authority. Agent sandboxes may block them entirely. Read [verifying](references/verifying.md) before you conclude that a page is broken, and never turn off TLS verification to make a check pass.

## When something fails

Start with `pv status --json` and `pv doctor --json`, then [troubleshooting](references/troubleshooting.md).
