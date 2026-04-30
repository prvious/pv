# MySQL artifacts pipeline (M-series, MySQL 8.0/8.4/9.7)

## Goal

Mirror Oracle's official MySQL Community Server macOS arm64 tarballs into the
existing `artifacts` release on `prvious/pv`, slimmed down by stripping
debug builds, mecab dictionaries, non-English locales, docs, and headers.

This is the foundational artifacts step. It does **not** replace the existing
MySQL Docker container service — pv users keep getting MySQL through Colima
until consumer code lands later. This PR only lays the foundation: published,
pv-owned tarballs we can later wire up the same way RustFS / Postgres are.

## Scope (v1)

- Platform: macOS arm64 only (Apple Silicon, "M-series").
- Versions: MySQL **8.0** (legacy, EOL April 2026 but very common in PHP
  apps), **8.4** (current LTS), **9.7** (Innovation track).
- Source: Oracle CDN, `cdn.mysql.com`.
- Cadence: weekly cron (Monday 00:00 UTC), plus manual `workflow_dispatch`.
- Output: three assets in the existing rolling `artifacts` release on
  `prvious/pv`:
  - `mysql-mac-arm64-8.0.tar.gz`
  - `mysql-mac-arm64-8.4.tar.gz`
  - `mysql-mac-arm64-9.7.tar.gz`

## Non-goals (v1)

- Other architectures (mac x86_64, linux amd64/arm64). Add later.
- Other major.minor versions (8.3, 9.6, 9.8 etc). Bump the matrix when
  needed.
- Replacing the Docker MySQL service — that lives in `internal/services/` and
  is unaffected by this PR.
- pv-side consumer code (downloader, `internal/mysqlenv/`, shims, daemon
  integration, `mysql:add` / `mysql:cli` commands). Separate plans.
- Notarization with an Apple Developer ID. Tarballs are unsigned beyond
  Oracle's release engineering — same posture as the postgres bundle.
- Auto-resolving the latest patch from `dev.mysql.com`. Static patch table
  in the workflow; bump manually with each upstream release.

## Why Oracle official over alternatives

Per the research in `docs/mysql.md`:

| Source | Verdict |
|---|---|
| Oracle MySQL Community (`cdn.mysql.com`) | **Recommended.** Official, all 3 majors covered, `@loader_path` everywhere, ships own openssl/Kerberos/FIDO2/protobuf/ICU. |
| Homebrew `mysql` formula | Not portable — links system libs at fixed Homebrew prefix. |
| MariaDB upstream | No first-party portable macOS arm64 tarball. |
| Percona Server | No macOS arm64 builds. |
| `MariaDB4j`, `wix-embedded-mysql`, `zonkyio` | Wrong vendor / deprecated / does not exist. |

Oracle's tarballs are the only practical source, and (unlike theseus's
postgres bundles) they're already essentially relocatable: 0 of 95 Mach-O
files reference Homebrew or build-host paths. **No `install_name_tool`
rewrite walk needed.**

## Decisions

