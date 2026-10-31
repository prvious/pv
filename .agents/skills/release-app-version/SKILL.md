---
name: release-app-version
description: Build, publish, and verify PV application releases. Use when the user asks to bump the PV app version, run the PV App Release workflow, publish app binaries, update pv-app-manifest.json or install.sh, inspect app release workflow inputs, or release a new stable PV CLI version.
---

# Release App Version

Requested operation: $ARGUMENTS

Read [the app release guide](../../../docs/release/README.md), `CONTRIBUTING.md`,
and the release/update sections of `DESIGN.md` before preparing a release.
This is separate from Managed Resource artifact releases.

## Establish the release scope

- Inspect the exact ref's `.github/workflows/app-release.yml` and its inputs.
- Establish whether the request authorizes publication or only build preparation.
  Do not publish when only analysis, version preparation or a build was requested.
- Select the tested ref, `minimum_pv_version`, `app_platforms`, `helper_only`
  and `publish` explicitly. Use authorized user preferences; ask only when a
  material choice or authorization is unresolved. Report the choices.
- The current preview target is `darwin-arm64`; public v1 intends both macOS
  architectures. Do not represent preview validation as full public readiness.
- Never set `minimum_pv_version` below `0.2.6`: it is the required runtime-monitor
  migration. Re-read the checked-in default before choosing inputs.
- Check `git status --short --branch`; preserve unrelated work.

## Prepare and verify

Change only `[workspace.package].version` in root `Cargo.toml`, then run:

```sh
cargo update -p pv --precise <new-version>
```

All 11 app workspace packages inherit that version. Inspect `Cargo.lock` and
reject unrelated dependency changes. Do not change the independently versioned
helper for an ordinary app release. When changing the helper, keep its package
version and `PRIVILEGED_HELPER_VERSION` equal; protocol changes require a matching
app release. Helper-only releases retain the current stable app version.
For helper-only releases, select the current stable app's complete platform set;
equal-app-version publication rejects a platform-set change.

Focused checks:

```sh
cargo nextest run -p pv-release --test app_release_records
cargo nextest run -p pv-release --test app_publication
cargo nextest run -p pv-release --test workflow_defaults
cargo nextest run -p self-update --test app_update_manifest
cargo nextest run -p cli -E 'test(/^update::/)'
```

Before claiming the preparation is ready, complete the normal full verification:

```sh
cargo fmt --all --check
cargo insta pending-snapshots --workspace
git diff --check
cargo nextest run --workspace --all-features --locked --no-fail-fast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Inspect any snapshot changes and accept only expected drift through `cargo insta`;
rerun the full suite after acceptance. Commit only release-related files with a
Conventional Commit and use the normal PR/CI process.

## One build/publication run

After publication or build authorization is established, dispatch once with all
inputs explicit:

```sh
gh workflow run app-release.yml --ref <tested-ref> \
  -f minimum_pv_version=<minimum-pv-version> \
  -f app_platforms=<platforms> \
  -f helper_only=<true-or-false> \
  -f publish=<true-or-false>
gh run list --workflow app-release.yml --event workflow_dispatch \
  --commit <tested-commit-sha> --limit 5 \
  --json databaseId,headSha,createdAt,status,url
gh run watch <run-id> --exit-status --interval 30
```

Identify this dispatch by commit and creation time before selecting its run ID.

The publication job uses this run's handoff artifact and source run ID. There is
no second publication workflow or run ID to copy. `publish=false` only builds;
`publish=true` also publishes immutable objects, updates stable entrypoints, and
verifies their public bytes. It uses the existing configured destination, not an
implicit staging-to-public promotion.

Inspect failures before retrying. Rerun failed jobs on the same run to retain the
original artifacts. Never overwrite an immutable collision with different bytes.
Readback failure can occur after publication; inspect endpoints and the plan.
The plan artifact is `pv-app-publication-plan-<run-id>`. The build handoff is
`pv-app-release-<version>-<run-id>` and includes app/helper binaries, records,
`pv-app-manifest.json` and `install.sh`.

Native install/setup/update acceptance remains in the RC workflows and checklist;
successful public metadata readback is not runtime validation.

## Final report

Report the version, commit/ref, explicit inputs, local/CI checks, run URL and
conclusion, handoff evidence, publication/readback result if requested, and any
remaining RC or public-launch limitations.
