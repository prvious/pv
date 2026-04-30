# MySQL artifacts pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `mysql` job to `.github/workflows/build-artifacts.yml` that downloads Oracle's official MySQL Community Server tarballs (8.0.43 / 8.4.9 / 9.7.0), strips them, smoke-tests them, and publishes three `mysql-mac-arm64-{major}.tar.gz` assets to the existing rolling `artifacts` release on `prvious/pv`.

**Architecture:** Mirror the existing `postgres` job pattern, simplified: Oracle's tarballs are essentially relocatable as-shipped (research found 0 of 95 Mach-O files reference Homebrew/build-host paths), so no `install_name_tool -change` walk and no codesigning. Job runs in parallel with `frankenphp` and `postgres`, on `macos-15`, matrix over the three majors via `matrix.include` (each entry pins exact patch version + CDN path because `8.0` lives at `archives/` and `8.4` / `9.7` live at `Downloads/`). New `scripts/test-mysql-bundle.sh` boots a real mysqld and exercises CRUD + JSON + fulltext (ICU) + dump roundtrip + mysqlslap.

**Tech Stack:** GitHub Actions (`macos-15` runner), bash, `curl`, `tar`, `otool`, `install_name_tool`, Oracle MySQL `cdn.mysql.com`. No new dependencies. Spec lives at `docs/superpowers/specs/2026-04-30-mysql-artifacts-design.md`.

---

## File Structure

**Create:**
- `scripts/test-mysql-bundle.sh` — integration test that boots mysqld from a staged bundle and exercises ICU + plugin loading + dump roundtrip. Exit 0 = pass.

