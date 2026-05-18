# E2E Release Evidence Template

Use this template when validating Epic 6 release readiness.

## Release Metadata

- PR:
- Branch:
- Base branch:
- Commit:
- Recorded by:
- Recorded at:
- GitHub run:
- Overall result:

## Tier 0 Required Gate

- Tier: Tier 0
- Command: `go test ./test/e2e/scenarios -count=1`
- Started at:
- Finished at:
- Log path:
- Artifact path:
- Result:

## Tier 0 Scenario Evidence

Use the full Tier 0 gate command for release approval. The targeted commands below are for scenario-level reruns and failure follow-up.

| Tier | Scenario | Command | Expected result | Actual result | Log path | Artifact path | Follow-up issue link |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Tier 0 | Harness isolation support | `go test ./test/e2e/harness -run TestNewSandboxIsolatesPvPathsUnderTempRoot -count=1` | | | | | |
| Tier 0 | Init fresh Laravel project | `go test ./test/e2e/scenarios -run TestPvInitLifecycle -count=1` | | | | | |
| Tier 0 | Init overwrite refusal | `go test ./test/e2e/scenarios -run TestPvInitLifecycle -count=1` | | | | | |
| Tier 0 | Link declared project | `go test ./test/e2e/scenarios -run TestPvLinkEnvSetupLifecycle -count=1` | | | | | |
| Tier 0 | Aggregate and targeted status | `go test ./test/e2e/scenarios -run TestPvStatusAndHelperWorkflows -count=1` | | | | | |
| Tier 0 | Helper commands | `go test ./test/e2e/scenarios -run TestPvStatusAndHelperWorkflows -count=1` | | | | | |
| Tier 0 | Missing install blocked state | `go test ./test/e2e/scenarios -run TestPvMissingInstallAndBlockedDependencyFailures -count=1` | | | | | |
| Tier 0 | Setup failure | `go test ./test/e2e/scenarios -run TestPvSetupProcessAndGatewayFailureEvidence -count=1` | | | | | |
| Tier 0 | Runnable process crash | `go test ./test/e2e/scenarios -run TestPvSetupProcessAndGatewayFailureEvidence -count=1` | | | | | |
| Tier 0 | Gateway failure | `go test ./test/e2e/scenarios -run TestPvSetupProcessAndGatewayFailureEvidence -count=1` | | | | | |
| Tier 0 | Recovery | `go test ./test/e2e/scenarios -run TestPvRecoveryAfterCorrectiveAction -count=1` | | | | | |

## CI Tier 1 Local Process Checks

- GitHub run:
- Tier: CI Tier 1
- Command: `go test -tags=e2e_tier1 ./test/e2e/tier1 -count=1`
- Resource names and versions:
- Temp directories:
- Ports used:
- Cleanup performed:
- Log path:
- Artifact path:
- Result:

| Tier | Scenario | Command | Expected result | Actual result | Log path | Artifact path | Follow-up issue link |
| --- | --- | --- | --- | --- | --- | --- | --- |
| CI Tier 1 | Tier 1 guard | `go test -tags=e2e_tier1 ./test/e2e/tier1 -count=1` | | | | | |

## CI Tier 2 Privileged Host Checks

- GitHub run:
- Tier: CI Tier 2
- Command: `go test -tags=e2e_tier2 ./test/e2e/tier2 -count=1`
- Host actions:
- Files changed:
- Trust-store or keychain actions:
- Browser action:
- Cleanup performed:
- Log path:
- Artifact path:
- Result:

| Tier | Scenario | Command | Expected result | Actual result | Log path | Artifact path | Follow-up issue link |
| --- | --- | --- | --- | --- | --- | --- | --- |
| CI Tier 2 | Tier 2 guard | `go test -tags=e2e_tier2 ./test/e2e/tier2 -count=1` | | | | | |

## Skipped Tier Evidence

Use this section for any tier or scenario that was intentionally skipped.

| Tier | Scenario | Command | Reason skipped | Expected result | Actual result | Log path | Artifact path | Follow-up issue link |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| | | | | | | | | |

## Failure Follow-ups

Every failed or skipped scenario must link to a follow-up issue before release approval.

| Tier | Scenario | Failure or skip summary | Follow-up issue link | Owner | Status |
| --- | --- | --- | --- | --- | --- |
| | | | | | |
