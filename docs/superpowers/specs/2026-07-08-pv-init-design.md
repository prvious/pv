# PV Init Design

## Summary

PV will add `pv init` as a guided Project config initializer for existing directories. The command writes or previews `pv.yml`; it does not scaffold applications, link Projects, install resources, run package managers, run framework commands, mutate `.env`, or touch daemon state.

The first version focuses on a polished Laravel-first path: inspect the target directory, show detected defaults, let the user edit high-value structured choices, preview the generated YAML, and write only after confirmation. Herd migration and `serve: false` are deferred.

## Goals

- Add `pv init [path]` for existing Project directories.
- Generate valid Project config using the existing `ProjectConfig` model.
- Keep `pv link` responsible for recording desired Project state and reconciliation.
- Detect Laravel/PHP shape and common resource needs from local files.
- Treat detection as suggestions, not hidden behavior.
- Provide an interactive checklist plus structured edits instead of a free-form YAML editor.
- Preview the exact generated YAML before writing.
- Support `--yes` for prompt-free generation and `--print` for docs/automation.
- Preserve existing config safely through parse, merge, and atomic writes.

## Non-Goals

- Do not create a new Laravel application.
- Do not run `composer install`, package managers, Artisan commands, migrations, app key generation, or starter kit setup.
- Do not link the Project or request daemon reconciliation.
- Do not install, start, or update Managed Resources.
- Do not write or edit `.env` directly.
- Do not add Herd, Valet, DDEV, or other migration modes in the first version.
- Do not add `serve: false` or any unserved Project config shape in this work.
- Do not add hooks or command-runner behavior.
- Do not implement an arbitrary YAML or env-key editor inside the wizard.

## Command Shape

The first-pass public surface is:

```shell
pv init
pv init path/to/project
pv init --yes
pv init --print
```

`pv init [path]` runs the guided interactive flow. If `path` is omitted, PV uses the current directory.

`--yes` accepts detected/default selections and writes without prompting. It still refuses invalid target paths, invalid existing config, config file conflicts, and unsafe writes.

`--print` renders the generated YAML to stdout and never prompts or writes files. It is useful for documentation, scripts, and users who want to redirect or manually edit the first draft.

`--yes` and `--print` are mutually exclusive because one writes and the other is explicitly read-only.

## Detection Inputs

`pv init` inspects only local Project files. It should not call the daemon, query PV state, refresh manifests, or run framework commands.

Inputs:

- `composer.json` for Laravel/PHP shape, package hints, and PHP constraint hints.
- Laravel files such as `artisan`, `bootstrap/app.php`, `config/app.php`, and `public/index.php`.
- Directory layout, especially `public/`, for `document_root`.
- `.env.example`, then `.env` only as a fallback for key names and non-secret shape.
- `package.json` for Vite and frontend tooling signals.
- Existing `pv.yml` or `pv.yaml` for update/merge behavior.

Detection must produce explanations such as:

```text
Detected Laravel project from composer.json, artisan, and public/index.php.
Detected Vite from package.json.
Detected MySQL from DB_CONNECTION=mysql.
Detected Redis from REDIS_HOST and CACHE_STORE=redis.
```

It should not read secret values from `.env` into generated YAML. `.env` and `.env.example` are key-shape inputs only.

## PHP Defaults

PHP detection uses Composer constraints as a hint, not as a hard fact. Composer constraints can be ranges such as `^8.3` or `>=8.2`, so `pv init` should suggest a supported PV track rather than claiming certainty.

First-pass behavior:

- If an existing config declares `php`, preserve it.
- If a Composer PHP constraint clearly maps to a supported PV track, preselect that track.
- If the constraint is ambiguous, preselect `latest`.
- If no PHP signal exists, preselect `latest`.
- The interactive flow lets the user choose another supported track or `latest`.

The generated config writes the selected PHP value explicitly:

```yaml
php: latest
```

or:

```yaml
php: "8.4"
```

Project-level PHP extension opt-ins are not inferred in the first version. Users can add `php.extensions` manually after generation.

## Resource Defaults

The checklist includes MySQL, Postgres, Redis, Mailpit, and RustFS/S3. Strong detections are preselected; weak or ambiguous detections are shown but not silently selected.

Each selected resource defaults to `version: latest` unless an existing config or user edit chooses a concrete track.

### MySQL And Postgres

Database selection comes primarily from `DB_CONNECTION`.

