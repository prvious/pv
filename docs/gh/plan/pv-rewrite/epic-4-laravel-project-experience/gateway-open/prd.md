# Feature PRD: Gateway And pv open

## Epic

- Parent: [Epic 4 - Laravel Project Experience](../README.md)
- Architecture: [Epic 4 Architecture](../arch.md)

## Goal

**Problem:** Linked Laravel projects need a native HTTPS `.test` experience, but route, DNS, TLS, browser, and process behavior must be testable without mutating the developer machine during unit tests.

**Solution:** Add gateway desired/observed state, deterministic FrankenPHP/Caddy route rendering, TLS and DNS adapters, gateway process definition, route status, browser adapter, and `pv open`.

**Impact:** The MVP delivers the Herd replacement web workflow while preserving safe test boundaries.

## User Personas

- Laravel developer.
- Maintainer.

## User Stories

- As a Laravel developer, I want linked apps served at HTTPS `.test` hosts so that local development feels native.
- As a Laravel developer, I want aliases supported so that related local hosts route to the project.
- As a Laravel developer, I want `pv open` to open the linked app quickly.
- As a maintainer, I want DNS, TLS, and browser work behind adapters so that tests are safe.

## Requirements

### Functional Requirements

- Add gateway desired state with primary host, aliases, project path, and runtime reference.
- Add gateway observed state with route status and failures.
- Render deterministic FrankenPHP/Caddy route config.
- Model TLS material and SAN behavior.
- Add DNS and browser adapters.
- Submit gateway process definition to supervisor.
- Implement `pv open` using current-project resolution.

### Non-Functional Requirements

- FrankenPHP is gateway infrastructure, not a user-managed service.
- Unit tests must not mutate real DNS, trust stores, keychains, or browsers.
- Route output must be stable and diffable.

## Acceptance Criteria

- [ ] Linked project has primary `.test` host.
- [ ] Aliases appear in route and TLS SAN behavior.
- [ ] Gateway process definition runs through supervisor.
- [ ] Status can explain DNS, TLS, route, or process failures.
- [ ] `pv open` uses a browser adapter and actionable missing-state errors.

## Out Of Scope

- LAN sharing.
- Custom Caddy snippets.
- Mobile device access.
