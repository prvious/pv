---
name: pr-review-toolkit
description: "Orchestrates six specialized agents for pull requests and code changes: comments, tests, silent failures, type design, general code quality, and simplification. Selects the applicable specialists, delegates each through the session's active delegation facility, runs advisory review concurrently by default, and aggregates the reports. Use for PR reviews, code reviews, pre-commit or pre-merge checks, targeted review concerns, and proactively after changing code, tests, comments, error handling, or types."
license: Apache-2.0; see LICENSE
compatibility: Requires git and an active delegation facility capable of launching independent agent contexts, either through the host or a session-level orchestration workflow. A hosting CLI or API such as gh is needed only when the review scope must be resolved from a hosted pull request.
metadata:
  author: "Anthropic; adapted by Prvious"
  source: "https://github.com/anthropics/claude-plugins-official/tree/main/plugins/pr-review-toolkit"
  source-commit: "d33732b67aef18b3409d2c566e1d15541906726d"
---

# PR Review Toolkit

Coordinate six specialists, each focused on a distinct aspect of code quality. Determine which specialists apply, delegate them to independent agent contexts, and aggregate their results into a comprehensive report.

> Adapted from Anthropic's `commands/review-pr.md` and agent prompts; see `NOTICE` and `LICENSE`. The specialist prompts and review policies are preserved, while this port runs applicable advisory reviewers concurrently by default.

## Define Specialists and Delegation

A **specialist** is an independently running agent, session, or agent-backed process with its own context, assigned exactly one toolkit role. The general code reviewer, test analyzer, silent-failure hunter, comment analyzer, and type-design analyzer are advisory specialists. The code simplifier is a mutation-capable specialist that runs separately from advisory review.

Context independence does not imply filesystem isolation. Specialists may share a working tree. Advisory specialists must not modify files; only the code simplifier may edit files within this workflow.

Use the session's **active delegation facility**:

1. If a session-level orchestration workflow has selected a delegation facility, it takes precedence.
2. Otherwise use the host's built-in facility for independent agent contexts.
3. Never dispatch the same assignment through more than one delegation facility.
4. If no available facility can provide genuinely independent contexts, explain that the compatibility requirement is not satisfied. Do not simulate multiple specialists in the lead context.

## Preserve Specialist Behavior

Preserve the upstream specialist behavior while adapting packaging and runtime-specific mechanics:

- Translate Claude's `Agent` delegation (`Task` in the upstream source) into the active delegation facility.
- Treat `CLAUDE.md` as the upstream name for applicable project instructions. Also read equivalent instruction files such as `AGENTS.md` when the runtime or repository uses them.
- Use the strongest available model for the general code reviewer and code simplifier when the active facility supports model selection. Other reviewers inherit the lead's model when supported; otherwise use the facility's configured defaults.
- Preserve each specialist's philosophy, thresholds, output contract, and mutation behavior. Do not normalize them into a different review policy.
- Do not replace independent specialists with multiple perspectives in one context.

## Choose the Invocation Mode

Interpret the user's request as one or more review aspects:

- `comments`: analyze comment accuracy and maintainability;
- `tests`: review test coverage quality and completeness;
- `errors`: hunt for silent failures and inadequate error handling;
- `types`: analyze type design and invariants;
- `code`: perform general review against project guidelines;
- `simplify`: simplify changed code while preserving functionality;
- `all`: run every applicable reviewer.

When the user names aspects, run those aspects. Otherwise, a comprehensive PR review defaults to `all` applicable reviewers.

The toolkit may also activate proactively after a logical chunk of work:

- after code changes: general code reviewer, followed by code simplifier;
- after tests or behavior changes: test analyzer;
- after error-handling or fallback changes: silent-failure hunter;
- after comments or documentation changes: comment analyzer;
- after types or data models change: type-design analyzer;
- before a commit or pull request: all applicable reviewers.

## Determine the Review Scope

1. Honor a user-specified pull request, branch, commit range, file list, or aspect.
2. Otherwise inspect `git status` and `git diff --name-only` to identify recent changes.
3. When a hosted pull request may exist, use the available hosting tool, such as `gh pr view`, to resolve its base, head, and changed files.
4. Tell every specialist exactly which files or diff to review. Do not let specialists silently choose different scopes.
5. Focus on the changed code rather than reviewing the entire repository unless the user explicitly asks for a broader audit.