| Topic | Decision | Rationale |
|---|---|---|
| Source | Oracle CDN direct (`cdn.mysql.com/{Downloads,archives}/...`) | Only viable upstream for portable macOS arm64 |
| Patch resolution | Hardcoded version table in the workflow matrix | Quarterly cadence; weekly cron surfaces stale entries; avoids fragile JS-page scrape |
| Version matrix | `8.0.43` (archived), `8.4.9` (LTS), `9.7.0` (Innovation) | Covers Laravel-pinned 8.0, current LTS, recent Innovation |
| Runner | `macos-15` | Match postgres job; Oracle ships `macos15-arm64` builds |
| openssl source | Bundle ships its own `libssl.3` / `libcrypto.3` — no Homebrew copy needed | Self-contained tarball |
| Patching | Single defensive `install_name_tool -id` on `libfido2` so its self-name doesn't read `/usr/local/mysql/lib/...` | Cosmetic only; nothing in the bundle links via that path. Belt-and-braces against future tooling that might inspect LC_ID_DYLIB. |
| Codesigning | None | We don't rewrite Mach-Os; signatures stay valid as Oracle shipped them |
| Verification | `otool -L` walk every Mach-O, fail if any reference `/opt/homebrew` or `/Users/runner` | Mirrors postgres verify; cheap insurance |
| Smoke test | `scripts/test-mysql-bundle.sh` runs: initialize, start, JSON+fulltext+CRUD, dump roundtrip, mysqlslap, shutdown | Catches runtime breakage; exercises ICU + plugin loading |
| Strip | Drop `mysqld-debug`, `lib/mecab/`, `lib/plugin/debug/`, non-English `share/{locale}/` dirs, `man/`, `docs/`, `support-files/`, `include/`, `lib/pkgconfig/`. Keep all auth plugins (LDAP/Kerberos/OCI/WebAuthn). | 637MB → 289MB extracted, ~86MB compressed. Auth plugins are dynamically loaded by name; dropping them is a footgun. |
| Asset naming | Major.minor: `mysql-mac-arm64-{major}.tar.gz` (e.g. `mysql-mac-arm64-8.4.tar.gz`) | Matches postgres `postgres-mac-arm64-17.tar.gz` convention; major.minor is needed because 9.x moves quarterly |
| Release tag | Same `artifacts` release as FrankenPHP + PHP CLI + Postgres | Single rolling release |
| Trigger | Always rebundle; `gh release upload --clobber` | Mirrors existing jobs |
| Workflow file | Extend `.github/workflows/build-artifacts.yml` | Single workflow |
| Job parallelism | `mysql` job has no `needs:` — runs in parallel with `frankenphp` and `postgres` | Independent build pipeline |
| Failure coupling | If any of `frankenphp` / `postgres` / `mysql` fails, `release` is skipped | Matches existing all-or-nothing behavior; weekly cron retries |

## Architecture

Three changes to `.github/workflows/build-artifacts.yml`:

1. **New `mysql` job** running in parallel with `frankenphp` and `postgres`,
   on `macos-15`, matrix over `["8.0", "8.4", "9.7"]`. Resolves URL from a
   `matrix.include` table, downloads and extracts, strips, runs a defensive
   `install_name_tool -id` fixup on libfido2, verifies no host-path leaks,
   smoke-tests, repacks, uploads.
2. **Extended `release` job** that also waits on `mysql`, downloads
   `mysql-mac-arm64-*` workflow artifacts, validates count == 3, copies to
   `release/`.
3. **Updated release notes/title** on first-time release creation to mention
   MySQL alongside PHP and Postgres. (No-op for the existing live release.)

Plus one new file:

4. **`scripts/test-mysql-bundle.sh`** — integration test that boots a real
   mysqld from the staged bundle and exercises CRUD, JSON, fulltext (ICU),
   mysqldump roundtrip, and mysqlslap.

## Components

### `mysql` job

