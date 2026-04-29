# PostgreSQL artifacts pipeline (M-series, PG 17/18)

## Goal

Mirror **theseus-rs's** compiled-from-source PostgreSQL bundles into the
existing `artifacts` release on `prvious/pv`, with a CI patching pass that
rewrites Homebrew-pinned openssl `install_name`s to relative
`@executable_path` references so the bundles run on any macOS Apple Silicon
machine without external dependencies.

This is the foundational artifacts step. It does not include any pv-side
consumer code (downloader, postgresenv package, shims, daemon integration);
those follow in separate plans.

## Scope (v1)

- Platform: macOS arm64 only (Apple Silicon, "M-series").
- Versions: PostgreSQL major **17** and **18**, latest patch of each.
- Source: `theseus-rs/postgresql-binaries` GitHub releases
  (asset: `postgresql-<ver>-aarch64-apple-darwin.tar.gz`).
- Cadence: weekly cron (Monday 00:00 UTC), plus manual `workflow_dispatch`.
- Output: two assets in the existing rolling `artifacts` release on
  `prvious/pv`:
  - `postgres-mac-arm64-17.tar.gz`
  - `postgres-mac-arm64-18.tar.gz`

## Non-goals (v1)

- Other architectures (mac x86_64, linux amd64/arm64). Add later.
- Other major versions (16, 15, etc.). Add later.
- Building PostgreSQL from source ourselves. Defer to v2 if/when theseus
  becomes inadequate.
- Notarization with an Apple Developer ID. Ad-hoc signing is enough for
  curl-tarball distribution today.
- pv-side consumer code (downloader, `internal/postgresenv/`, shims, daemon
  integration). Separate plans.

## Why theseus-rs over zonkyio

We initially planned to mirror zonkyio's bundles via Maven Central. Local
inspection revealed that zonkyio's `darwin-arm64v8` archive ships only 3
binaries (`postgres`, `initdb`, `pg_ctl`) and no `include/` — sufficient
for Java-test-style server lifecycle but missing `psql`, `pg_dump`,
`pg_config`, etc. that pv users will want.

theseus-rs ships the full PostgreSQL binary suite (37 tools including
`psql`, `pg_dump`, `pg_restore`, `pg_isready`, `pg_config`, `createdb`,
`pgbench`, etc.) plus `include/` for compiling third-party extensions like
pgvector. The trade-off is that theseus's macOS builds use hardcoded
Homebrew paths (`/opt/homebrew/opt/openssl@3/lib/...`), which we patch out
in CI before publishing.

## Decisions

| Topic | Decision | Rationale |
|---|---|---|
| Source | `theseus-rs/postgresql-binaries` GitHub releases | Full binary suite + `include/` |
| Runner | `macos-15` | `install_name_tool` and `codesign` are darwin-only |
| openssl source | Copy from runner's `/opt/homebrew/opt/openssl@3/lib/` into bundle's `lib/` | Pre-installed on `macos-15` runner |
| Patching | `install_name_tool -change` every Mach-O reference of `/opt/homebrew/opt/openssl@3/lib/lib{ssl,crypto}.3.dylib` → `@executable_path/../lib/lib{ssl,crypto}.3.dylib`; rewrite `LC_ID_DYLIB` on bundled openssl; rewrite libssl's internal libcrypto reference (Homebrew Cellar path) | Standard relocatable-bundle pattern |
| Codesigning | Ad-hoc `codesign --force --sign -` over every Mach-O after patching | `install_name_tool` invalidates the signature; ad-hoc satisfies macOS Gatekeeper for non-Apple-Developer distribution |
| Smoke test | In CI: `initdb` + start `postgres` + `psql SELECT version()` + clean teardown | Catches runtime breakage before publishing |
| Strip | Drop only `share/postgresql/doc/`. Keep `bin/`, `lib/`, `share/`, `include/` in full | Enable third-party extension compilation |
| Asset naming | Major-only: `postgres-mac-arm64-17.tar.gz` | Mirrors PHP (`php-mac-arm64-php8.4.tar.gz`) |
| Release tag | Same `artifacts` release that already hosts FrankenPHP + PHP CLI | Single rolling release |
| Trigger | Always rebundle on every cron/dispatch run; `gh release upload --clobber` | Mirrors FrankenPHP workflow |
| Workflow file | Extend existing `.github/workflows/build-artifacts.yml` | Single workflow, single failure mode acceptable |
| Job parallelism | `postgres` job has no `needs:` — runs in parallel with `build` and `resolve-version` | Postgres has its own version source (theseus releases) |
| Failure coupling | If `build` fails, `release` is skipped, so postgres assets aren't uploaded that week either | Matches existing all-or-nothing behavior; weekly cron retries |