**Modify:**
- `.github/workflows/build-artifacts.yml`:
  - Add `mysql` job (between the existing `postgres` job and `release` job for readability, ordering doesn't matter functionally).
  - Update `release.needs` from `[frankenphp, postgres]` to `[frankenphp, postgres, mysql]`.
  - Add a third `actions/download-artifact@v4` step in `release`.
  - Add validation + copy block for mysql tarballs in "Prepare release assets".
  - Update first-time `gh release create` `--title` and `--notes` to mention MySQL.

No Go code. No pv-side consumer code. The Docker MySQL service in `internal/services/` is unchanged.

---

## Task 1: Local validation of the upstream tarball

Before touching CI, validate the assumption that Oracle's tarball runs cleanly on a fresh local extraction. If this fails locally, the CI job will fail too — this catches it 10× faster.

**Files:**
- No source changes. Working in a scratch dir.

- [ ] **Step 1: Create a scratch dir and download the 8.4.9 tarball**

```bash
mkdir -p /tmp/mysql-validate && cd /tmp/mysql-validate
curl -fsSL -o mysql.tar.gz \
  "https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.9-macos15-arm64.tar.gz"
ls -lh mysql.tar.gz
```

Expected: ~160 MB file downloaded.

- [ ] **Step 2: Extract and check for host-path leaks**

```bash
tar -xzf mysql.tar.gz
cd mysql-8.4.9-macos15-arm64

# Walk every Mach-O and check for /opt/homebrew, /Users/runner, /usr/local/mysql refs.
# Exception: libfido2's own LC_ID_DYLIB is expected to read /usr/local/mysql/...
# Per research, that's cosmetic — we'll fix it defensively in CI.
LEAKS=0
while IFS= read -r f; do
  [ -L "$f" ] && continue
  [ -f "$f" ] || continue
  if ! file -b "$f" | grep -q '^Mach-O'; then continue; fi
  bad=$(otool -L "$f" 2>/dev/null | tail -n +2 | awk '{print $1}' \
        | grep -E '^/(opt/homebrew|Users/runner|usr/local/mysql)' || true)
  if [ -n "$bad" ]; then
    echo "LEAK in $(basename "$f"):"
    echo "$bad"
    LEAKS=$((LEAKS+1))
  fi
done < <(find bin lib -type f)
echo "Total leak files: $LEAKS"
```

Expected output: exactly **one leak** — `libfido2.1.15.0.dylib` referencing `/usr/local/mysql/lib/libfido2.1.dylib` (its own LC_ID_DYLIB).

If the count differs, the strip / fixup steps in the workflow need adjustment before continuing. **Stop and re-read `docs/mysql.md` "Portability analysis" section.** Do not proceed.

- [ ] **Step 3: Apply the libfido2 defensive fixup, re-verify, expect 0 leaks**

```bash
FIDO=$(ls lib/libfido2.*.dylib | head -1)
install_name_tool -id "@loader_path/libfido2.1.dylib" "$FIDO"

# Re-run the same scan
LEAKS=0
while IFS= read -r f; do
  [ -L "$f" ] && continue
  [ -f "$f" ] || continue
  if ! file -b "$f" | grep -q '^Mach-O'; then continue; fi
  bad=$(otool -L "$f" 2>/dev/null | tail -n +2 | awk '{print $1}' \
        | grep -E '^/(opt/homebrew|Users/runner|usr/local/mysql)' || true)
  if [ -n "$bad" ]; then
    LEAKS=$((LEAKS+1))
  fi
done < <(find bin lib -type f)
echo "Total leak files after fixup: $LEAKS"
```

Expected: `Total leak files after fixup: 0`.

- [ ] **Step 4: Manual smoke test — boot mysqld, run a query, shut down**

```bash
DATA=/tmp/mysql-validate/data
RUN=/tmp/mysql-validate/run
rm -rf "$DATA" "$RUN"
mkdir -p "$RUN"

./bin/mysqld --initialize-insecure --datadir="$DATA" --basedir="$PWD" --user="$USER"
./bin/mysqld --datadir="$DATA" --basedir="$PWD" \
  --port=33099 --socket="$RUN/m.sock" --pid-file="$RUN/m.pid" \
  --mysqlx=OFF --innodb-buffer-pool-size=64M &
MYSQL_PID=$!

# Wait for ready (up to 30s)
for i in $(seq 1 30); do
  ./bin/mysqladmin --socket="$RUN/m.sock" -u root ping >/dev/null 2>&1 && break
  sleep 1
done

./bin/mysqladmin --socket="$RUN/m.sock" -u root ping
./bin/mysql --socket="$RUN/m.sock" -u root -e "SELECT VERSION();"
./bin/mysqladmin --socket="$RUN/m.sock" -u root shutdown
wait $MYSQL_PID 2>/dev/null || true
```

Expected: `mysqld is alive`, then `VERSION()` returns `8.4.9`, then a clean shutdown.

If this fails, either the bundle is broken in some new way (regenerate tarball; investigate) or the local environment is broken (port conflict, firewall). Do not proceed until this works locally.

- [ ] **Step 5: Clean up scratch dir**

```bash
rm -rf /tmp/mysql-validate
```

**No commit for Task 1** — this task only validates assumptions before touching code.

---

## Task 2: Write `scripts/test-mysql-bundle.sh`

Mirror of `scripts/test-postgres-bundle.sh`. Bash, `set -euo pipefail`, exits 0 on pass.

**Files:**
- Create: `scripts/test-mysql-bundle.sh`

- [ ] **Step 1: Create the script**

Create `scripts/test-mysql-bundle.sh` with this exact content:

```bash
#!/bin/bash
# test-mysql-bundle.sh — integration test for a built MySQL bundle.
#
# Verifies that an extracted Oracle MySQL Community bundle (after our strip
# pass) actually works end-to-end: initialize, server startup, JSON +
# fulltext (ICU) + CRUD, mysqldump → restore roundtrip, and mysqlslap.
#
# Usage:
#     scripts/test-mysql-bundle.sh <path-to-extracted-bundle>
#
# The path argument should be the directory containing bin/, lib/, share/.
# Used by .github/workflows/build-artifacts.yml against the staged tree, and
# locally by passing an extracted CI artifact.
#
# Exit code: 0 = all checks passed; non-zero = at least one failed.

set -euo pipefail

MYSQL_PREFIX="${1:?usage: $0 <path-to-extracted-bundle>}"
PORT="${MYSQL_TEST_PORT:-33099}"

if [ ! -x "$MYSQL_PREFIX/bin/mysqld" ]; then
    echo "::error::No bin/mysqld at $MYSQL_PREFIX"
    exit 1
fi

WORK_DIR="$(mktemp -d)"
DATA_DIR="$WORK_DIR/data"
RUN_DIR="$WORK_DIR/run"
mkdir -p "$RUN_DIR"
MYSQL_PID=""
SOCK="$RUN_DIR/m.sock"

cleanup() {
    if [ -n "$MYSQL_PID" ]; then
        "$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root shutdown 2>/dev/null || \
            kill "$MYSQL_PID" 2>/dev/null || true
        wait "$MYSQL_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

MYSQL=("$MYSQL_PREFIX/bin/mysql" --socket="$SOCK" -u root -N -B)

# 1. initialize
echo "== 1. mysqld --initialize-insecure =="
"$MYSQL_PREFIX/bin/mysqld" --initialize-insecure \
    --datadir="$DATA_DIR" --basedir="$MYSQL_PREFIX" --user="$USER" 2>&1 | tail -5
echo "  ✓ datadir initialized at $DATA_DIR"

# 2. start mysqld in background
echo "== 2. start mysqld =="
"$MYSQL_PREFIX/bin/mysqld" \
    --datadir="$DATA_DIR" --basedir="$MYSQL_PREFIX" \
    --port="$PORT" --socket="$SOCK" --pid-file="$RUN_DIR/m.pid" \
    --mysqlx=OFF --innodb-buffer-pool-size=64M --innodb-log-file-size=24M \
    >"$WORK_DIR/mysqld.log" 2>&1 &
MYSQL_PID=$!

for i in $(seq 1 30); do
    if "$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root ping >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

if ! "$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root ping >/dev/null 2>&1; then
    echo "::error::mysqld did not become ready within 30s"
    cat "$WORK_DIR/mysqld.log"
    exit 1
fi
echo "  ✓ accepting connections on socket $SOCK (port $PORT)"

# 3. server identity
echo "== 3. server identity =="
SERVER_VER=$("${MYSQL[@]}" -e "SELECT VERSION();")
echo "  $SERVER_VER"

# 4. CRUD + JSON + fulltext (exercises ICU + plugin loading)
echo "== 4. CRUD + JSON + fulltext =="
"${MYSQL[@]}" <<'SQL'
CREATE DATABASE pv_test;
USE pv_test;
CREATE TABLE t (
    id INT PRIMARY KEY,
    name VARCHAR(64),
    payload JSON,
    FULLTEXT KEY ft_name (name)
) ENGINE=InnoDB;
INSERT INTO t VALUES
    (1, 'one apple',   '{"k":1}'),
    (2, 'two bananas', '{"k":2}');
SQL

JSON_RESULT=$("${MYSQL[@]}" -e "SELECT JSON_EXTRACT(payload,'\$.k') FROM pv_test.t WHERE id=1;")
if [ "$JSON_RESULT" != "1" ]; then
    echo "::error::JSON_EXTRACT returned '$JSON_RESULT', expected '1'"
    exit 1
fi
echo "  ✓ JSON_EXTRACT"

FT_HITS=$("${MYSQL[@]}" -e "SELECT COUNT(*) FROM pv_test.t WHERE MATCH(name) AGAINST('apple');")
if [ "$FT_HITS" -lt 1 ]; then
    echo "::error::Fulltext MATCH returned $FT_HITS hits, expected >=1"
    exit 1
fi
echo "  ✓ Fulltext MATCH (ICU)"

# 5. mysqldump → restore roundtrip
echo "== 5. mysqldump → restore roundtrip =="
DUMP_FILE="$WORK_DIR/dump.sql"
"$MYSQL_PREFIX/bin/mysqldump" --socket="$SOCK" -u root --databases pv_test > "$DUMP_FILE"
DUMP_SIZE=$(wc -c < "$DUMP_FILE" | tr -d ' ')

"${MYSQL[@]}" -e "CREATE DATABASE pv_test_restored;"
"$MYSQL_PREFIX/bin/mysql" --socket="$SOCK" -u root pv_test_restored \
    < <(sed 's/`pv_test`/`pv_test_restored`/g' "$DUMP_FILE")

ORIG_ROWS=$("${MYSQL[@]}" -e "SELECT COUNT(*) FROM pv_test.t;")
REST_ROWS=$("${MYSQL[@]}" -e "SELECT COUNT(*) FROM pv_test_restored.t;")
if [ "$ORIG_ROWS" != "$REST_ROWS" ]; then
    echo "::error::Row count mismatch: original=$ORIG_ROWS restored=$REST_ROWS"
    exit 1
fi
echo "  ✓ dumped $DUMP_SIZE bytes; restored row count matches ($ORIG_ROWS)"

# 6. mysqlslap — exercises connection handling + threading
echo "== 6. mysqlslap (4 clients × 20 iterations) =="
"$MYSQL_PREFIX/bin/mysqlslap" --socket="$SOCK" -u root \
    --concurrency=4 --iterations=20 \
    --auto-generate-sql --auto-generate-sql-load-type=mixed \
    --number-of-queries=200 2>&1 | grep -E '(seconds to run|Average|Minimum|Maximum)' | head

# 7. clean shutdown
echo "== 7. shutdown =="
"$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root shutdown
wait "$MYSQL_PID" 2>/dev/null || true
MYSQL_PID=""
echo "  ✓ clean shutdown"

echo ""
echo "== ALL CHECKS PASSED for $SERVER_VER =="
```

- [ ] **Step 2: Make the script executable**

```bash
chmod +x scripts/test-mysql-bundle.sh
```

- [ ] **Step 3: Validate the script locally against a real bundle**

```bash
# Re-extract a fresh tarball (or reuse Task 1's if you kept it)
mkdir -p /tmp/mysql-smoke && cd /tmp/mysql-smoke
curl -fsSL -o mysql.tar.gz \
  "https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.9-macos15-arm64.tar.gz"
tar -xzf mysql.tar.gz
cd -

bash scripts/test-mysql-bundle.sh /tmp/mysql-smoke/mysql-8.4.9-macos15-arm64
```

Expected output: every step emits `✓`, ends with `== ALL CHECKS PASSED for 8.4.9 ==`, exit code 0.

If anything fails, fix the script and re-run. Common pitfalls:
- Port `33099` already in use → set `MYSQL_TEST_PORT=33199` in the env.
- Old datadir from a previous run → script uses `mktemp -d`, this shouldn't happen.

- [ ] **Step 4: Clean up the smoke-test scratch dir**

```bash
rm -rf /tmp/mysql-smoke
```

- [ ] **Step 5: Commit**

```bash
git add scripts/test-mysql-bundle.sh
git commit -m "ci: add test-mysql-bundle.sh integration test

Mirrors scripts/test-postgres-bundle.sh. Exercises CRUD + JSON +
fulltext (ICU) + mysqldump roundtrip + mysqlslap against a staged
MySQL bundle. Used by the upcoming mysql job in build-artifacts.yml."
```

---

## Task 3: Add the `mysql` job to `build-artifacts.yml`

Insert the new job between the existing `postgres` job and the `release` job. Whole job is self-contained; no need to modify `frankenphp` or `postgres`.

**Files:**
- Modify: `.github/workflows/build-artifacts.yml` — insert new job at line 285 (immediately before `release:`).

- [ ] **Step 1: Add the `mysql` job**

Open `.github/workflows/build-artifacts.yml`. Find the line that reads `  release:` (it's the start of the existing release job, currently line 285). **Insert** the following job *before* that line, keeping the existing `release:` job intact below it:

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
          echo "Downloading $URL"
          curl -fsSL -o mysql.tar.gz "$URL"
          ls -lh mysql.tar.gz

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
          # Drop debug build (~201M), mecab Japanese FTS dictionaries (~119M),
          # debug plugin variants (~18M), non-English locales (~10M), and
          # everything irrelevant at runtime.
          rm -f  "$STAGING/bin/mysqld-debug"
          rm -rf "$STAGING/lib/mecab"
          rm -rf "$STAGING/lib/plugin/debug"
          rm -rf "$STAGING/lib/pkgconfig"
          rm -rf "$STAGING/docs" "$STAGING/man" "$STAGING/support-files" "$STAGING/include"
          # Keep share/english + share/charsets (mandatory at runtime); drop other locales.
          find "$STAGING/share" -mindepth 1 -maxdepth 1 -type d \
            ! -name english ! -name charsets -exec rm -rf {} +

      - name: Defensive install_name fixup on libfido2
        run: |
          set -euo pipefail
          # libfido2's LC_ID_DYLIB ships with /usr/local/mysql/lib/libfido2.1.dylib
          # — cosmetic; nothing in the bundle links via that path. We rewrite it
          # so the verify step below can be strict about /usr/local/mysql refs.
          FIDO=$(ls "$STAGING"/lib/libfido2.*.dylib 2>/dev/null | head -1 || true)
          if [ -n "$FIDO" ]; then
            install_name_tool -id "@loader_path/libfido2.1.dylib" "$FIDO"
            echo "Fixed up $FIDO"
          else
            echo "No libfido2 dylib found (unexpected — research showed this should be present)"
            exit 1
          fi

      - name: Verify no build-host paths
        run: |
          set -euo pipefail
          shopt -s nullglob
          LEAKS=0
          # Walk every Mach-O recursively under bin/ and lib/ (plugins live in lib/plugin/).
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

**Important YAML formatting notes:**
- Two-space indentation throughout (matches the file's existing style).
- The blank line at the end is intentional — it separates `mysql` from the existing `release:` job.
- `permissions: contents: read` is required even though it's the default, to match the postgres job's explicit declaration.

- [ ] **Step 2: Validate YAML syntax**

```bash
# If you have actionlint installed (recommended):
actionlint .github/workflows/build-artifacts.yml

# If not, at minimum a Python parse:
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-artifacts.yml'))"
```

Expected: no errors. If actionlint flags anything, fix before continuing.

(Per CLAUDE.md, prefer Go/shell tooling. `actionlint` is fine — it's a Go binary, and it's the standard linter for GH Actions. `python3 -c` is just a fallback for YAML well-formedness; if neither is available locally, skip and rely on CI.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: add mysql job for 8.0/8.4/9.7 macOS arm64 bundles

Downloads Oracle's official mysql-X.Y.Z-macos15-arm64.tar.gz from
cdn.mysql.com (archives/ for 8.0, Downloads/ for 8.4 and 9.7), strips
mysqld-debug + mecab + debug plugins + non-English locales, applies
a defensive install_name_tool fixup on libfido2, verifies no host-path
leaks, smoke-tests via scripts/test-mysql-bundle.sh, and uploads a
slimmed (~86MB compressed) tarball as a workflow artifact.

Does not yet wire into the release job — that's the next commit.

Spec: docs/superpowers/specs/2026-04-30-mysql-artifacts-design.md"
```

---

## Task 4: Wire `mysql` artifacts into the `release` job

The `release` job runs only on `main`. It needs to (1) wait on the new `mysql` job, (2) download the new artifacts, (3) validate count == 3, (4) copy to `release/`. The existing `gh release upload release/* --clobber` step handles publishing without changes.

**Files:**
- Modify: `.github/workflows/build-artifacts.yml`:
  - Line 286 (`needs:` of release job)
  - Lines 305–308 area (add a new download step)
  - Lines 340–346 area (add mysql validation block)
  - Lines 359–362 area (add mysql copy block)

- [ ] **Step 1: Update `release.needs`**

Find:

```yaml
  release:
    needs: [frankenphp, postgres]
```

Replace with:

```yaml
  release:
    needs: [frankenphp, postgres, mysql]
```

- [ ] **Step 2: Add the mysql download step**

Find the existing block (around line 305):

```yaml
      - uses: actions/download-artifact@v4
        with:
          pattern: postgres-mac-arm64-*
          path: artifacts
```

**Insert immediately after it:**

```yaml
      - uses: actions/download-artifact@v4
        with:
          pattern: mysql-mac-arm64-*
          path: artifacts
```

- [ ] **Step 3: Add the mysql validation block in "Prepare release assets"**

Find this existing block (around line 340–346):

```bash
          # Postgres bundles: expect one per major in postgres.strategy.matrix.pg.
          # Keep this count in sync with that matrix when adding/removing majors.
          pg_dirs=(artifacts/postgres-mac-arm64-*)
          if [ ${#pg_dirs[@]} -ne 2 ]; then
            echo "::error::Expected 2 postgres bundles (17 + 18), found ${#pg_dirs[@]}"
            exit 1
          fi
```

**Insert immediately after that closing `fi`:**

```bash

          # MySQL bundles: expect one per major in mysql.strategy.matrix.include.
          # Keep this count in sync with that matrix when adding/removing majors.
          mysql_dirs=(artifacts/mysql-mac-arm64-*)
          if [ ${#mysql_dirs[@]} -ne 3 ]; then
            echo "::error::Expected 3 mysql bundles (8.0 + 8.4 + 9.7), found ${#mysql_dirs[@]}"
            exit 1
          fi
```

(Leading blank line keeps the two validation blocks visually separate.)

- [ ] **Step 4: Add the mysql copy block**

Find the existing block (around line 359–362):

```bash
          # Postgres tarballs: copy as-is (no chmod needed).
          for dir in "${pg_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
          ls -la release/
```

**Insert before the `ls -la release/` line:**

```bash
          # MySQL tarballs: copy as-is (no chmod needed).
          for dir in "${mysql_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
```

After the edit, that section reads:

```bash
          # Postgres tarballs: copy as-is (no chmod needed).
          for dir in "${pg_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
          # MySQL tarballs: copy as-is (no chmod needed).
          for dir in "${mysql_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
          ls -la release/
```

- [ ] **Step 5: Validate YAML syntax**

```bash
actionlint .github/workflows/build-artifacts.yml
# or
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-artifacts.yml'))"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: publish mysql bundles to artifacts release

release job now waits on the mysql job, downloads its 3 artifacts,
validates the count matches the matrix, and copies the tarballs to
release/ alongside FrankenPHP, PHP CLI, and Postgres assets. The
existing 'gh release upload release/* --clobber' step picks them up
without changes."
```

---

## Task 5: Update first-time release-create title and notes

The `gh release view "$TAG" >/dev/null 2>&1` guard means the title/notes only fire if the release is ever re-created from scratch. This is a no-op for the current live `artifacts` release. We update it anyway so the file is honest.

**Files:**
- Modify: `.github/workflows/build-artifacts.yml`, lines 376–383 area.

- [ ] **Step 1: Update the title and notes**

Find:

```yaml
          if ! gh release view "$TAG" --repo "${{ github.repository }}" >/dev/null 2>&1; then
            echo "Creating $TAG release"
            gh release create "$TAG" \
              --repo "${{ github.repository }}" \
              --title "PHP binaries (FrankenPHP + static PHP CLI)" \
              --notes "Rolling release of FrankenPHP and static PHP CLI binaries, rebuilt weekly by build-artifacts.yml. Not a versioned pv release." \
              --latest=false
          fi
```

Replace `--title` and `--notes` with:

```yaml
          if ! gh release view "$TAG" --repo "${{ github.repository }}" >/dev/null 2>&1; then
            echo "Creating $TAG release"
            gh release create "$TAG" \
              --repo "${{ github.repository }}" \
              --title "pv binaries (FrankenPHP, PHP CLI, PostgreSQL, MySQL)" \
              --notes "Rolling release of binaries used by pv: FrankenPHP, static PHP CLI, PostgreSQL, and MySQL. Rebuilt weekly by build-artifacts.yml. Not a versioned pv release." \
              --latest=false
          fi
```

- [ ] **Step 2: Update the upload echo line for accuracy (optional but cheap)**

Find:

```yaml
          echo "Uploading FrankenPHP + PHP CLI assets to $TAG"
```

Replace with:

```yaml
          echo "Uploading all binaries to $TAG"
```

- [ ] **Step 3: Validate YAML syntax**

```bash
actionlint .github/workflows/build-artifacts.yml
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: update artifacts release title and notes

Mention PostgreSQL and MySQL alongside the original PHP binaries.
No-op for the current live release (the gh release view guard skips
re-creation), but keeps the workflow file honest."
```

---

## Task 6: End-to-end CI verification

Push the branch, dispatch the workflow, inspect artifacts. The `release` job is gated on `github.ref == 'refs/heads/main'`, so dispatching from the feature branch runs `frankenphp` + `postgres` + `mysql` jobs without touching the live release.

**Files:**
- No changes. Only verification.

- [ ] **Step 1: Push the feature branch**

```bash
# Branch should already be created by the worktree.
# If not, create one now:
#   git checkout -b mysql-artifacts
git push -u origin HEAD
```

- [ ] **Step 2: Dispatch the workflow on the feature branch**

```bash
gh workflow run build-artifacts.yml --ref "$(git rev-parse --abbrev-ref HEAD)"
```

Wait a few seconds, then:

```bash
gh run list --workflow=build-artifacts.yml --branch "$(git rev-parse --abbrev-ref HEAD)" --limit 3
```

Expected: a run in `queued` or `in_progress` state.

- [ ] **Step 3: Watch the run**

```bash
RUN_ID=$(gh run list --workflow=build-artifacts.yml --branch "$(git rev-parse --abbrev-ref HEAD)" --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$RUN_ID"
```

Expected: all three `MySQL 8.0` / `MySQL 8.4` / `MySQL 9.7` jobs succeed. The `release` job is **skipped** (correct — we're not on main).

If a job fails:
- "Download Oracle tarball" step fails → check Oracle CDN; if a 404, the patch version in the matrix may have been pulled; bump it and re-run.
- "Verify no build-host paths" step fails → Oracle shipped a new dependency that links to a host path. Read the leak output, decide whether to add a defensive `install_name_tool -change` or expand the strip list. Update spec + plan + workflow.
- "Smoke test" fails → pull the workflow log; re-run `scripts/test-mysql-bundle.sh` locally against the same patch version. Most likely a regression in the script itself.

- [ ] **Step 4: Spot-check the produced artifacts**

```bash
mkdir -p /tmp/mysql-ci-check && cd /tmp/mysql-ci-check
gh run download "$RUN_ID" -n mysql-mac-arm64-8.4
ls -lh mysql-mac-arm64-8.4.tar.gz

# Should be ~86 MB compressed.
# Extract and run the smoke test against the produced bundle:
mkdir extracted && tar -xzf mysql-mac-arm64-8.4.tar.gz -C extracted
cd -
bash scripts/test-mysql-bundle.sh /tmp/mysql-ci-check/extracted
```

Expected: smoke test passes (0 exit code), `== ALL CHECKS PASSED for 8.4.9 ==`.

```bash
rm -rf /tmp/mysql-ci-check
```

- [ ] **Step 5: Open PR**

```bash
gh pr create --title "ci: add MySQL artifacts pipeline (8.0/8.4/9.7, macOS arm64)" \
  --body "$(cat <<'EOF'
## Summary
- Adds a new `mysql` job to `build-artifacts.yml` that downloads, strips, smoke-tests, and uploads Oracle's official MySQL Community Server 8.0.43 / 8.4.9 / 9.7.0 macOS arm64 bundles.
- Wires those bundles into the existing rolling `artifacts` release alongside FrankenPHP + PHP CLI + Postgres.
- Adds `scripts/test-mysql-bundle.sh` (mirror of `test-postgres-bundle.sh`) — boots a real mysqld and exercises CRUD + JSON + fulltext (ICU) + mysqldump roundtrip + mysqlslap.

Does not replace the existing Docker MySQL service. This PR is foundation only — pv-side consumer code (downloader, `internal/mysqlenv/`, daemon integration) follows in a separate spec.

Spec: `docs/superpowers/specs/2026-04-30-mysql-artifacts-design.md`

## Test plan
- [x] Local validation: 0 host-path leaks after libfido2 fixup on Oracle's 8.4.9 tarball
- [x] Local smoke test: `scripts/test-mysql-bundle.sh` passes against 8.4.9
- [x] CI dispatch from feature branch: all three MySQL jobs green; release job correctly skipped
- [x] Spot-check downloaded CI artifact: extracted bundle passes smoke test on a clean machine
EOF
)"
```

- [ ] **Step 6: After PR merges to main, verify live release update**

The next scheduled run (or a manual dispatch from `main`) will publish the three new tarballs. Confirm:

```bash
gh release view artifacts | grep mysql-mac-arm64
```

Expected: three lines listing `mysql-mac-arm64-8.0.tar.gz`, `mysql-mac-arm64-8.4.tar.gz`, `mysql-mac-arm64-9.7.tar.gz`.

---

## Self-review

**Spec coverage:**
- "New `mysql` job" — Task 3 ✓
- "Extended `release` job (`needs`, download, validate, copy)" — Task 4 ✓
- "Updated release notes/title" — Task 5 ✓
- "New `scripts/test-mysql-bundle.sh`" — Task 2 ✓
- "Defensive `install_name_tool -id` on libfido2" — Task 3, "Defensive install_name fixup" step ✓
- "Verify no host-path leaks" — Task 3, "Verify no build-host paths" step ✓
- "Smoke test (CRUD + JSON + fulltext + dump roundtrip + mysqlslap)" — Task 2 script content ✓
- "Strip plan (mysqld-debug, mecab, plugin/debug, non-English locales, docs/man/support-files/include/pkgconfig)" — Task 3, "Strip" step ✓
- "Hardcoded version table (8.0.43 / 8.4.9 / 9.7.0)" — Task 3, `matrix.include` block ✓
- "End-to-end manual verification" — Task 6 ✓

**Placeholder scan:** No "TBD" / "TODO" / "implement later" / "similar to Task N" / "appropriate error handling". Every code block is the literal content. ✓

**Type / name consistency:**
- `MYSQL_PREFIX` used consistently in test-mysql-bundle.sh ✓
- `STAGING` env var consistent across workflow steps ✓
- Asset names `mysql-mac-arm64-{major}.tar.gz` consistent across job, release-job validation, and PR body ✓
- Matrix variable `matrix.major` (not `matrix.mysql`) used consistently ✓
- The exact patch versions `8.0.43` / `8.4.9` / `9.7.0` appear identically in the spec and the matrix ✓

No issues found. Plan ready.