```yaml
mysql:
  strategy:
    fail-fast: false
    matrix:
      include:
        - major: "8.0"
          version: "8.0.43"
          path: "archives/mysql-8.0"
        - major: "8.4"
          version: "8.4.9"
          path: "Downloads/MySQL-8.4"
        - major: "9.7"
          version: "9.7.0"
          path: "Downloads/MySQL-9.7"
  name: MySQL ${{ matrix.major }}
  runs-on: macos-15
  permissions:
    contents: read
  steps:
    - name: Checkout pv (for scripts/test-mysql-bundle.sh)
      uses: actions/checkout@v4

    - name: Download Oracle tarball
      run: |
        set -euo pipefail
        URL="https://cdn.mysql.com/${{ matrix.path }}/mysql-${{ matrix.version }}-macos15-arm64.tar.gz"
        curl -fsSL -o mysql.tar.gz "$URL"

    - name: Extract
      run: |
        set -euo pipefail
        mkdir extracted
        tar -xzf mysql.tar.gz -C extracted
        STAGING=$(ls -d extracted/mysql-*-macos15-arm64)
        echo "STAGING=$STAGING" >> "$GITHUB_ENV"

    - name: Strip
      run: |
        set -euo pipefail
        rm -f  "$STAGING/bin/mysqld-debug"
        rm -rf "$STAGING/lib/mecab"
        rm -rf "$STAGING/lib/plugin/debug"
        rm -rf "$STAGING/lib/pkgconfig"
        rm -rf "$STAGING/docs" "$STAGING/man" "$STAGING/support-files" "$STAGING/include"
        # Keep share/english + share/charsets; drop other locales:
        find "$STAGING/share" -mindepth 1 -maxdepth 1 -type d \
          ! -name english ! -name charsets -exec rm -rf {} +

    - name: Defensive install_name fixup on libfido2
      run: |
        set -euo pipefail
        # libfido2's LC_ID_DYLIB reads /usr/local/mysql/lib/libfido2.1.dylib
        # — cosmetic; nothing in the bundle links via that path. But we
        # rewrite it for cleanliness and so the verify step below can be
        # strict.
        FIDO=$(ls "$STAGING"/lib/libfido2.*.dylib 2>/dev/null | head -1 || true)
        if [ -n "$FIDO" ]; then
          install_name_tool -id "@loader_path/libfido2.1.dylib" "$FIDO"
        fi

    - name: Verify no build-host paths
      run: |
        set -euo pipefail
        shopt -s nullglob
        LEAKS=0
        # Walk every Mach-O under bin/ and lib/ (recursive — plugins live in lib/plugin/).
        while IFS= read -r f; do
          [ -L "$f" ] && continue
          [ -f "$f" ] || continue
          if ! file -b "$f" | grep -q '^Mach-O'; then continue; fi
          bad=$(otool -L "$f" 2>/dev/null | tail -n +2 | awk '{print $1}' \
                | grep -E '^/(opt/homebrew|Users/runner|usr/local/mysql)' || true)
          if [ -n "$bad" ]; then
            echo "::error::Host path leak in $(basename "$f"):"
            echo "$bad"
            LEAKS=$((LEAKS+1))
          fi
        done < <(find "$STAGING/bin" "$STAGING/lib" -type f)
        if [ "$LEAKS" -ne 0 ]; then
          echo "::error::$LEAKS file(s) reference build-host paths"
          exit 1
        fi
        echo "All Mach-O files reference only @loader_path / system paths."

    - name: Smoke test
      run: bash scripts/test-mysql-bundle.sh "$STAGING"

    - name: Structural sanity checks
      run: |
        set -euo pipefail
        test -x "$STAGING/bin/mysqld"
        test -x "$STAGING/bin/mysql"
        test -x "$STAGING/bin/mysqladmin"
        test -x "$STAGING/bin/mysqldump"
        test -f "$STAGING/lib/libssl.3.dylib"
        test -f "$STAGING/lib/libcrypto.3.dylib"
        test -d "$STAGING/share/english"
        test -d "$STAGING/share/charsets"
        echo "Structural sanity checks passed."

    - name: Repack
      run: tar -czf "mysql-mac-arm64-${{ matrix.major }}.tar.gz" -C "$STAGING" .

    - name: Upload binary
      uses: actions/upload-artifact@v4
      with:
        name: mysql-mac-arm64-${{ matrix.major }}
        path: mysql-mac-arm64-${{ matrix.major }}.tar.gz
        compression-level: 0
```

### `release` job extension

Three changes to the existing `release` job:

1. `needs: [frankenphp, postgres]` → `needs: [frankenphp, postgres, mysql]`.
2. Add a download step:
   ```yaml
   - uses: actions/download-artifact@v4
     with:
       pattern: mysql-mac-arm64-*
       path: artifacts
   ```
