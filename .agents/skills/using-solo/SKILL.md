---
name: using-solo
description: Always use this skill at the start of every conversation and before responding to or acting on any user request. Determine whether the current process is a Solo-managed lead. When confirmed, make Solo the active delegation facility and apply its orchestration workflow. If Solo MCP is unavailable or the process is not Solo-managed, continue normally. Skip this skill only when the agent was dispatched as a bounded worker.
license: MIT
compatibility: Requires Solo MCP for managed-session detection; delegation requires at least one configured CLI Agent Tool. Automatic start-of-conversation activation depends on the host supporting implicit skill invocation. The gate exits quietly when Solo MCP is unavailable. The optional configuration helper runs on Node.js 18.3+, Bun, or Deno 2 with scoped project and Git permissions.
metadata:
    short-description: "Always apply before responding or acting: detect Solo-managed leads, route delegation through Solo when confirmed, and skip bounded workers."
    author: "Clovis Muneza"
    version: "0.1.0"
    tags: "solo, mcp, orchestration, multi-agent, coding-agents, delegation, session-detection"
---

# Using Solo

<SUBAGENT-STOP>
If a parent agent dispatched you to execute a bounded task, ignore this skill and complete that assignment directly. Proceed only when the parent explicitly assigned you the lead or orchestrator role.
</SUBAGENT-STOP>

## Check the Session

Perform this check before responding, asking clarifying questions, inspecting files, or taking any other action.

1. If Solo `whoami` is unavailable, stop using this skill and continue normally.
2. Call Solo `whoami` without arguments.
3. If it identifies this MCP session as a Solo-managed process, continue to [Route the Work](#route-the-work).
4. If identification fails, inspect this process's `SOLO_PROCESS_ID`. When it contains a Solo process ID, pass that value as `solo_process_id` to `identify_session`, then call `whoami` again.
5. If the session is still not identified as a Solo-managed process, stop using this skill and continue normally.

Use `SOLO_PROCESS_ID` only to identify the current process. Never pass another process's ID, treat it as an operating-system PID, or register an external identity merely to activate this skill. An effective project scope alone does not prove that the current process is Solo-managed.

## Route the Work

For a confirmed Solo-managed lead, read [references/orchestration.md](references/orchestration.md) completely before responding or acting, then apply it to the current request.

Solo is the active delegation facility for that request. Fulfill delegation required by other skills through the Solo workflow rather than also launching the same assignments through the host's built-in delegation facility.

Read [references/configuration.md](references/configuration.md) only when selecting configured roles or changing remembered role preferences.

Applying the orchestration workflow does not require spawning an agent. Keeping simple or tightly coupled work with the lead is a valid orchestration decision.
