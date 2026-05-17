# Epic 2: Store, Host, And Install Infrastructure

This directory contains the execution package for Epic 2 of the pv rewrite.

Epic 2 prevents architecture drift before more resources are added:

- canonical `~/.pv` filesystem layout;
- store schema and migration seams;
- contract-version decision point;
- install planner for runtimes, tools, and services;
- bounded downloads, dependency-ordered installs, atomic shim exposure;
- failure behavior that does not advertise incomplete work.

Legacy references:

- #112 - state migration and filesystem guardrails.
- #110 - scriptable install planner.

These are reference material only. Execute from this planning package.

## Documents

- `project-plan.md` - Epic 2 work hierarchy, dependencies, estimates, and risks.
- `implementation-plan.md` - task-by-task implementation sequence.
- `issues-checklist.md` - GitHub issue bodies for the Epic 2 hierarchy.
- `test-strategy.md` - Epic 2 test strategy and test issue plan.
- `qa-plan.md` - Epic 2 quality gates and acceptance checklist.