3. In "Prepare release assets", after the existing postgres validation,
   add:
   ```bash
   mysql_dirs=(artifacts/mysql-mac-arm64-*)
   if [ ${#mysql_dirs[@]} -ne 3 ]; then
     echo "::error::Expected 3 mysql bundles (8.0 + 8.4 + 9.7), found ${#mysql_dirs[@]}"
     exit 1
   fi
   for dir in "${mysql_dirs[@]}"; do
     cp "$dir"/* "release/"
   done
   ```

The first-time release-create `gh release create` block updates its
`--title` and `--notes` to mention MySQL too. This is a no-op for the
current live release (the `gh release view` guard skips re-creation), but
keeps the workflow file honest if the release is ever re-created.

The existing `gh release upload "$TAG" release/* --clobber` step needs no
changes.

### `scripts/test-mysql-bundle.sh`

Mirrors `test-postgres-bundle.sh`. Bash, `set -euo pipefail`, exit code
0 = pass.

```
1. Argument check; verify bin/mysqld is executable
2. mkdtemp DATA_DIR and RUN_DIR; trap cleanup (mysqladmin shutdown, kill, rm -rf)
3. mysqld --initialize-insecure --datadir=$DATA --basedir=$STAGING --user=$USER
4. Start mysqld in background:
     --port=33099 --socket=$RUN/m.sock --pid-file=$RUN/m.pid
     --mysqlx=OFF --innodb-buffer-pool-size=64M --innodb-log-file-size=24M
5. Poll mysqladmin ping (up to 30s — initial InnoDB init takes 5–10s)
6. SELECT VERSION() — print server identity
7. Functional exercise (catches plugin/ICU breakage that static checks miss):
     CREATE DATABASE pv_test;
     CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR(64), payload JSON);
     INSERT 2 rows;
     SELECT JSON_EXTRACT(payload,'$.k') WHERE id=1  → assert '1'
     CREATE FULLTEXT INDEX ft ON t(name)            → exercises ICU
     SELECT count(*) FROM t WHERE MATCH(name) AGAINST('one') → assert > 0
8. mysqldump pv_test → mysql restore into pv_test_restored;
   assert row count matches
9. mysqlslap --concurrency=4 --iterations=20 (connection handling)
10. mysqladmin shutdown; wait $PID
```

Skipped on purpose:
- WebAuthn / Kerberos / LDAP / OCI auth plugins — need external fixtures.
- `mysql_secure_installation` — interactive.
- `mysqld_safe` / `mysqld_multi` — pv won't use them either.

## Tarball layout

Flat root, all paths relative — same shape Oracle ships, minus the trim:

```
mysql-mac-arm64-8.4.tar.gz   (~86 MB compressed)
├── bin/
│   ├── mysqld
│   ├── mysql
│   ├── mysqladmin
│   ├── mysqldump
│   ├── mysqlbinlog
│   ├── mysqlcheck / mysqlimport / mysqlshow / mysqlslap
│   ├── mysql_secure_installation
│   ├── mysql_tzinfo_to_sql
│   ├── my_print_defaults / perror
│   ├── mysqld_safe (sh) / mysqld_multi (perl)   # kept; unused by pv
│   ├── myisam* / ibd2sdi / innochecksum
│   ├── mysql_config / mysql_config_editor
│   └── mysql_migrate_keyring
├── lib/
│   ├── libssl.3.dylib / libcrypto.3.dylib       # bundled by Oracle
│   ├── libfido2.*.dylib                         # WebAuthn auth plugin
│   ├── libgssapi_krb5.* / libkrb5* / libk5crypto* / libkrb5support* / libcom_err*
│   ├── libprotobuf.*.dylib / libprotobuf-lite.*.dylib  # group replication
│   ├── libmysqlclient.24.dylib / libmysqlclient.a / libmysqlservices.a
│   ├── private/icudt77l/                        # ICU data for fulltext regex
│   └── plugin/                                  # auth + component plugins
└── share/
    ├── english/errmsg.sys                       # required at runtime
    ├── charsets/                                # required at runtime
    ├── dictionary.txt                           # FTS stopwords
    ├── install_rewriter.sql / uninstall_rewriter.sql
    └── (other component setup .sql files)
```

