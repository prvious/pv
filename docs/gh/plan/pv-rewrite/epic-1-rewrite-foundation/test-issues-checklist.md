# Test Issues Checklist: Epic 1 - Rewrite Foundation

## Test Issue #122: Root/Prototype Build And CLI Checks

Labels: `test`, `priority-high`, `control-plane`, `ready-for-agent`

Blocked by: #119, #120, #121.

Required coverage:

- [ ] Root `go build ./...` passes from repository root.
- [ ] Root `go test ./...` passes from repository root.
- [ ] Prototype `go build ./...` passes from `legacy/prototype` when prototype files move or change.
- [ ] Prototype `go test ./...` passes from `legacy/prototype` when prototype files move or change.
- [ ] `help` writes usage to stdout.
- [ ] `version` writes pipeable version output to stdout.
- [ ] Unknown command writes the message and usage hint to stderr.
- [ ] Unknown command returns a non-zero error.
- [ ] Fang is absent from the root module unless a later issue explicitly adds it.

Exit evidence:

- [ ] Test names are listed in the PR body.
- [ ] Exact verification commands are listed in the PR body.

## Test Issue #127: Control-Plane Tracer Behavior

Labels: `test`, `priority-high`, `control-plane`, `ready-for-agent`

Blocked by: #123, #124, #125, #126.

Required coverage:

- [ ] Desired state write persists the requested Mago version.
- [ ] Desired state write does not create observed status.
- [ ] Observed status write does not mutate desired state.
- [ ] Invalid versions are rejected before persistence.
- [ ] Corrupted store data returns a clear error.
- [ ] Mago controller no-ops when desired state is absent.
- [ ] Mago controller writes ready observed status on fake install success.
- [ ] Mago controller writes failed observed status and next action on fake install failure.
- [ ] `mago:install <version>` writes desired state only.
- [ ] `status` distinguishes no desired state, pending, ready, and failed.
- [ ] Tests use deterministic clocks.
- [ ] Tests do not download real artifacts.

Exit evidence:

- [ ] Store, CLI, controller, and status tests are present.
- [ ] Root verification passes.
