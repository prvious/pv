# Implementation Plan: Epic 4 - Laravel Project Experience

## Execution Rules

- Treat legacy issues #106-#108 and #111 as reference only.
- Laravel is the primary product path.
- Keep project contracts human-authored and reviewable.
- Machine-owned state belongs in the store, not in `pv.yml`.
- Do not infer services, env values, or setup commands from `.env`.
- Use Go for repository logic.
- Before Go work, activate `golang-pro` and `modern-go`.
- Before each commit, run `go-simplifier` on changed Go code.
- Always try to add or update tests for changed behavior.
- Before handing off Go changes, run:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Implementation Contract

Execute the published leaf issues in dependency order. Do not add hidden setup,
service inference, or env writes outside these issue contracts.

| Issue range | Required output |
| --- | --- |
| #171-#175 | Versioned `pv.yml` parser/generator and deterministic `pv init`. |
| #176-#181 | Project desired state, managed env writer, setup runner, and `pv link`. |
| #182-#187 | Gateway desired/observed state, route rendering, DNS/TLS adapters, and `pv open`. |
| #188-#192 | Artisan, database, mail, and S3 helper routing. |

Non-negotiable decisions:

- Stacked diff branch is `rewrite/epic-4-laravel-project-experience` and its
  base is `rewrite/epic-3-runtime-daemon-resources`.
- Epic 4 PRs do not target `main` directly.
- New rewrite contracts require top-level `version: 1`.
- `setup` is an ordered list of shell command strings.
- Each setup command runs in its own shell from the project root with managed PHP first on `PATH`.
- `pv init` never mutates `.env`, installs resources, or reads `.env` to choose services.
- `pv link` writes only declared env keys and runs only declared setup commands.
- Helper commands never auto-create missing resources.
- DNS, TLS, browser, and gateway process behavior must be adapter-driven in tests.

## Suggested Package Ownership

- `internal/project` owns contract parsing, validation, current-project
  resolution, and project registry models.
- `internal/resources/laravel` owns Laravel detection, defaults, contract
  generation, and Laravel-specific command behavior.
- `internal/app` owns command use cases such as init, link, open, artisan, db,
  mail, and s3.
- `internal/host` owns file, DNS, TLS, and browser adapters.
- `internal/resources/gateway` owns gateway desired state, route rendering, and
  gateway process definitions.

## Feature 4.1: Project Contract And Init

**Goal:** Generate and validate explicit Laravel project contracts.

### Implementation Sequence

1. Add versioned `pv.yml` contract schema with required top-level `version: 1`.
2. Add validation for PHP version, services, aliases, setup commands, and
   unsupported fields.
3. Add Laravel project detection using explicit markers.
4. Add Laravel defaults for PHP, aliases, env templates, and setup without
   reading service decisions from `.env`; service declarations are generated
   only from explicit flags or documented generator defaults.
5. Add contract generator that produces stable, reviewable YAML.
6. Add `pv init` command.
7. Add overwrite protection and `--force` behavior.
8. Add unsupported project errors; do not add generic fallback project behavior.

### Acceptance Notes

- `pv init` creates a contract only; it does not install resources or mutate
  `.env`.
- Generated YAML should be deterministic for review.
- Unsupported project types should fail clearly.

## Feature 4.2: Link, Env, And Setup

**Goal:** Turn a project contract into durable desired project state and safe
local configuration.

### Implementation Sequence

1. Add project registry or project desired-state model.
2. Parse and validate `pv.yml` during `pv link`.
3. Record project desired state durably.
4. Add env parser and managed block writer.
5. Add `.env` backup behavior before mutation.
6. Render only env keys declared by contract/resource env providers.
7. Add setup runner with project working directory, managed PATH, pinned PHP,
   env propagation, and stdout/stderr streaming.
8. Make setup fail fast on first command error.
9. Signal daemon after durable link state is written.
10. Add clear errors for missing declared resources or installs.

### Acceptance Notes

- Removed declarations should remove or update only pv-managed env keys.
- User-authored `.env` lines must be preserved.
- Setup commands run only when declared.

## Feature 4.3: Gateway And pv open

**Goal:** Serve linked Laravel projects at HTTPS `.test` hosts.

### Implementation Sequence

1. Add gateway desired and observed state.
2. Add deterministic route model for primary host and aliases.
3. Add Caddy/FrankenPHP config rendering.
4. Add TLS certificate material model with SAN behavior.
5. Add DNS adapter through host primitives.
6. Add gateway process definition for supervisor.
7. Add route/status data for linked projects.
8. Add browser-open adapter.
9. Add `pv open` command with current-project resolution.

### Acceptance Notes

- Treat FrankenPHP as gateway infrastructure, not a user-managed service.
- DNS, TLS, and browser behavior must be adapter-driven for tests.
- Route rendering must be stable and diffable.

## Feature 4.4: Laravel Helper Commands

**Goal:** Make common Laravel workflows route through managed runtimes and
declared resources.

### Implementation Sequence

1. Add current-project resolution for helper commands.
2. Add `pv artisan` to run through pinned managed PHP.
3. Add `pv db` helper routing to declared Postgres or MySQL resource.
4. Add `pv mail` helper routing to declared Mailpit resource.
5. Add `pv s3` helper routing to declared RustFS resource.
6. Add missing-resource and wrong-project errors.
7. Add stdout/stderr behavior that remains scriptable.

### Acceptance Notes

- Helpers should not auto-create missing resources.
- Helper commands must use declared project state.
- Missing resources should produce actionable errors.

## Critical Path

1. Contract schema and parser.
2. Laravel detection and deterministic contract generation.
3. Project registry and `pv link`.
4. Managed env writer.
5. Setup runner.
6. Gateway route rendering, DNS/TLS adapters, and process definition.
7. `pv open`.
8. Laravel helper commands.

## Review Checklist

- [ ] Generated `pv.yml` is stable and reviewable.
- [ ] `pv link` records desired state before signaling daemon.
- [ ] `.env` writes are declared-only and labeled.
- [ ] Setup uses managed PHP.
- [ ] Gateway behavior uses adapters for OS mutation.
- [ ] Helper commands resolve the current project first.
- [ ] Tests isolate `HOME` for pv state.
- [ ] Tests do not use `t.Parallel()` with `t.Setenv`.