## Architecture

Three changes to `.github/workflows/build-artifacts.yml`:

1. **New `postgres` job** running in parallel with `build`, on `macos-15`,
   matrix over `pg: ["17", "18"]`. Resolves latest patch from theseus-rs
   GitHub releases, downloads + extracts the tarball, bundles Homebrew
   openssl into the staging tree, patches `install_name`s, codesigns,
   verifies cleanliness, runs an end-to-end smoke test, strips docs,
   repacks, and uploads as a workflow artifact.
2. **Extended `release` job** that also waits on `postgres` and downloads
   `postgres-mac-arm64-*` workflow artifacts into the staging dir
   alongside the existing FrankenPHP + PHP CLI assets.
3. **Updated release notes/title** on first-time release creation to
   mention PostgreSQL alongside PHP. (No-op for the existing live release;
   only fires if the release is recreated from scratch.)

## Components

### `postgres` job

```yaml
postgres:
  strategy:
    fail-fast: false
    matrix:
      pg: ["17", "18"]
  name: PostgreSQL ${{ matrix.pg }}
  runs-on: macos-15
  permissions:
    contents: read
  steps:
    - name: Resolve latest patch version from theseus-rs
      id: resolve
      env:
        GH_TOKEN: ${{ github.token }}
      run: |
        set -euo pipefail
        ALL=$(gh api 'repos/theseus-rs/postgresql-binaries/releases?per_page=100' --paginate --jq '.[].tag_name')
        VER=$(echo "$ALL" | grep "^${{ matrix.pg }}\." | sort -V | tail -1)
        if [ -z "$VER" ]; then
          echo "::error::No ${{ matrix.pg }}.x release on theseus-rs"
          exit 1
        fi
        echo "Resolved PG ${{ matrix.pg }} → $VER"
        echo "version=$VER" >> "$GITHUB_OUTPUT"

    - name: Download and extract
      run: |
        set -euo pipefail
        VER="${{ steps.resolve.outputs.version }}"
        URL="https://github.com/theseus-rs/postgresql-binaries/releases/download/${VER}/postgresql-${VER}-aarch64-apple-darwin.tar.gz"
        curl -fsSL -o pg.tar.gz "$URL"
        mkdir extracted
        tar -xzf pg.tar.gz -C extracted
        STAGING=$(ls -d extracted/postgresql-*-aarch64-apple-darwin)
        echo "STAGING=$STAGING" >> "$GITHUB_ENV"

    - name: Bundle Homebrew openssl into staging
      run: |
        set -euo pipefail
        cp /opt/homebrew/opt/openssl@3/lib/libssl.3.dylib   "$STAGING/lib/"
        cp /opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib "$STAGING/lib/"
        chmod 755 "$STAGING/lib/libssl.3.dylib" "$STAGING/lib/libcrypto.3.dylib"

    - name: Patch install_names to relative paths
      run: |
        set -euo pipefail
        # Rewrite LC_ID_DYLIB on bundled openssl
        install_name_tool -id "@executable_path/../lib/libssl.3.dylib"   "$STAGING/lib/libssl.3.dylib"
        install_name_tool -id "@executable_path/../lib/libcrypto.3.dylib" "$STAGING/lib/libcrypto.3.dylib"
        # libssl carries a hardcoded reference to libcrypto via Homebrew Cellar version path.
        # Discover it from otool -L rather than reconstructing the version string.
        CRYPTO_REF=$(otool -L "$STAGING/lib/libssl.3.dylib" | awk '/libcrypto\.3\.dylib/ && !/@executable_path/ {print $1; exit}')
        if [ -n "$CRYPTO_REF" ]; then
          install_name_tool -change "$CRYPTO_REF" "@executable_path/../lib/libcrypto.3.dylib" "$STAGING/lib/libssl.3.dylib"
        fi
        # Walk every Mach-O in bin/ and lib/, rewrite the two openssl paths.
        for f in "$STAGING/bin/"* "$STAGING/lib/"*.dylib; do
          [ -L "$f" ] && continue
          [ -f "$f" ] || continue
          if file -b "$f" | grep -q '^Mach-O'; then
            install_name_tool -change \
              "/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib" \
              "@executable_path/../lib/libssl.3.dylib" \
              "$f" 2>/dev/null || true
            install_name_tool -change \
              "/opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib" \
              "@executable_path/../lib/libcrypto.3.dylib" \
              "$f" 2>/dev/null || true
          fi
        done

    - name: Ad-hoc codesign all Mach-O files
      run: |
        set -euo pipefail
        for f in "$STAGING/bin/"* "$STAGING/lib/"*.dylib; do
          [ -L "$f" ] && continue
          [ -f "$f" ] || continue
          if file -b "$f" | grep -q '^Mach-O'; then
            codesign --force --sign - "$f"
          fi
        done

    - name: Verify deps reference no Homebrew paths
      run: |
        set -euo pipefail
        echo "postgres dependencies:"
        otool -L "$STAGING/bin/postgres"
        if otool -L "$STAGING/bin/postgres" | grep -E '/opt/homebrew' ; then
          echo "::error::postgres still references Homebrew paths after patching"
          exit 1
        fi

    - name: Smoke test (initdb + start + psql + stop)
      run: |
        set -euo pipefail
        DATA_DIR="$RUNNER_TEMP/pgdata"
        rm -rf "$DATA_DIR"
        "$STAGING/bin/initdb" -D "$DATA_DIR" -U postgres --auth=trust >/dev/null
        PORT=54199
        "$STAGING/bin/postgres" -D "$DATA_DIR" -p "$PORT" -k "$DATA_DIR" >"$RUNNER_TEMP/pg.log" 2>&1 &
        PG_PID=$!
        for i in 1 2 3 4 5 6 7 8 9 10; do
          if "$STAGING/bin/pg_isready" -h 127.0.0.1 -p "$PORT" >/dev/null 2>&1; then break; fi
          sleep 1
        done
        if ! "$STAGING/bin/pg_isready" -h 127.0.0.1 -p "$PORT"; then
          echo "::error::pg_isready failed; server log:"
          cat "$RUNNER_TEMP/pg.log"
          kill $PG_PID 2>/dev/null || true
          exit 1
        fi
        VER_OUT=$("$STAGING/bin/psql" -h 127.0.0.1 -p "$PORT" -U postgres -tAc "SELECT version();")
        echo "Server reported: $VER_OUT"
        kill $PG_PID
        wait $PG_PID 2>/dev/null || true

    - name: Strip docs
      run: rm -rf "$STAGING/share/postgresql/doc"

    - name: Structural sanity checks
      run: |
        set -euo pipefail
        test -f "$STAGING/bin/postgres"
        test -f "$STAGING/bin/initdb"
        test -f "$STAGING/bin/pg_ctl"
        test -f "$STAGING/bin/psql"
        test -f "$STAGING/bin/pg_dump"
        test -f "$STAGING/bin/pg_config"
        test -f "$STAGING/lib/libssl.3.dylib"
        test -f "$STAGING/lib/libcrypto.3.dylib"
        test -d "$STAGING/share/postgresql/extension"
        test -d "$STAGING/include"
        echo "Structural sanity checks passed."

    - name: Repack
      run: tar -czf "postgres-mac-arm64-${{ matrix.pg }}.tar.gz" -C "$STAGING" bin lib share include

    - name: Upload binary
      uses: actions/upload-artifact@v4
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
postgres-mac-arm64-17.tar.gz   (~10 MB compressed)
├── bin/                       # 37 binaries
│   ├── postgres
│   ├── initdb
│   ├── pg_ctl
│   ├── psql
│   ├── pg_dump
│   ├── pg_restore
│   ├── pg_isready
│   ├── pg_config
│   ├── createdb / dropdb
│   ├── createuser / dropuser
│   ├── vacuumdb / reindexdb
│   ├── pg_basebackup
│   ├── pg_upgrade
│   ├── pgbench
│   └── … (37 total)
├── lib/
│   ├── libssl.3.dylib            # bundled from runner's Homebrew openssl@3
│   ├── libcrypto.3.dylib         # bundled from runner's Homebrew openssl@3
│   ├── libpq.5.dylib             # postgres own
│   ├── libecpg.6.dylib           # postgres own
│   ├── libpgtypes.3.dylib        # postgres own
│   ├── pgcrypto.dylib            # contrib extension
│   ├── citext.dylib              # contrib extension
│   ├── pg_trgm.dylib             # contrib extension
│   ├── pg_stat_statements.dylib  # contrib extension
│   └── … (~87 dylibs in v18)
├── share/
│   └── postgresql/
│       ├── extension/            # contrib SQL + control files
│       ├── tsearch_data/
│       ├── timezones/
│       ├── *.sample              # postgresql.conf.sample, pg_hba.conf.sample
│       └── *.sql                 # system_views.sql, etc.
└── include/                      # C headers for compiling pgvector etc.
```

