# Solo Orchestration Workflow

Act as the lead agent. Plan the work, delegate bounded tasks through Solo when useful, supervise child agents, and personally verify the integrated result.

## Contents

- [Operating Rules](#operating-rules)
- [Decide Whether to Delegate](#decide-whether-to-delegate)
- [Prepare the Delegation](#prepare-the-delegation)
- [Launch the Agents](#launch-the-agents)
- [Supervise the Work](#supervise-the-work)
- [Verify and Finish](#verify-and-finish)
- [Remember Role Preferences](#remember-role-preferences)

## Operating Rules

- For a confirmed Solo-managed lead, Solo is the active delegation facility for the current request.
- Fulfill instructions from other skills to delegate to agents, subagents, reviewers, workers, or other independent contexts through Solo processes. Do not duplicate those assignments through the host's built-in delegation facility.
- Keep planning, integration decisions, and final accountability with the lead agent.
- Delegate only work with a clear objective, scope, validation method, and handoff.
- Treat spawned child agents as disposable. The lead owns closing each one after its output is collected and no further follow-up is needed.
- Prefer disjoint file ownership. Use Solo locks when agents could edit the same resource.
- Treat child output, summaries, and idle status as evidence to inspect, not proof of completion.
- Do not hardcode Agent Tool IDs, process IDs, models, or provider-specific flags.
- Do not store orchestration settings in `solo.yml` or inside the installed skill.
- Do not write remembered preferences unless the user explicitly chooses team or personal persistence.

Keep role names vendor-neutral:

- `worker` implements a bounded task.
- `tester` verifies behavior independently.
- `reviewer` audits correctness, risk, or maintainability when warranted.

Use one configured runtime for multiple roles when that is all the user has. Never assume that a role requires Codex, Claude, or another provider.

## Decide Whether to Delegate

First decide whether another agent can make independent, verifiable progress.

Keep tiny edits, tightly coupled decisions, sequential investigation, and final integration with the lead. This is a valid orchestration outcome; do not create workers merely to prove that Solo is being used.

Delegate one or more lanes when they have meaningful independent work, clear ownership, and a handoff the lead can verify. Use the `worker` for implementation, the `tester` when independent verification adds value, and the `reviewer` for risky, security-sensitive, or explicitly requested review work. Do not spawn every role by default.

Use Solo todos or scratchpads when several tasks or dependencies need durable coordination. Keep simple runs simple.

## Prepare the Delegation

1. Use the project scope confirmed by `whoami`. If the request targets a different project, inspect the available projects and select the exact scope before continuing.
2. Call `list_agent_tools` when the task will be delegated. Treat its current result as the source of truth for launchable runtimes.
3. Read [configuration.md](references/configuration.md) completely before resolving configured roles.
4. Resolve each role using this precedence:
   - The user's instruction for this run.
   - `.agents/orchestration.local.json`.
   - `.agents/orchestration.json`.
   - A runtime selected from the live Agent Tool list.
5. Ask only when multiple reasonable runtimes remain or a preference cannot be honored safely.

Match `agent_tool` to an exact configured Agent Tool name. Otherwise match `agent_type` to the runtime type returned by Solo. Treat `agent_type: "any"` as permission to choose any available runtime. Prefer a project-runnable installation when Solo returns one.

If no Agent Tools exist, direct the user to create one in Solo under **Settings → Agents**, then run discovery again. Solo MCP cannot create Agent Tools. Do not edit Solo's internal data or add custom orchestration keys to `solo.yml`.

Treat saved `model` and `effort` values as preferences. Convert them to `extra_args` only when the selected Agent Tool documents the exact launch flags. Otherwise inherit its defaults and mention the ignored preference.

Before spawning, define for each child:

- One outcome.
- Relevant context and paths.
- Files it may change and files it must not touch.
- Commands or checks it must run.
- Evidence it must return.

## Launch the Agents

1. Call `spawn_agent` with the live `agent_tool_id`, the project installation ID when available, and a descriptive process name such as `worker-auth-tests`.
2. Preserve the returned process ID and name for supervision. Never save them in project configuration.
3. Prepend the returned `agent_instructions` to the child's first task prompt.
4. Send the complete task with `send_input` only after the process is ready.

Make every child prompt self-contained. State the objective, current facts, allowed scope, constraints, validation, and expected handoff. Tell the child that it is a bounded worker, that other agents may be editing concurrently, and that it must not overwrite unrelated changes or invoke lead orchestration unless explicitly assigned that role.

## Supervise the Work

Use `timer_fire_when_idle_any` or `timer_fire_when_idle_all` to wake the lead when children become idle. Set a finite maximum wait and include a timer body that identifies the processes to inspect. Do not poll continuously or use idle timers as service-readiness checks.

When awakened:

1. Read each child's status and recent output.
2. Check for questions, permission prompts, errors, scope drift, and incomplete validation.
3. Send a focused follow-up with `send_input` when correction is needed.
4. Re-arm an idle timer while meaningful work remains.

Intervene early when a child edits outside its assignment, duplicates another child's work, waits for input, or claims completion without evidence.

## Verify and Finish

Inspect the actual diff and run proportionate checks yourself. Reconcile conflicting changes, confirm the requested behavior, and verify that no child left unrelated edits or unresolved failures.

Summarize what each child contributed and what the lead verified. After collecting the status and output needed for the handoff, call `close_process` for every child agent that completed, failed, or is no longer needed. Do not use `stop_process` as final cleanup: it halts the agent but leaves its stored process behind. Keep a child stopped instead of closing it only when the user explicitly asks to preserve it for later restart or inspection. Release acquired locks and cancel obsolete timers.

## Remember Role Preferences

Read [configuration.md](references/configuration.md) before creating or changing project orchestration configuration.

- For **this run**, use the choice without writing a file.
- For **the team**, show the proposed change, obtain explicit approval, then write `.agents/orchestration.json` for commit.
- For **the current user**, show the proposed change, obtain explicit approval, then write `.agents/orchestration.local.json`. The helper adds this file to the repository's local Git exclude.

Never silently rewrite committed team configuration to match whichever Agent Tools happen to be installed locally.