- `DB_CONNECTION=mysql` preselects MySQL.
- `DB_CONNECTION=pgsql` or `DB_CONNECTION=postgres` preselects Postgres.
- If both MySQL and Postgres keys exist but `DB_CONNECTION` is absent or ambiguous, neither is silently selected.

The default allocation is `app`. The edit flow lets users enter allocation names such as `app, analytics`.

Single allocation defaults:

```yaml
mysql:
  version: latest
  allocations:
    app:
      env:
        DB_CONNECTION: mysql
        DB_HOST: "${host}"
        DB_PORT: "${port}"
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
```

For multiple allocations, the first allocation may use the standard `DB_*` keys. Later allocations should use an uppercase allocation prefix to avoid duplicate rendered env keys, for example `ANALYTICS_DB_HOST`.

Postgres uses the same shape, with `DB_CONNECTION: pgsql`.

### Redis

Redis is preselected from signals such as `REDIS_HOST`, `REDIS_URL`, `CACHE_STORE=redis`, `CACHE_DRIVER=redis`, `SESSION_DRIVER=redis`, or `QUEUE_CONNECTION=redis`.

The default allocation is `cache`:

```yaml
redis:
  version: latest
  allocations:
    cache:
      env:
        REDIS_HOST: "${host}"
        REDIS_PORT: "${port}"
        REDIS_PREFIX: "${prefix}"
```

`REDIS_URL` may be generated only when the selected/app-detected env shape already prefers URL-style Redis configuration. Otherwise host/port/prefix is the conservative default.

### Mailpit

Mailpit is preselected from mail or SMTP signals such as `MAIL_MAILER=smtp`, `MAIL_HOST`, `MAIL_PORT`, or `MAIL_FROM_ADDRESS`.

Mailpit has resource-level env only:

```yaml
mailpit:
  version: latest
  env:
    MAIL_MAILER: smtp
    MAIL_HOST: "${smtp_host}"
    MAIL_PORT: "${smtp_port}"
```

Mailpit dashboard URL support may be offered when detected env keys already include a dashboard URL-style variable, but the base Laravel-first default should stay SMTP-focused.

### RustFS / S3

RustFS/S3 is preselected from `AWS_*`, `S3_*`, filesystem disk signals, or Laravel filesystem config hints.

The default allocation is `uploads`:

```yaml
rustfs:
  version: latest
  allocations:
    uploads:
      env:
        AWS_ENDPOINT: "${endpoint}"
        AWS_BUCKET: "${bucket}"
        AWS_ACCESS_KEY_ID: "${access_key}"
        AWS_SECRET_ACCESS_KEY: "${secret_key}"
```

When the detected app uses `S3_*` names instead of `AWS_*`, `pv init` may generate the matching `S3_*` keys using the same placeholders.

## Root Env And Vite TLS

For Laravel Projects, or when `APP_URL` exists in `.env.example` / `.env`, `pv init` should generate:

```yaml
env:
  APP_URL: "${project_url}"
```

If Vite is detected, `pv init` should offer PV TLS placeholder values. Laravel's Vite documentation configures HTTPS through `vite.config.js` with `server.https` and `detectTls`; it does not define a universal env variable contract. Therefore PV must not claim these variables are consumed automatically.

When selected, PV should generate Laravel-oriented env values for the Project to read from `vite.config.js`:

```yaml
env:
  APP_URL: "${project_url}"
  VITE_DEV_SERVER_CERT: "${tls_cert}"
  VITE_DEV_SERVER_KEY: "${tls_key}"
```

The command output should include a concise note that the app's Vite config must read these values for Vite dev-server HTTPS. `pv init` does not edit `vite.config.js` in the first version.

## Interactive Flow

The guided flow is:

1. Resolve the target directory.
2. Read existing Project config if present.
3. Inspect local files and build detection hints.
4. Print a concise detection summary.
5. Show a resource checklist with strong detections preselected.
6. Ask whether to use the selections, cancel, or edit.
7. In edit mode, prompt only structured fields:
   - PHP track,
   - document root,
   - selected resources,
   - resource track,
   - allocation names.
8. Render the final `ProjectConfig`.
9. Print the YAML preview.
10. Ask for final confirmation.
11. Write the config atomically.

The prompt should stay fast. Editing env key names, deeply custom resource layouts, and unusual app conventions are manual follow-up edits in `pv.yml`.

Non-interactive stdin without `--yes` or `--print` fails with a clear message.