Consumer expectation (future pv code): `tar -xzf … -C ~/.pv/postgres/<major>/`
lays this tree down directly under e.g. `~/.pv/postgres/17/`.

After patching, every Mach-O references either:
- `@executable_path/../lib/...` (our bundled libs and postgres' own modules), or
- `/usr/lib/...` (system: libxml2, libz, libSystem, libpam, LDAP framework — present on every macOS).

## Verification

**On the macos-15 runner (workflow):**
- HTTP errors from theseus-rs / GitHub API → curl/`gh api` exits non-zero
  → step fails.
- Tarball extraction errors → tar exits non-zero → step fails.
- Patching errors → `install_name_tool` exits non-zero → step fails.
- Codesigning errors → `codesign` exits non-zero → step fails.
- Verify-clean step explicitly greps `otool -L` output for `/opt/homebrew`
  and fails if anything still references it.
- **Smoke test** runs the actual server: initdb → postgres → pg_isready →
  psql `SELECT version()` → teardown. Catches runtime breakage that static
  checks miss (e.g. ABI mismatch between Homebrew openssl on the runner
  and theseus's build environment).
- Per-version isolation via `fail-fast: false` — PG17 and PG18 run
  independently within the matrix.

**Manual pre-merge verification:**
Dispatch the workflow from the feature branch. The `build` and `postgres`
jobs run; the `release` job is gated on `github.ref == 'refs/heads/main'`
and skipped, so the live release isn't touched. Inspect the workflow
artifact tarballs as a sanity check.

**Out of scope:** end-to-end pv consumer testing (download from artifacts
release, install into `~/.pv/postgres/<ver>/`, run via daemon). That
belongs to pv-side e2e tests in `scripts/e2e/` once consumer code lands.

## Failure modes accepted

- theseus-rs / GitHub down for the entire weekly window → no release that
  week. Manual `workflow_dispatch` recovers it.
- theseus-rs publishes a bundle with a structurally-different lib set
  (e.g. introduces a new Homebrew dep we haven't accounted for) → patcher
  doesn't touch it, smoke test fails (or worse, runtime fails for end
  users) → patch the workflow.
- Homebrew openssl ABI version drift between runner image releases → smoke
  test catches it before publishing. We pin to whatever's on `macos-15`
  at build time.
- PHP `build` job fails → no release for either PHP or postgres that
  week. Matches existing all-or-nothing behavior.

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
