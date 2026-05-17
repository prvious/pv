# Legacy Prototype Relocation Design

## Goal

Move the current prototype implementation out of the repository root before the rewrite starts, so the new architecture can use root-level package names such as `cmd/` and `internal/` without mixing old and new code.

The old implementation should remain buildable and testable for reference.

## Decision

Move the current Go application into:

```text
legacy/prototype/
```

This directory is a buildable snapshot of the old prototype, not the active product architecture.

## What Moves

Move the current implementation as a complete Go module:

- `go.mod`
- `go.sum`
- `main.go`
- `cmd/`
- `internal/`
- prototype-specific build and release configuration
- prototype-specific scripts
- prototype-specific README content

The module may keep the original module path so old imports continue to compile inside the nested module.

## What Stays At Root

Keep repository coordination and rewrite material at the root:

- agent instructions
- issue-tracker/domain docs
- rewrite PRD and architecture docs
- root `.gitignore`
- workspace/tooling files that apply to the whole repository

The root becomes the home for the new rewrite module once scaffolding starts.

## Rules

- The prototype is reference-only.
- New rewrite code must not import prototype packages.
- Behavior and tests may be copied forward deliberately, but shared code between prototype and rewrite is not allowed.
- Any agent or human should be able to tell which tree is active by reading the root README and rewrite docs.
- Build commands for the old prototype should run from `legacy/prototype/`.

## Success Criteria

- The root no longer contains the old `cmd/`, `internal/`, `main.go`, `go.mod`, or `go.sum`.
- `legacy/prototype/` contains the old Go module and can still build independently.
- Root rewrite docs explain that `legacy/prototype/` is the old implementation.
- The first implementation issue in the rewrite breakdown handles this relocation before any new code scaffold is created.

## Out Of Scope

- Rewriting imports or changing old prototype architecture.
- Migrating old state or storage paths.
- Creating the new root Go module.
- Moving prototype code into packages shared by the rewrite.
