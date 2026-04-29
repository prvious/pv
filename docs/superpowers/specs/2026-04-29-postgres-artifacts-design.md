# PostgreSQL artifacts pipeline (M-series, PG 17/18)

## Goal

Mirror zonkyio's PostgreSQL bundles from Maven Central into the existing
`artifacts` release on `prvious/pv`, so pv can consume them via the same
download pattern it already uses for FrankenPHP and the static PHP CLI.

This is the foundational artifacts step. It does not include any pv-side
consumer code (downloader, postgresenv package, shims, daemon integration);
those follow in separate plans.

## Scope (v1)

- Platform: macOS arm64 only (Apple Silicon, "M-series").
- Versions: PostgreSQL major **17** and **18**, latest patch of each.
- Source: `io.zonky.test.postgres:embedded-postgres-binaries-darwin-arm64v8`
  on Maven Central.
- Cadence: weekly cron (Monday 00:00 UTC), plus manual `workflow_dispatch`.
- Output: two assets in the existing rolling `artifacts` release on
  `prvious/pv`:
  - `postgres-mac-arm64-17.tar.gz`
  - `postgres-mac-arm64-18.tar.gz`

## Non-goals (v1)

- Other architectures (mac x86_64, linux amd64/arm64). Add later.
- Other major versions (16, 15, 14, etc.). Add later.
- Building PostgreSQL from source. We're mirroring zonkyio.
- Codesigning. zonkyio's binaries ship as-is; if Gatekeeper bites, address
  in v2.
- `lipo`-thinning the universal binary to drop the x86_64 slice. Add later
  if archive size matters.
- Version metadata sidecar (e.g. `postgres-mac-arm64-17.version.txt`
  containing `17.6.0`). Defer; the bundled README inside the archive carries
  the version if needed.
- pv-side consumer code (downloader, `internal/postgresenv/`, shims, daemon
  integration). Separate plans.

## Decisions

| Topic | Decision | Rationale |
|---|---|---|
| Release tag | Same `artifacts` release that already hosts FrankenPHP + PHP CLI | Single rolling release for all pv-managed binaries; matches existing pattern |
| Asset naming | Major-only: `postgres-mac-arm64-17.tar.gz` | Mirrors PHP (`php-mac-arm64-php8.4.tar.gz`); pv resolves "17" → fetch directly |
| Strip aggressiveness | Drop only `share/postgresql/doc/`. Keep `bin/`, `lib/`, `share/`, `include/` in full | Allows third-party extension compilation (pgvector, postgis); ~30–35 MB bundle |
| Trigger semantics | Always rebundle on every cron/dispatch run; `gh release upload --clobber` overwrites | Mirrors FrankenPHP workflow; no version-comparison state to drift |
| Workflow file | Extend existing `.github/workflows/build-artifacts.yml` (add a `postgres` job) | Single workflow, reuses release job |
| Job parallelism | `postgres` job has no `needs:` — runs in parallel with `build` and `resolve-version` | Postgres has its own version source (Maven), independent of FrankenPHP |
| Failure coupling | If `build` fails, `release` is skipped, so postgres assets aren't uploaded that week either | Matches existing all-or-nothing behavior; weekly cron retries |

## Architecture

Three changes to `.github/workflows/build-artifacts.yml`:

1. **New `postgres` job** running in parallel with `build`, on `ubuntu-latest`,
   matrix over `pg: ["17", "18"]`. Each matrix entry resolves the latest
   patch from Maven, downloads the JAR, unwraps it, strips `doc/`, runs
   structural sanity checks, and uploads a workflow artifact.
2. **Extended `release` job** that also waits on `postgres` and downloads
   `postgres-mac-arm64-*` workflow artifacts into the staging dir alongside
   the existing FrankenPHP + PHP CLI assets.
3. **Updated release notes/title** on first-time release creation to mention
   PostgreSQL alongside PHP. (No-op for the existing live release; would
   apply only if the release is recreated from scratch. Safe to leave.)

## Components

### `postgres` job

```yaml
postgres:
  strategy:
    fail-fast: false
    matrix:
      pg: ["17", "18"]
  name: PostgreSQL ${{ matrix.pg }}
  runs-on: ubuntu-latest
  permissions:
    contents: read
  steps:
    - name: Resolve latest patch version from Maven
      id: resolve
      run: |
        META_URL="https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-darwin-arm64v8/maven-metadata.xml"
        ALL=$(curl -fsSL "$META_URL" | grep -oE '<version>[^<]+' | sed 's/<version>//')
        VER=$(echo "$ALL" | grep "^${{ matrix.pg }}\." | sort -V | tail -1)
        [ -n "$VER" ] || { echo "::error::no ${{ matrix.pg }}.x version on Maven"; exit 1; }
        echo "version=$VER" >> "$GITHUB_OUTPUT"

    - name: Download and unwrap JAR
      run: |
        VER="${{ steps.resolve.outputs.version }}"
        JAR_URL="https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-darwin-arm64v8/${VER}/embedded-postgres-binaries-darwin-arm64v8-${VER}.jar"
        curl -fsSL -o pg.jar "$JAR_URL"
        mkdir -p unpacked staging
        unzip -q pg.jar -d unpacked
        tar -xJf unpacked/postgres-darwin-arm_64.txz -C staging

    - name: Strip docs
      run: rm -rf staging/share/postgresql/doc

    - name: Structural sanity checks
      run: |
        test -f staging/bin/postgres
        test -f staging/bin/initdb
        test -f staging/bin/psql
        test -f staging/bin/pg_ctl
        test -f staging/bin/pg_dump
        test -f staging/bin/pg_config
        test -f staging/lib/libssl.3.dylib
        test -f staging/lib/libcrypto.3.dylib
        test -f staging/lib/libicuuc.77.1.dylib
        test -d staging/share/postgresql/extension
        test -d staging/include

    - name: Repack
      run: tar -czf "postgres-mac-arm64-${{ matrix.pg }}.tar.gz" -C staging bin lib share include

    - uses: actions/upload-artifact@v4
      with:
        name: postgres-mac-arm64-${{ matrix.pg }}
        path: postgres-mac-arm64-${{ matrix.pg }}.tar.gz
        compression-level: 0
```

