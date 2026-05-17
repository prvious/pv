# Test Issues Checklist: Epic 4 - Laravel Project Experience

## Test Issue #175: Project Contract And Init Behavior

Labels: `test`, `priority-high`, `laravel`, `ready-for-agent`

Required coverage:

- [ ] Minimal Laravel `pv.yml` with `version: 1` parses.
- [ ] Full Laravel `pv.yml` with services, env, aliases, and setup parses.
- [ ] Unsupported contract version is rejected.
- [ ] Unknown fields are rejected with clear errors.
- [ ] Laravel detection uses explicit markers.
- [ ] Non-Laravel or unsupported projects fail clearly.
- [ ] Generated YAML ordering is deterministic.
- [ ] `pv init` does not mutate `.env`.
- [ ] Existing `pv.yml` is not overwritten by default.
- [ ] Forced overwrite is explicit and tested.

## Test Issue #181: Link Env And Setup Behavior

Labels: `test`, `priority-high`, `laravel`, `control-plane`, `ready-for-agent`

Required coverage:

- [ ] `pv link` validates `pv.yml` before state writes.
- [ ] Project desired state records path, host, aliases, version, PHP, services, env declarations, and setup commands.
- [ ] Project identity is deterministic.
- [ ] Store write happens before daemon signal.
- [ ] `.env` backup is written before mutation.
- [ ] User-authored `.env` lines are preserved.
- [ ] pv-managed entries are labeled.
- [ ] Removed declarations update only pv-managed entries.
- [ ] Existing `.env` values are not used to infer services.
- [ ] Setup commands run from project root.
- [ ] Managed PHP path precedes system PATH.
- [ ] Setup stdout and stderr stream predictably.
- [ ] Setup stops on first failed command.
- [ ] Missing declared resources and installs produce actionable errors.

## Test Issue #187: Gateway And pv open Behavior

Labels: `test`, `priority-high`, `gateway`, `laravel`, `ready-for-agent`

Required coverage:

- [ ] Gateway desired state includes primary host, aliases, project path, and runtime reference.
- [ ] Observed state records route status and failure information.
- [ ] Route rendering is stable across runs.
- [ ] Primary host and aliases are included in route output.
- [ ] TLS SANs cover primary host and aliases.
- [ ] DNS adapter errors are actionable.
- [ ] Browser adapter is used by `pv open`.
- [ ] `pv open` resolves the current linked project.
- [ ] Missing link and missing gateway state return actionable errors.
- [ ] Unit tests do not mutate real DNS, trust stores, keychains, or browsers.

## Test Issue #192: Laravel Helper Command Routing

Labels: `test`, `priority-high`, `laravel`, `ready-for-agent`

Required coverage:

- [ ] `pv artisan` resolves current project and uses managed PHP.
- [ ] Artisan arguments pass through unchanged.
- [ ] `pv db` routes to declared Postgres or MySQL resource.
- [ ] Missing database declaration returns a clear error.
- [ ] `pv mail` routes to declared Mailpit resource.
- [ ] Missing Mailpit declaration returns a clear error.
- [ ] `pv s3` routes to declared RustFS resource.
- [ ] Missing RustFS declaration returns a clear error.
- [ ] Helpers do not auto-create missing resources.
- [ ] Helpers do not print secret values.

## Exit Evidence For All Epic 4 Test Issues

- [ ] Generated Laravel fixtures are deterministic and small.
- [ ] Tests touching pv state isolate `HOME`.
- [ ] Tests that call `t.Setenv` do not call `t.Parallel()`.
- [ ] Root verification passes.
