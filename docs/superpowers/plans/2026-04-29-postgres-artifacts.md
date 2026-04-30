# Postgres Artifacts Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `postgres` job to `.github/workflows/build-artifacts.yml` that mirrors theseus-rs's PostgreSQL bundles (PG 17 + 18, macOS arm64), patches Homebrew-pinned openssl `install_name`s to relative `@executable_path` references, runs an in-CI smoke test, and publishes the corrected bundles to the existing rolling `artifacts` release on `prvious/pv`.

**Architecture:** Single workflow file, single rolling release. The new `postgres` job runs in parallel with the existing `build` job on `macos-15` (required because `install_name_tool` and `codesign` are darwin-only). Steps: download from theseus-rs, bundle Homebrew openssl into the staging tree, patch `install_name`s on every Mach-O binary/dylib, ad-hoc codesign, verify no Homebrew paths remain, smoke-test (initdb + start + psql query), strip docs, repack, upload.

**Tech Stack:** GitHub Actions YAML, bash, `gh api` (for theseus-rs release listing), `curl`, `tar`, `install_name_tool`, `codesign`, `otool`.

**Spec:** `docs/superpowers/specs/2026-04-29-postgres-artifacts-design.md`

---

## File Structure

Single file modified — no new files.

| File | Change | Responsibility |
|---|---|---|
| `.github/workflows/build-artifacts.yml` | Modify | Add `postgres` matrix job (macos-15) + extend `release` job to consume postgres workflow artifacts |

The existing `resolve-version` and `build` jobs stay untouched. The `postgres` job runs in parallel with them. The `release` job's existing PHP/FrankenPHP logic stays intact; we only append postgres handling.

---

## Task 1: Pre-flight verification (already done)

Verified locally before plan execution:

- ✅ theseus-rs publishes both PG 17.9.0 and PG 18.3.0 with `aarch64-apple-darwin` assets.
- ✅ Both archives ship 37 binaries (`postgres`, `initdb`, `pg_ctl`, `psql`, `pg_dump`, `pg_restore`, `pg_config`, `pg_isready`, `createdb`, etc.) and an `include/` directory.
- ✅ Both archives' `bin/postgres` references `/opt/homebrew/opt/openssl@3/lib/lib{ssl,crypto}.3.dylib` and otherwise only `/usr/lib/*` system libs.
- ✅ End-to-end pipeline verified locally (download → bundle Homebrew openssl → `install_name_tool` patching → ad-hoc codesign → `initdb` → start postgres → `psql SELECT version()` → teardown) for both PG 17.9.0 and PG 18.3.0.

No further pre-flight needed.

---

## Task 2: Feature branch (already done)

