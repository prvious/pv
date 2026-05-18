# Epic 3: Runtime, Daemon, And Resources

This package is the focused execution plan for Epic 3 of the pv rewrite.

Epic 3 turns the control-plane and install infrastructure into real managed
local resources: PHP, Composer, the daemon loop, the resource-agnostic
supervisor, Mailpit, Postgres, MySQL, Redis, and RustFS.

## Documents

- `project-plan.md` - Epic 3 work hierarchy, dependencies, risks, and done rules.
- `arch.md` - Epic 3 architecture specification.
- `technical-breakdown.md` - module roles, command contracts, and resource status requirements.
- `implementation-plan.md` - implementation sequence and package guidance.
- `issues-checklist.md` - GitHub issue bodies and publishing tracker.
- `test-strategy.md` - focused test strategy for runtime, daemon, and resources.
- `test-issues-checklist.md` - concrete test coverage for #151, #156, #161, and #165.
- `qa-plan.md` - review gates and manual QA expectations.
- `php-runtime-composer-tooling/prd.md` - Feature 3.1 PRD.
- `daemon-supervisor-mailpit/prd.md` - Feature 3.2 PRD.
- `stateful-database-resources/prd.md` - Feature 3.3 PRD.
- `cache-mail-object-storage-resources/prd.md` - Feature 3.4 PRD.

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
