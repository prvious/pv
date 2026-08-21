# Project Role Configuration

Store mutable orchestration preferences in the project, not in the installed skill and not in `solo.yml`.

## Contents

- [Files and Precedence](#files-and-precedence)
- [Format](#format)
- [Resolve a Role](#resolve-a-role)
- [Use the Helper](#use-the-helper)
- [Work Without the Helper](#work-without-the-helper)
- [Change Preferences Safely](#change-preferences-safely)

## Files and Precedence

Use these files at the project root:

- `.agents/orchestration.json` is shared team policy and may be committed.
- `.agents/orchestration.local.json` is an optional personal override and should remain untracked.

Resolve each setting in this order:

1. The user's explicit instruction for the current run.
2. The personal override.
3. The shared team policy.
4. Live Agent Tool discovery and, when still ambiguous, a user choice.

The live result of Solo `list_agent_tools` is always an availability gate. Configuration expresses preferences; it cannot make an unavailable runtime launchable.

## Format

Each file is a JSON object keyed by role name. Role names must begin with a lowercase letter and contain only lowercase letters, numbers, and hyphens. Every configured field must be a non-empty string.

Use a portable shared policy when teammates may have different providers:

```json
{
  "worker": {
    "agent_type": "any",
    "model": "inherit",
    "effort": "inherit"
  },
  "tester": {
    "agent_type": "any",
    "model": "inherit",
    "effort": "inherit"
  }
}
```

Use a personal override to bind roles to locally configured Agent Tool names:

```json
{
  "worker": {
    "agent_tool": "Claude"
  },
  "tester": {
    "agent_tool": "Codex",
    "effort": "high"
  }
}
```

Each role accepts only these fields:

- `agent_tool`: exact display name from Solo `list_agent_tools`.
- `agent_type`: runtime type from Solo, or `any` for a portable selection.
- `model`: model preference, or `inherit` to use the Agent Tool default.
- `effort`: reasoning-effort preference, or `inherit` to use the Agent Tool default.

The personal file may override any subset of a shared role. A role without `agent_tool` or `agent_type` uses live Agent Tool selection.

Do not store numeric Agent Tool IDs, process IDs, secrets, arbitrary command-line arguments, or permission-bypass flags. IDs are local and ephemeral; discover them before every launch.

## Resolve a Role

For each requested role:

1. Merge the personal role fields over the shared role fields.
2. Prefer an exact `agent_tool` name match.
3. Otherwise match `agent_type` against Solo's returned runtime type.
4. For `agent_type: "any"` or a missing selector, use the sole available runtime or ask when several materially different choices remain.
5. Prefer a project-runnable installation when Solo exposes installation choices.
6. If no configured match exists, explain the mismatch and offer the live alternatives without changing either file.

Apply `model` and `effort` only through launch arguments explicitly documented by the selected Agent Tool. These fields do not authorize guessing provider flags.

## Use the Helper

The bundled JavaScript helper validates, merges, and atomically writes these files without external packages. Resolve `scripts/orchestration-config.mjs` relative to the installed skill root, not the project working directory. Use the first installed runtime in this order: Node.js, Bun, then Deno. Do not install a runtime solely for the helper unless the user asks.

Run it with Node.js 18.3 or newer:

```text
node scripts/orchestration-config.mjs read --project-root /path/to/project
node scripts/orchestration-config.mjs write team --project-root /path/to/project < proposed-team.json
node scripts/orchestration-config.mjs write local --project-root /path/to/project < proposed-local.json
```

With Bun, replace `node` with `bun`; the commands and behavior are otherwise identical.

With Deno 2, grant only the permissions needed by the operation:

```text
deno run --allow-read=/path/to/project scripts/orchestration-config.mjs read --project-root /path/to/project
deno run --allow-read=/path/to/project --allow-write=/path/to/project scripts/orchestration-config.mjs write team --project-root /path/to/project < proposed-team.json
deno run --allow-read=/path/to/project --allow-write=/path/to/project --allow-run=git scripts/orchestration-config.mjs write local --project-root /path/to/project < proposed-local.json
```

Always pass an explicit project root. The helper rejects unknown fields and validates the effective merged configuration before writing. Running `read` also validates both files.

Writing the personal file adds `/.agents/orchestration.local.json` to the clone-local Git exclude. It does not alter the project's committed `.gitignore`. Outside a Git worktree, the helper writes the file and reports that no exclusion was added.

## Work Without the Helper

If Node.js, Bun, and Deno are unavailable:

1. Parse each existing file as a JSON object keyed by valid role name.
2. Reject fields outside `agent_type`, `agent_tool`, `model`, and `effort`, and require non-empty string values.
3. Merge personal role fields over matching shared role fields.
4. Show and obtain approval for persistent changes before editing either file.
5. Keep `.agents/orchestration.local.json` untracked using the path returned by `git rev-parse --git-path info/exclude` when Git is available.

Do not skip validation merely because the helper runtime is unavailable.

## Change Preferences Safely

Never write either file merely because a runtime was used successfully.

1. Ask whether the choice is for this run, the current user, or the team.
2. Show the exact proposed JSON or diff.
3. Obtain explicit approval for a persistent change.
4. Use the helper to validate and write it.
5. For a team change, leave the resulting Git diff for normal review and commit.