Branch `feat/postgres-artifacts` already exists and is checked out. The
prior bad commit (zonkyio-based postgres job) has been reset; the branch
is currently at `23efcea` (parent of main's plan/spec commits).

---

## Task 3: Add the `postgres` job to the workflow

**Files:**
- Modify: `.github/workflows/build-artifacts.yml`

Insert a new top-level job named `postgres` between the existing `build` job (ends around line 155) and the existing `release` job (starts around line 157). The new job has no `needs:` and runs in parallel with `build`.

- [ ] **Step 1: Read the file to confirm the insertion point**

```bash
sed -n '150,165p' .github/workflows/build-artifacts.yml
```

Expected: shows the tail of the `build` job's `Upload PHP CLI` step and the start of the `release` job. The insertion point is the blank line between them.

- [ ] **Step 2: Insert the `postgres` job**

Use Edit to insert this block immediately before `  release:`. The new content goes between the existing `build` job and `release` job, indented at 2 spaces for the job key (matching the `build` and `release` siblings).

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

      - name: Download and extract theseus-rs bundle
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
          # Rewrite LC_ID_DYLIB on bundled openssl (their own identity)
          install_name_tool -id "@executable_path/../lib/libssl.3.dylib"   "$STAGING/lib/libssl.3.dylib"
          install_name_tool -id "@executable_path/../lib/libcrypto.3.dylib" "$STAGING/lib/libcrypto.3.dylib"
          # libssl carries a hardcoded reference to libcrypto via Homebrew Cellar version path.
          # Discover it from otool -L rather than reconstructing the path.
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
        run: rm -rf "$STAGING/share/doc"

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
          test -d "$STAGING/share/extension"
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

(Note the trailing blank line — preserves spacing before `release:`.)

- [ ] **Step 3: Verify the file parses as YAML**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-artifacts.yml'))" && echo "YAML OK"
```

Expected: `YAML OK`. If yaml fails, the error message will point at the offending line — fix indentation and re-run.

- [ ] **Step 4: Verify job structure with grep**

```bash
grep -n '^  [a-z-]*:$' .github/workflows/build-artifacts.yml
```

Expected: four lines listing the four top-level jobs in order:
```
  resolve-version:
  build:
  postgres:
  release:
```

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: add postgres job mirroring theseus-rs bundles for PG 17/18"
```

---

## Task 4: Wire postgres into the `release` job

**Files:**
- Modify: `.github/workflows/build-artifacts.yml`

Three small edits to the existing `release` job:
1. Add `postgres` to `needs:`
2. Add a `download-artifact` step for the postgres workflow artifacts
3. Add postgres validation + staging in the existing `Prepare release assets` step

- [ ] **Step 1: Update `needs:` on the `release` job**

The current line is:

```yaml
  release:
    needs: [resolve-version, build]
```

Change it to:

```yaml
  release:
    needs: [resolve-version, build, postgres]
```

Edit:
- old_string: `    needs: [resolve-version, build]`
- new_string: `    needs: [resolve-version, build, postgres]`

- [ ] **Step 2: Add `download-artifact` step for postgres**

Add this step in the `release` job's `steps:` block, **immediately after** the existing `download-artifact` step that downloads `php-mac-*`. Insert before the `Prepare release assets` step.

The existing block to find and anchor on:

```yaml
      - uses: actions/download-artifact@v4
        with:
          pattern: php-mac-*
          path: artifacts

      - name: Prepare release assets
```

Replace with:

```yaml
      - uses: actions/download-artifact@v4
        with:
          pattern: php-mac-*
          path: artifacts

      - uses: actions/download-artifact@v4
        with:
          pattern: postgres-mac-arm64-*
          path: artifacts

      - name: Prepare release assets
```

- [ ] **Step 3: Extend `Prepare release assets` with postgres validation + staging**

Inside the existing `Prepare release assets` step's `run:` block, find the validation block that ends with the FrankenPHP↔PHP-CLI matching check, and add postgres validation.

Find this anchor (existing code):

```bash
          for dir in "${fp_dirs[@]}"; do
            suffix="${dir#artifacts/frankenphp-}"
            if [ ! -d "artifacts/php-${suffix}" ]; then
              echo "::error::Missing PHP CLI artifact for frankenphp-${suffix}"
              exit 1
            fi
          done

          mkdir -p release
```

Replace with:

```bash
          for dir in "${fp_dirs[@]}"; do
            suffix="${dir#artifacts/frankenphp-}"
            if [ ! -d "artifacts/php-${suffix}" ]; then
              echo "::error::Missing PHP CLI artifact for frankenphp-${suffix}"
              exit 1
            fi
          done

          # Postgres bundles: expect both 17 and 18.
          pg_dirs=(artifacts/postgres-mac-arm64-*)
          if [ ${#pg_dirs[@]} -lt 2 ]; then
            echo "::error::Expected 2 postgres bundles (17 + 18), found ${#pg_dirs[@]}"
            exit 1
          fi

          mkdir -p release
```

Then add postgres staging at the end of the staging loops. Find this anchor (existing code):

```bash
          # PHP CLI tarballs: just copy as-is (no chmod needed).
          for dir in "${php_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
          ls -la release/
```

Replace with:

```bash
          # PHP CLI tarballs: just copy as-is (no chmod needed).
          for dir in "${php_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
          # Postgres tarballs: copy as-is (no chmod needed).
          for dir in "${pg_dirs[@]}"; do
            cp "$dir"/* "release/"
          done
          ls -la release/
```

- [ ] **Step 4: Verify the file still parses as YAML**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-artifacts.yml'))" && echo "YAML OK"
```

Expected: `YAML OK`.

- [ ] **Step 5: Verify the `release` job's `needs:` was updated**

```bash
grep -A1 '^  release:' .github/workflows/build-artifacts.yml | head -2
```

Expected:
```
  release:
    needs: [resolve-version, build, postgres]
```

- [ ] **Step 6: Verify the postgres validation block is in place**

```bash
grep -n 'pg_dirs' .github/workflows/build-artifacts.yml
```

Expected: 3 lines — array assignment, count check, staging loop.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: wire postgres bundles into release job"
```

---

## Task 5: Push and dispatch from the feature branch

**Files:** none (CI verification)

The feature-branch dispatch runs `build` and `postgres` but skips `release` (gated on `github.ref == 'refs/heads/main'`). This validates the new job without touching the live `artifacts` release.

- [ ] **Step 1: Push the feature branch**

```bash
git push -u origin feat/postgres-artifacts
```

Expected: `* [new branch] feat/postgres-artifacts -> feat/postgres-artifacts`.

- [ ] **Step 2: Dispatch the workflow from the feature branch**

```bash
gh workflow run build-artifacts.yml --ref feat/postgres-artifacts
```

Expected: `✓ Created workflow_dispatch event for build-artifacts.yml at feat/postgres-artifacts`.

- [ ] **Step 3: Watch the workflow**

```bash
sleep 5
gh run list --workflow=build-artifacts.yml --branch=feat/postgres-artifacts --limit 1
```

Note the run ID, then:

```bash
gh run watch <RUN_ID>
```

Expected: `build` matrix and `postgres` matrix both complete successfully. `release` is skipped.

If `postgres` fails: inspect with `gh run view <RUN_ID> --log-failed`. Common failure modes:
- theseus-rs API/release URL changed → fix URL.
- Homebrew openssl path on `macos-15` runner changed → adjust `cp` source path.
- Smoke test fails: `psql` returned non-zero or unexpected output → check the server log printed by the workflow.

- [ ] **Step 4: Inspect the workflow artifacts**

```bash
gh run download <RUN_ID> --name postgres-mac-arm64-17
gh run download <RUN_ID> --name postgres-mac-arm64-18
ls -lh postgres-mac-arm64-*.tar.gz
tar -tzf postgres-mac-arm64-17.tar.gz | head -20
tar -tzf postgres-mac-arm64-17.tar.gz | grep -E 'bin/postgres$|bin/psql$|bin/pg_dump$|bin/pg_config$|lib/libssl|include' | head
```

Expected:
- File sizes ~10-15 MB compressed each.
- `tar -tzf` shows `bin/`, `lib/`, `share/`, `include/` at the root (flat structure).
- All key binaries (`postgres`, `psql`, `pg_dump`, `pg_config`) present.
- `include/` directory present.

- [ ] **Step 5: Verify dependencies on a downloaded bundle**

```bash
mkdir /tmp/pg-verify && cd /tmp/pg-verify && tar -xzf "$OLDPWD/postgres-mac-arm64-18.tar.gz"
otool -L bin/postgres
cd "$OLDPWD"
```

Expected: every line should reference either `@executable_path/...` or `/usr/lib/...` — NO `/opt/homebrew/...` paths.

- [ ] **Step 6: Clean up downloaded test artifacts**

```bash
rm -f postgres-mac-arm64-17.tar.gz postgres-mac-arm64-18.tar.gz
rm -rf /tmp/pg-verify
```

---

## Task 6: Open a PR

**Files:** none (GitHub PR creation)

- [ ] **Step 1: Open the PR**

```bash
gh pr create --title "ci: add postgres artifacts pipeline (PG 17/18, macOS arm64)" --body "$(cat <<'EOF'
## Summary
- Adds a new `postgres` job to `build-artifacts.yml` that mirrors theseus-rs's PostgreSQL bundles (PG 17 + 18, macOS arm64) from GitHub Releases
- Patches Homebrew-pinned openssl `install_name`s to relative `@executable_path/../lib/...` references so bundles run on Macs without Homebrew openssl@3 installed
- Bundles `libssl.3.dylib` + `libcrypto.3.dylib` from the runner's Homebrew openssl@3 into the archive's `lib/`
- Ad-hoc codesigns every Mach-O after patching to satisfy macOS Gatekeeper
- In-CI smoke test: `initdb` + start postgres + `psql SELECT version()` + clean teardown — catches runtime breakage before publishing
- Released to the existing rolling `artifacts` tag on `prvious/pv` alongside FrankenPHP + PHP CLI assets
- Weekly cron + manual dispatch, parallel to existing FrankenPHP build

Spec: `docs/superpowers/specs/2026-04-29-postgres-artifacts-design.md`

## Test plan
- [x] Local end-to-end pipeline verified for both PG 17.9.0 and PG 18.3.0 (download → patch → codesign → initdb → postgres → psql roundtrip)
- [x] YAML validates with `python3 -c "import yaml; yaml.safe_load(...)"`
- [x] Feature-branch workflow dispatch: `build` and `postgres` jobs both succeed; `release` correctly skipped
- [x] Workflow artifacts inspected: ~10-15 MB tarballs, flat root, all 37 binaries present, `include/` present
- [x] `otool -L bin/postgres` on downloaded bundle shows no `/opt/homebrew/*` references
- [ ] After merge: first scheduled (or dispatched) run from `main` publishes `postgres-mac-arm64-17.tar.gz` and `postgres-mac-arm64-18.tar.gz` to the `artifacts` release
EOF
)"
```

- [ ] **Step 2: Note the PR URL**

The command above prints the PR URL. Save it for the post-merge verification step.

---

## Task 7: After merge — verify live release

**Files:** none (post-merge verification, only after PR is merged to main)

- [ ] **Step 1: Confirm merge and switch back to main**

```bash
git checkout main
git pull
```

- [ ] **Step 2: Trigger the workflow from main**

The weekly cron will fire on the next Monday, but a manual dispatch verifies immediately:

```bash
gh workflow run build-artifacts.yml --ref main
sleep 5
gh run list --workflow=build-artifacts.yml --branch=main --limit 1
gh run watch <RUN_ID>
```

Expected: all four build-side jobs (`resolve-version`, `build`, `postgres`) succeed, then `release` runs and uploads.

- [ ] **Step 3: Verify assets are live**

```bash
gh release view artifacts --json assets --jq '.assets[].name' | grep '^postgres-'
```

Expected:
```
postgres-mac-arm64-17.tar.gz
postgres-mac-arm64-18.tar.gz
```

- [ ] **Step 4: Spot-check by downloading one asset**

```bash
gh release download artifacts -p 'postgres-mac-arm64-17.tar.gz' -O /tmp/pg17-live.tar.gz
ls -lh /tmp/pg17-live.tar.gz
tar -tzf /tmp/pg17-live.tar.gz | head -10
rm /tmp/pg17-live.tar.gz
```

Expected: ~10-15 MB, flat root with `bin/`, `lib/`, `share/`, `include/`.

If everything verifies, the artifacts pipeline is live and the weekly cron will keep it fresh from here on.