## Write Behavior

PV prefers `pv.yml` for new config.

Existing files:

- If `pv.yml` exists, update it.
- If only `pv.yaml` exists, update `pv.yaml`.
- If both exist, fail with the existing config conflict error.
- If the existing config is invalid, fail and leave it unchanged.
- If the existing config path is a valid symlink inside the Project root, update the symlink target using existing config writer behavior.
- Preserve existing file mode when updating.
- Use the standard `0644` Project config mode for a new file.

Existing user config values win unless the user changes them through the structured flow. This keeps `pv init` repair-oriented and safe to rerun.

Examples:

- Existing `php` is preserved unless edited.
- Existing `document_root` is preserved unless edited.
- Existing resources remain present.
- Newly selected resources are added.
- Existing env mappings are not deleted just because detection did not rediscover them.

## Architecture

Add a small reusable config-initialization component in the `config` crate. It should own local file inspection, detection hints, generated `ProjectConfig` proposals, and merge behavior. It should expose typed results so the CLI can render summaries and prompts without parsing strings.

The CLI owns interaction:

- argument parsing,
- terminal prompts,
- checklist rendering,
- `--yes` and `--print` behavior,
- final preview and confirmation.

The config crate owns safe config persistence:

- config discovery,
- preferred vs alternate file handling,
- conflict detection,
- parse/validate before write,
- mode preservation,
- symlink target handling,
- atomic writes.

This should extend or sit beside `write_project_php_track` so `pv init` and `pv php:use` share the same safe writing rules instead of duplicating filesystem behavior.

Daemon, state, protocol, and resource install code should not change for first-pass `pv init`.

## Error Handling

Domain detection and config generation errors should use typed `config` errors where they affect library behavior. CLI orchestration may wrap them in `anyhow` / `ExecuteError` with context.

Important failures:

- missing target path,
- non-directory target path,
- non-UTF-8 paths,
- `pv.yml` / `pv.yaml` conflict,
- invalid existing Project config,
- invalid detected/generated config,
- document root that does not exist or escapes the Project root,
- non-interactive run that needs prompts,
- refusal to overwrite or write after the user declines confirmation.

Declining an interactive write exits with failure, leaves files unchanged, and prints a clear cancellation message.

## Testing

Prefer integration tests and snapshots for command behavior.

Integration coverage in `it/cli.rs` should include:

- `pv init --help` in core workflow help snapshots.
- `pv init --print` for a Laravel fixture with MySQL, Redis, Mailpit, RustFS/S3, Vite, and `public/`.
- `pv init --yes` writes `pv.yml` in an injected temp Project and home.
- `pv init path/to/project --yes` accepts relative paths.
- Existing `pv.yaml` is updated instead of creating `pv.yml`.
- `pv.yml` / `pv.yaml` conflict fails without writing.
- Invalid existing config fails without writing.
- Non-interactive `pv init` without `--yes` or `--print` fails clearly.
- Existing config merge preserves user values while adding selected generated defaults.

Focused `config` crate tests should cover:

- Laravel/PHP/Vite/resource detection from fixture files.
- Composer PHP constraint hints, including ambiguous constraints.
- Resource default generation and multi-allocation env-key prefixing.
- Merge behavior on existing `ProjectConfig`.
- Writer behavior for new, existing, alternate, and symlinked config files.

Use `insta` snapshots for rendered CLI output and generated YAML where practical. Prefer typed assertions for internal detection and merge models.

Focused verification during implementation:

```shell
cargo insta test --accept --test-runner nextest -- <pv_init_test_name>
cargo nextest run -E 'test(<pv_init_test_name>)'
cargo fmt --all -- --check
git diff --check
```

Before completion, run a broader workspace check if the implementation remains limited to config/CLI behavior:

```shell
cargo nextest run --workspace --locked
```

## Documentation Impact

`DESIGN.md` currently states that PV v1 does not include `pv init`. Implementation should replace that sentence with the approved behavior in this spec.

User documentation should add a short `pv init` section before `pv link`, showing:

```shell
pv init
pv link
```

The docs should emphasize that `pv init` writes Project config and `pv link` registers/reconciles the Project.

## Deferred Work

- `pv init --migrate herd`
- Valet/DDEV migration modes
- `serve: false`
- editing `.env`
- editing `vite.config.js`
- hooks / command runner
- package manager or framework command execution
- rich per-env-key editing inside the wizard
