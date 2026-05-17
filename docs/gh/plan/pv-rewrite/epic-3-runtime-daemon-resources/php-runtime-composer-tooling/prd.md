# Feature PRD: PHP Runtime And Composer Tooling

## Epic

- Parent: [Epic 3 - Runtime, Daemon, And Resources](../README.md)
- Architecture: [Epic 3 Architecture](../arch.md)

## Goal

**Problem:** Laravel projects must not depend on system PHP or Composer behavior. Composer also cannot be treated as independent from the PHP runtime that executes it.

**Solution:** Add managed PHP runtime desired state, a PHP controller, Composer desired state with explicit PHP dependency, and runtime-aware shims.

**Impact:** Later Laravel init, link, setup, gateway, and helper work can rely on deterministic managed runtime paths.

## User Personas

- Laravel developer.
- Automation user.
- Maintainer.

## User Stories

- As a Laravel developer, I want `php:install <version>` to request a managed PHP runtime so that my project does not depend on system PHP.
- As a Laravel developer, I want Composer to run through a selected managed PHP runtime so that installs are reproducible.
- As a maintainer, I want missing runtime dependencies to produce blocked status so that failures are actionable.

## Requirements

### Functional Requirements

- Add PHP runtime desired state and controller.
- Add `php:install <version>` desired-state command.
- Add Composer desired state with required PHP runtime version.
- Add Composer controller dependency checks.
- Add `composer:install <version> --php <php-version>` desired-state command.
- Expose PHP and Composer shims atomically.
- Extend status for missing, blocked, ready, and failed runtime/tool states.

### Non-Functional Requirements

- No implicit system PHP fallback.
- Shims must point at managed paths.
- Tests must use fake installers and temp roots.

## Acceptance Criteria

- [ ] PHP desired state persists requested version.
- [ ] PHP controller reconciles through canonical runtime paths.
- [ ] Composer records required PHP runtime version.
- [ ] Missing PHP runtime creates blocked Composer status with next action.
- [ ] PHP and Composer shims are atomic and runtime-aware.
- [ ] Tests prove system PHP is not used implicitly.

## Out Of Scope

- Project contract parsing.
- Setup runner.
- Per-project extensions or Xdebug.
