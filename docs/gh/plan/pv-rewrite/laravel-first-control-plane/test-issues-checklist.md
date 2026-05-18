# Test Issues Checklist: Laravel-First Local Control Plane

Create these as test work items linked to the feature issues in
`issues-checklist.md`.

## Test Issue Template

```markdown
## Test Scope

{Feature or story being validated}

## ISTQB Techniques

- [ ] Equivalence partitioning
- [ ] Boundary value analysis
- [ ] Decision table testing
- [ ] State transition testing
- [ ] Experience-based testing

## Test Cases

- [ ] Happy path
- [ ] Invalid input or missing dependency
- [ ] Failure state and next action
- [ ] Regression coverage from prototype behavior where relevant

## Quality Characteristics

- [ ] Functional suitability
- [ ] Reliability
- [ ] Maintainability
- [ ] Security or secret-handling, if relevant
- [ ] Portability boundary, if relevant

## Definition Of Done

- [ ] Tests are committed with implementation or before implementation.
- [ ] Root verification commands pass.
- [ ] Manual QA steps are documented if automation is not practical.
```

## Test Issues To Create

### Test: Root Scaffold And CLI Scriptability

**Labels:** `test`, `priority-critical`, `control-plane`

- [ ] Help output is stable.
- [ ] Version output is pipeable on stdout.
- [ ] Unknown commands return usage errors.
- [ ] Human status is written to stderr.
- [ ] Fang or another presentation dependency is absent unless justified.

### Test: Control-Plane Desired/Observed Tracer

**Labels:** `test`, `priority-critical`, `control-plane`

- [ ] Desired writes persist.
- [ ] Observed writes persist separately.
- [ ] Desired writes do not imply observed status.
- [ ] Controller no-ops when desired state is absent.
- [ ] Controller records ready and failed observed status.

### Test: Store Migrations And Filesystem Layout

**Labels:** `test`, `priority-critical`, `control-plane`

- [ ] Path helpers cover every canonical layout family.
- [ ] `bin/` is shims only.
- [ ] Services cannot invent ambiguous top-level binary/data paths.
- [ ] Schema version is stored.
- [ ] Applied migrations are recorded.
- [ ] Migration failures do not silently reinterpret state.

### Test: Install Planner Scheduling And Failures

**Labels:** `test`, `priority-high`, `control-plane`

- [ ] Plans include runtimes, tools, and services.
- [ ] Downloads are bounded.
- [ ] Installs respect dependency order.
- [ ] Shim exposure is atomic.
- [ ] Failed install does not advertise completed work.
- [ ] Successful durable changes signal daemon reconciliation.

### Test: PHP And Composer Runtime Dependency Behavior

**Labels:** `test`, `priority-critical`, `runtime`

- [ ] PHP runtime desired state reconciles to ready.
- [ ] PHP install failure records failed observed status.
- [ ] Composer requires runtime version.
- [ ] Composer blocks when PHP runtime is missing.
- [ ] Composer shim uses pinned PHP runtime.
- [ ] Status shows runtime dependency state.

### Test: Daemon And Supervisor Process Lifecycle

**Labels:** `test`, `priority-critical`, `resource`

- [ ] Supervisor starts and stops fake processes.
- [ ] Supervisor readiness checks pass and fail.
- [ ] Crash restart obeys restart budget.
- [ ] Log path is attached to observed status.
- [ ] Daemon signal triggers reconcile.
- [ ] Supervisor API remains resource-agnostic.

### Test: Database Resource Behavior

**Labels:** `test`, `priority-critical`, `resource`

- [ ] Postgres version lines are stable.
- [ ] Postgres process definition and readiness are explicit.
- [ ] Postgres env values are correct.
- [ ] Postgres create/drop are explicit commands.
- [ ] MySQL version lines are stable.
- [ ] MySQL init, socket/PID, privileges, and readiness are explicit.
- [ ] MySQL env values are correct.
- [ ] Shared helpers do not erase database-specific behavior.

### Test: Redis, Mailpit, And RustFS Resources

**Labels:** `test`, `priority-high`, `resource`

- [ ] Redis process flags and persistence behavior are explicit.
- [ ] Redis env values are correct.
- [ ] Mailpit SMTP/web ports are correct.
- [ ] Mailpit env values are correct.
- [ ] RustFS credentials and routes are correct.
- [ ] RustFS status does not expose secrets.
- [ ] All resources report missing install, running, stopped, and failed states.

### Test: Project Contract And Init Generation

**Labels:** `test`, `priority-critical`, `laravel`

- [ ] Minimal `pv.yml` parses.
- [ ] Full Laravel `pv.yml` parses.
- [ ] Unsupported fields error clearly.
- [ ] Laravel detection works.
- [ ] Generated contract includes PHP, env, setup, services, and aliases.
- [ ] Existing `pv.yml` is not overwritten unless forced.

### Test: Link, Env Merge, And Setup Runner

**Labels:** `test`, `priority-critical`, `laravel`

- [ ] Link records project desired state.
- [ ] Link does not infer services from `.env`.
- [ ] Only declared env keys are written.
- [ ] pv-managed labels are added and preserved.
- [ ] Removed declarations stop future updates without deleting existing keys.
- [ ] Setup commands run from project root.
- [ ] Setup fails fast.
- [ ] Setup uses pinned PHP runtime on `PATH`.

### Test: Gateway, TLS, Routes, And pv open

**Labels:** `test`, `priority-critical`, `gateway`

- [ ] Primary host is deterministic.
- [ ] Aliases are represented in routes.
- [ ] Aliases are represented in certificate SAN material.
- [ ] Route rendering is deterministic.
- [ ] DNS integration is behind an adapter.
- [ ] Browser opening is behind an adapter.
- [ ] Gateway observed status includes route, cert, process, and failure info.

### Test: Laravel Helper Command Routing

**Labels:** `test`, `priority-high`, `laravel`

- [ ] Helper commands resolve current linked project.
- [ ] `pv artisan` uses pinned PHP runtime.
- [ ] Database helpers route to declared database resource.
- [ ] Mail helper errors clearly when Mailpit is not declared.
- [ ] S3 helper errors clearly when RustFS is not declared.

### Test: Status UX And Failure Reporting

**Labels:** `test`, `priority-critical`, `quality`

- [ ] Desired and observed state are visually distinct.
- [ ] Healthy state is clear.
- [ ] Stopped state is clear.
- [ ] Missing install includes next action.
- [ ] Blocked state includes next action.
- [ ] Crashed state includes log path.
- [ ] Partial reconciliation is visible.
- [ ] Stdout remains reserved for pipeable output.

### Test: MVP And Post-MVP Scope Guardrails

**Labels:** `test`, `priority-high`, `quality`

- [ ] Every PRD out-of-scope item is listed in backlog.
- [ ] Every backlog item has a deferral reason.
- [ ] Every backlog item has a reconsideration trigger.
- [ ] MVP feature issues do not contain hidden implementation of deferred work.