After the trim, every Mach-O references either:
- `@loader_path/...` (bundled dylibs), or
- `/usr/lib/...` / `/System/Library/Frameworks/...` (macOS-stable system libs).

Consumer expectation (future pv code): `tar -xzf … -C ~/.pv/mysql/<major>/`
lays this tree down directly under e.g. `~/.pv/mysql/8.4/`.

## Verification

**On the macos-15 runner (workflow):**

- HTTP errors from cdn.mysql.com → `curl -fsSL` exits non-zero → step fails.
- Tarball extraction errors → tar exits non-zero → step fails.
- `install_name_tool` failure on libfido2 → step fails.
- Verify-no-host-paths step explicitly greps `otool -L` output for
  `/opt/homebrew`, `/Users/runner`, and `/usr/local/mysql` and fails if
  anything still references them.
- **Smoke test** (`scripts/test-mysql-bundle.sh`) runs the actual server
  — initialize, start, JSON+fulltext+CRUD, dump roundtrip, mysqlslap,
  clean shutdown. Catches runtime breakage that static checks miss
  (e.g. ICU data missing after over-aggressive locale strip).
- Per-version isolation via `fail-fast: false` — each major runs
  independently within the matrix.
- Structural sanity check confirms every binary the consumer side will
  depend on is present.

**Manual pre-merge verification:**
Dispatch the workflow from the feature branch. The `frankenphp`,
`postgres`, and `mysql` jobs run; the `release` job is gated on
`github.ref == 'refs/heads/main'` and skipped, so the live release isn't
touched. Inspect the workflow artifact tarballs as a sanity check.

**Out of scope:** end-to-end pv consumer testing (download from artifacts
release, install into `~/.pv/mysql/<major>/`, run via daemon, migration
from the existing Docker MySQL service). Those belong to follow-up specs.

## Failure modes accepted

- Oracle CDN down for the entire weekly window → no MySQL release that
  week. Manual `workflow_dispatch` recovers it.
- Oracle bumps a patch version and the matrix entry goes stale → still
  builds the older patch successfully; weekly job keeps running. Bump in
  a follow-up PR. Acceptable: MySQL releases quarterly.
- Oracle promotes 9.8 to LTS and we want to track it → bump matrix entry
  in the same PR. No tooling change.
- Oracle changes archive layout (e.g. moves 8.4 from `Downloads/` to
  `archives/` after EOL) → 404, step fails, fix the matrix `path`. Weekly
  cron surfaces this within a week.
- Oracle introduces a new bundled dependency that links to a
  `/usr/local/...` self-name → verify-no-host-paths step catches it,
  patch the workflow.
- One of `frankenphp` / `postgres` / `mysql` fails → no release for any
  of them that week. Matches existing all-or-nothing behavior.

## Migration / rollout

1. Open PR with workflow + script changes.
2. Dispatch from the feature branch to verify all three majors build and
   smoke-test cleanly. The `release` job is skipped (gated on `main`).
3. Inspect workflow artifacts; spot-check `tar -tzf` and a manual run of
   `mysqld --initialize` from the unpacked bundle on a clean local user.
4. Merge to main.
5. First scheduled (or dispatched) run from main publishes the three
   `mysql-mac-arm64-{8.0,8.4,9.7}.tar.gz` files to the `artifacts`
   release alongside the existing PHP and Postgres assets.
6. Verify via `gh release view artifacts` that all three new assets are
   listed.

No coordination needed with pv consumer code — the existing Docker MySQL
service path is unchanged. Future `internal/mysqlenv/` work consumes
these artifacts in a separate spec.
