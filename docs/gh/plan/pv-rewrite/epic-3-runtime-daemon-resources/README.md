# Epic 3: Runtime, Daemon, And Resources

This package is the focused execution plan for Epic 3 of the pv rewrite.

Epic 3 turns the control-plane and install infrastructure into real managed
local resources: PHP, Composer, the daemon loop, the resource-agnostic
supervisor, Mailpit, Postgres, MySQL, Redis, and RustFS.

## Documents

- `project-plan.md` - Epic 3 work hierarchy, dependencies, risks, and done rules.
- `implementation-plan.md` - implementation sequence and package guidance.
- `issues-checklist.md` - GitHub issue bodies and publishing tracker.
- `test-strategy.md` - focused test strategy for runtime, daemon, and resources.
- `qa-plan.md` - review gates and manual QA expectations.

## Legacy Reference

- #100 - service/resource shape reference.
- #101 - PHP runtime reference.
- #102 - Composer/tooling reference.
- #103 - daemon/supervisor reference.
- #104 - database/resource reference.
- #105 - cache/mail/object storage reference.
- #115 - reference PR for PHP and Composer tracer work.

The legacy issues and PR are source material only. New work should use this
Epic -> Feature -> Story/Enabler -> Test hierarchy.