## Select Applicable Reviewers

For `all` or an aspect-unspecified comprehensive review, apply the upstream selection rules:

- Always run the **general code reviewer**.
- Run the **test analyzer** if test files changed.
- Run the **comment analyzer** if comments or documentation were added or changed.
- Run the **silent-failure hunter** if error handling, catch blocks, or fallback behavior changed.
- Run the **type-design analyzer** if types or data models were added or modified.
- Run the **code simplifier** after the review passes.

An explicitly requested reviewer runs even when its normal change-based trigger is absent.

## Delegate to Independent Specialists

Create one independent specialist context for every selected role. Give it:

- the exact review scope;
- the applicable repository instructions;
- the corresponding reference file;
- relevant pull-request context;
- any user-requested focus.

Use these assignments:

| Specialist | Reference |
| --- | --- |
| General code reviewer | [general-review.md](references/general-review.md) |
| Test analyzer | [test-coverage.md](references/test-coverage.md) |
| Silent-failure hunter | [error-handling.md](references/error-handling.md) |
| Comment analyzer | [comments.md](references/comments.md) |
| Type-design analyzer | [type-design.md](references/type-design.md) |
| Code simplifier | [simplification.md](references/simplification.md) |

Dispatch all selected advisory specialists as one concurrent review stage by default. Respect the active delegation facility's concurrency limit; when capacity is lower than the number of selected specialists, queue the remaining specialists without merging their contexts. Honor an explicit request for sequential execution.

Use the active facility's normal foreground or non-blocking execution mode to realize the concurrent stage. When the facility lets the lead continue while advisory specialists run, do useful, non-overlapping work. Use its normal completion signal or wake-up mechanism instead of polling continuously. Collect every advisory report before deciding whether the review passes.

Never run the code simplifier alongside advisory specialists. Its upstream role is to refine code only after review passes, except when the user explicitly requests `simplify` or the skill invokes it directly after coding.

## Preserve Specialist Output Contracts

Do not discard specialist details while aggregating:

- **General code reviewer**: report only issues with confidence at least 80; retain its 0–100 confidence score and Critical (90–100) or Important (80–89) grouping.
- **Test analyzer**: retain 1–10 criticality ratings, the failure each proposed test would catch, and its Critical Gaps, Important Improvements, Test Quality Issues, and Positive Observations sections.
- **Silent-failure hunter**: retain location, severity, issue description, specific hidden errors, user impact, recommendation, and corrected example.
- **Comment analyzer**: retain Critical Issues, Improvement Opportunities, Recommended Removals, and Positive Findings.
- **Type-design analyzer**: retain per-type invariants plus 1–10 ratings for encapsulation, invariant expression, invariant usefulness, and invariant enforcement.
- **Code simplifier**: preserve exact functionality, edit recently changed code proactively, and summarize only significant changes.

## Preserve Simplifier Mutation Behavior

The code simplifier is not an advisory review lens. When selected, it applies behavior-preserving refinements to the recently modified code.

For an `all` review, run it after the other selected reviewers complete and the review passes. For an explicit `simplify` request or proactive post-coding invocation, run it directly. Follow the project standards and the complete instructions in [simplification.md](references/simplification.md).

After simplification, inspect the resulting diff and run the relevant verification before reporting completion.

## Aggregate Results

After the specialists complete:

1. Keep each specialist's required scores, evidence, and details.
2. Deduplicate overlapping findings without dropping specialist-specific information.
3. Organize the combined result into this summary:

```markdown
# PR Review Summary

## Critical Issues (X found)
- [reviewer]: Issue description [file:line]

## Important Issues (X found)
- [reviewer]: Issue description [file:line]

## Suggestions (X found)
- [reviewer]: Suggestion [file:line]

## Strengths
- What is well done in this pull request

## Recommended Action
1. Fix critical issues first
2. Address important issues
3. Consider suggestions
4. Re-run the relevant reviewers after fixes
```

4. Include the detailed specialist reports after the summary or preserve their required fields within the corresponding aggregated findings.
5. If a reviewer reports no issues, retain its positive observations or state that the aspect passed.
6. Re-run targeted reviewers after fixes to verify that their findings are resolved.