### `release` job extension

Additions to the existing job (existing steps unchanged):

```yaml
- uses: actions/download-artifact@v4
  with:
    pattern: postgres-mac-arm64-*
    path: artifacts
```

In the existing "Prepare release assets" step, after the FrankenPHP/PHP
validation block, add:

```bash
pg_dirs=(artifacts/postgres-mac-arm64-*)
if [ ${#pg_dirs[@]} -lt 2 ]; then
  echo "::error::Expected 2 postgres bundles (17 + 18), found ${#pg_dirs[@]}"
  exit 1
fi
for dir in "${pg_dirs[@]}"; do
  cp "$dir"/* "release/"
done
```

The existing `gh release upload "$TAG" release/* --clobber` step requires
no changes — it already uploads everything in `release/`.

## Tarball layout

Flat root, matches pv's existing extraction expectation:

```
postgres-mac-arm64-17.tar.gz
├── bin/
│   ├── postgres
│   ├── initdb
│   ├── pg_ctl
│   ├── psql
│   ├── pg_dump
│   ├── pg_config
│   └── … (37 binaries total)
├── lib/
│   ├── libssl.3.dylib            # OpenSSL 3 (bundled)
│   ├── libcrypto.3.dylib         # OpenSSL 3 (bundled)
│   ├── libicu*.dylib             # ICU 77 (bundled)
│   ├── libxml2.16.dylib          # libxml2 (bundled)
│   ├── liblz4, libzstd, libz, libiconv, libgssapi_krb5, libk5crypto
│   ├── pgcrypto.dylib            # contrib extensions (built-in)
│   ├── citext.dylib
│   ├── pg_trgm.dylib
│   └── … (87 dylibs total)
├── share/
│   └── postgresql/
│       ├── extension/            # contrib extension SQL + control files
│       ├── tsearch_data/
│       ├── timezones/
│       ├── *.sample              # postgresql.conf.sample, pg_hba.conf.sample, etc.
│       └── *.sql                 # system_views.sql, etc.
└── include/                      # C headers for third-party extension builds
```

Consumer expectation (future pv code): `tar -xzf … -C ~/.pv/postgres/<major>/`
lays this tree down directly under e.g. `~/.pv/postgres/17/`.

## Verification

**On the Linux runner (workflow):**
- Maven HTTP errors → curl exits non-zero → step fails.
- JAR/txz extraction errors → unzip/tar exit non-zero → step fails.
- Missing critical files → explicit `test -f` checks fail the step.
- Per-version isolation: `fail-fast: false` lets PG17 and PG18 succeed/fail
  independently within the matrix; the job's overall result still fails if
  either matrix entry fails, gating release.

**Manual pre-merge verification:**
Dispatch the workflow from the feature branch. The `build` and `postgres`
jobs run; the `release` job is gated on `github.ref == 'refs/heads/main'`
and skipped, so the live release isn't touched. Inspect the workflow
artifact tarballs as a sanity check.

**Out of scope:** runtime verification on macOS (binary actually executes,
universal slices intact, etc.). That belongs to pv-side e2e tests in
`scripts/e2e/` once consumer code lands.

## Failure modes accepted

- Maven Central down for the entire weekly window → no release that week.
  Manual `workflow_dispatch` once Maven is back recovers it.
- zonkyio publishes a structurally-valid but runtime-broken bundle → user
  reports the issue, we patch (skip that version manually, override pin,
  etc.). Acceptable for v1.
- PHP `build` job fails → no release for either PHP or postgres that week.
  Matches existing all-or-nothing behavior.

## Migration / rollout

1. Open PR with workflow changes.
2. Dispatch from the feature branch to verify builds succeed; inspect
   workflow artifacts.
3. Merge to main.
4. First scheduled (or dispatched) run from main publishes
   `postgres-mac-arm64-17.tar.gz` and `postgres-mac-arm64-18.tar.gz` to the
   `artifacts` release alongside the existing PHP assets.
5. Verify via `gh release view artifacts` that the assets are present.

No coordination needed with pv consumer code — that's a separate plan, and
this artifacts pipeline can ship and live on its own.
