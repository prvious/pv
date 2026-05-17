# Epic 1: Rewrite Foundation

This directory contains the execution package for Epic 1 of the pv rewrite.

Epic 1 proves the rewrite can stand on its own:

- the prototype is isolated as reference-only;
- the root module becomes the active rewrite workspace;
- the command layer is minimal and scriptable;
- the first control-plane tracer proves command -> desired state -> controller
  -> observed status.

Legacy references:

- #97 - move prototype implementation to `legacy/prototype`.
- #98 - scaffold active root module and guardrails.
- #99 - build first control-plane resource tracer.
- #114 - reference PR covering the same early slice.

These are reference material only. Execute from this planning package.

## Documents

- `project-plan.md` - Epic 1 work hierarchy, milestones, estimates, and risks.
- `arch.md` - Epic 1 architecture specification.
- `technical-breakdown.md` - module roles, flows, and boundaries.
- `implementation-plan.md` - task-by-task implementation sequence.
- `issues-checklist.md` - GitHub issue bodies for the Epic 1 hierarchy.
- `test-strategy.md` - Epic 1 test strategy and test issue plan.
- `test-issues-checklist.md` - concrete test coverage for #122 and #127.
- `qa-plan.md` - Epic 1 quality gates and acceptance checklist.
- `prototype-isolation-root-scaffold/prd.md` - Feature 1.1 PRD.
- `first-desired-state-resource-tracer/prd.md` - Feature 1.2 PRD.
