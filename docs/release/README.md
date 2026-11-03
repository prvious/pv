# Releasing PV

An ordinary PV app release needs one version change and one workflow dispatch.
Managed Resource releases are separate: only rebuild PHP, databases and other
resources when their sources, packaging or recipes change. See
[the artifact guide](../../release/artifacts/README.md) for that process.

## Prepare the version

Change `[workspace.package].version` in the root `Cargo.toml`. All 11 app
workspace packages inherit it. Update their lockfile entries with one precise
operation, without refreshing third-party dependencies:

```sh
cargo update -p pv --precise <new-version>
```

Review `Cargo.lock`: only the app workspace package versions should change.
Commit the version preparation through the normal PR/CI process and choose that
tested commit for release. The app version is embedded in the binary, so changing
an RC into another version requires a new build.

The privileged helper has its own version in
`crates/privileged-helper/Cargo.toml` and `PRIVILEGED_HELPER_VERSION` in
`crates/platform/src/helper.rs`. Only change it when releasing a changed helper;
keep those two identities equal. A helper protocol change requires a matching
app release. Ordinary app releases reuse and verify the published helper bytes.

## Build and publish

Run **PV App Release** once, with publication explicitly enabled:

```sh
gh workflow run app-release.yml --ref <tested-ref> \
  -f minimum_pv_version=0.2.6 \
  -f app_platforms=darwin-arm64 \
  -f helper_only=false \
  -f publish=true
```

The workflow builds the selected native binaries, generates the manifest and
installer, then publishes that run's exact handoff artifact. It verifies records,
checksums, sizes, current stable version and immutable objects before updating
the app manifest and installer. Concurrent publications are serialized. Finally,
it downloads both public entrypoints and requires an exact match to the staged
files. The run summary links to the verified endpoints; the publication plan and
build handoff remain downloadable as evidence.

Set `publish=false` for a build-only run. This is the default; a workflow dispatch
does not publish unless explicitly requested. Build-only runs cannot be promoted
later through a second workflow. Do not rebuild an already published version to
change its bytes; release a new version instead.

The current preview target is `darwin-arm64`. Explicitly select
`darwin-arm64,darwin-amd64` for the intended public native matrix after its
resource/RC gates are cleared. `minimum_pv_version=0.2.6` preserves the required
runtime-monitor migration; raise it only for a deliberate compatibility change.
For a helper-only release, keep the app version equal to the current stable app,
increment the helper version, and set `helper_only=true`. The workflow verifies
and reuses the existing app bytes and preserves its minimum-PV requirement.
Select the current stable app's complete platform set: an equal app version
cannot add or remove platforms, even for a helper-only release.

## Observe and recover

Watch the run in Actions, or use:

```sh
gh run list --workflow app-release.yml --event workflow_dispatch \
  --commit <tested-commit-sha> --limit 5 \
  --json databaseId,headSha,createdAt,status,url
gh run watch <run-id> --exit-status --interval 30
```

Select the run from this dispatch by its commit and creation time, rather than
assuming the newest repository run is yours.

If publication fails, inspect the failed step and the publication plan. Rerun
only failed jobs on the same run to keep the original binary bytes, source
commit and provenance. Matching immutable objects are reused; different bytes
at an existing immutable key are rejected. A readback failure can happen after
stable entrypoints were written, so inspect the endpoints before retrying.
Manifest and installer replacements are individually atomic, but the pair is
not a storage transaction; retrying publication repairs a partial pair.

The workflow's endpoint readback proves metadata availability. It does not prove
that the app installs or that macOS integrations work. Use
[the RC checklist](rc-checklist.md) for fresh-install, setup, update and uninstall
acceptance. The existing **Real Artifact E2E** and **Privileged macOS RC**
workflows remain the native runtime/system validation tools.

## Staging and public release

`R2_BUCKET`, `R2_PUBLIC_BASE_URL` and the existing R2 credentials select the
publication destination. The base URL is compiled into the app's update/resource
defaults; it also supplies the generated installer URLs. The manifest's
`stable` channel means the single update feed within that destination. A staging
deployment is not a second runtime-selectable channel.

This workflow publishes to the configured destination. It does not promote
between staging and production, run native RC acceptance automatically, or add
Developer ID signing and notarization. Those public-launch requirements remain
separate decisions. Do not point public users at staging or treat a successful
metadata readback as public-launch sign-off.
