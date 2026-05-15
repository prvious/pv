# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Layout

This is a single-context repo.

Use these files when present:

- `CONTEXT.md` at the repo root for project domain language.
- `docs/adr/` for architectural decision records.

If these files do not exist, proceed silently. Do not flag their absence or create them unless the current task is specifically about domain docs or ADRs.

## Use the glossary's vocabulary

When output names a domain concept, use the term as defined in `CONTEXT.md`. Avoid drifting to synonyms the glossary explicitly avoids.

If the concept is missing, note the gap only when it affects the task.

## Flag ADR conflicts

If output contradicts an existing ADR, surface it explicitly rather than silently overriding the decision.
