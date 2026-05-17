# MVP Scope Checklist

Use this checklist when creating or reviewing rewrite work.

- [ ] The work maps to one published MVP issue between #116 and #205.
- [ ] The work preserves the control-plane rule: commands request state, controllers reconcile, supervisor runs processes, store is authority.
- [ ] The work is Laravel-first or directly supports a Laravel-first MVP dependency.
- [ ] The work does not infer service, env, setup, or migration behavior from `.env`.
- [ ] The work does not require a post-MVP backlog capability.
- [ ] If a deferred capability is touched, the PR updates `post-mvp-backlog.md` instead of expanding MVP scope silently.
- [ ] The work has a linked test issue or a documented manual QA check.
- [ ] The PR body lists exact verification commands run.
