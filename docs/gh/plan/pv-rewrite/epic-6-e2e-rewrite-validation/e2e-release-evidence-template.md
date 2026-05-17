# E2E Release Evidence Template

Use this template when validating Epic 6 release readiness.

## Tier 0 Required Gate

- Command:
- Branch:
- Base branch:
- Commit:
- Started at:
- Finished at:
- Result:

## Scenario Evidence

| Scenario | Expected result | Actual result | Log path | Follow-up issue |
| --- | --- | --- | --- | --- |
| Harness isolation | | | | |
| Init fresh Laravel project | | | | |
| Init overwrite refusal | | | | |
| Link declared project | | | | |
| Aggregate and targeted status | | | | |
| Helper commands | | | | |
| Missing install blocked state | | | | |
| Setup failure | | | | |
| Runnable process crash | | | | |
| Gateway failure | | | | |
| Recovery | | | | |

## CI Tier 1 Local Process Checks

- GitHub run:
- Command:
- Resource names and versions:
- Temp directories:
- Ports used:
- Cleanup performed:
- Result:

## CI Tier 2 Privileged Host Checks

- GitHub run:
- Command:
- Host actions:
- Files changed:
- Trust-store or keychain actions:
- Browser action:
- Cleanup performed:
- Result:
