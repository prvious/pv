# Postgres Artifacts Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `postgres` job to `.github/workflows/build-artifacts.yml` that mirrors zonkyio's PostgreSQL bundles (PG 17 + 18, macOS arm64) from Maven Central into the existing rolling `artifacts` release on `prvious/pv`.

**Architecture:** Single workflow file, single rolling release. The new `postgres` job runs in parallel with the existing `build` job on `ubuntu-latest`, downloads the JAR from Maven, unwraps it (zip → xz tarball), strips docs, packs as `postgres-mac-arm64-{17,18}.tar.gz`, and hands off to the existing `release` job. The `release` job is extended to also wait on `postgres`, download the new workflow artifacts, validate them, and stage them for the existing `gh release upload --clobber` step.

**Tech Stack:** GitHub Actions YAML, bash, `curl`, `unzip`, `tar` (with xz support, available on `ubuntu-latest`), `gh` CLI.

**Spec:** `docs/superpowers/specs/2026-04-29-postgres-artifacts-design.md`

---

## File Structure

Single file modified — no new files.

| File | Change | Responsibility |
|---|---|---|
| `.github/workflows/build-artifacts.yml` | Modify | Add `postgres` matrix job; extend `release` job to consume postgres workflow artifacts |

The existing `resolve-version` and `build` jobs stay untouched. The `postgres` job is independent of them and runs in parallel. The `release` job's existing PHP/FrankenPHP logic stays intact; we only append postgres handling.

---

## Task 1: Pre-flight — verify Maven version resolution works

**Files:** none (local sanity check before touching the workflow)

This is a quick local check to confirm the Maven Central metadata URL and the version-grep one-liner produce sane results. If this fails, the workflow would fail too — better to catch it now.

- [ ] **Step 1: Run the resolution one-liner for PG 17**

```bash
curl -fsSL "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-darwin-arm64v8/maven-metadata.xml" \
  | grep -oE '<version>[^<]+' \
  | sed 's/<version>//' \
  | grep "^17\." \
  | sort -V \
  | tail -1
```

Expected: a single line like `17.6.0` (the actual patch version may differ — anything in the form `17.X.Y` is correct).

- [ ] **Step 2: Run the resolution one-liner for PG 18**

```bash
curl -fsSL "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-darwin-arm64v8/maven-metadata.xml" \
  | grep -oE '<version>[^<]+' \
  | sed 's/<version>//' \
  | grep "^18\." \
  | sort -V \
  | tail -1
```

Expected: `18.X.Y` (e.g. `18.3.0`).

- [ ] **Step 3: Verify the JAR URL pattern downloads**

Use the version from Step 2:

```bash
VER="18.3.0"  # substitute the version from Step 2
curl -fsSI "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-darwin-arm64v8/${VER}/embedded-postgres-binaries-darwin-arm64v8-${VER}.jar" | head -1
```

Expected: `HTTP/2 200` (or `HTTP/1.1 200 OK`). Confirms the URL pattern is correct and the asset exists.

If any of these three steps return empty / 404 / unexpected output, **stop and re-investigate** — the workflow will inherit the same problem.

---

## Task 2: Create a feature branch for the work

**Files:** none

- [ ] **Step 1: Verify git is clean**

```bash
git status
```

Expected: `nothing to commit, working tree clean`. If not clean, stash or commit unrelated changes first.

- [ ] **Step 2: Create and switch to the feature branch**

```bash
git checkout -b feat/postgres-artifacts
```

- [ ] **Step 3: Confirm branch**

```bash
git branch --show-current
```

Expected: `feat/postgres-artifacts`.

---

## Task 3: Add the `postgres` job to the workflow

**Files:**
- Modify: `.github/workflows/build-artifacts.yml`

Insert a new top-level job named `postgres` between the existing `build` job (ends at line 155) and the existing `release` job (starts at line 157). The new job has no `needs:` and runs in parallel with `build`.

- [ ] **Step 1: Read the file to confirm the insertion point**

```bash
sed -n '150,165p' .github/workflows/build-artifacts.yml
```

Expected: shows the tail of the `build` job's `Upload PHP CLI` step (around line 150–155) and the start of the `release` job at line 157. The insertion point is the blank line between them.

- [ ] **Step 2: Insert the `postgres` job**

Use Edit to insert this block immediately before `  release:` at line 157. The new content goes between the existing `build` job and `release` job.

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
          set -euo pipefail
          META_URL="https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-darwin-arm64v8/maven-metadata.xml"
          ALL=$(curl -fsSL "$META_URL" | grep -oE '<version>[^<]+' | sed 's/<version>//')
          VER=$(echo "$ALL" | grep "^${{ matrix.pg }}\." | sort -V | tail -1)
          if [ -z "$VER" ]; then
            echo "::error::No ${{ matrix.pg }}.x version found on Maven Central"
            exit 1
          fi
          echo "Resolved PG ${{ matrix.pg }}.x → $VER"
          echo "version=$VER" >> "$GITHUB_OUTPUT"

      - name: Download and unwrap JAR
        run: |
          set -euo pipefail
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
          set -euo pipefail
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
          echo "Structural sanity checks passed."

      - name: Repack
        run: tar -czf "postgres-mac-arm64-${{ matrix.pg }}.tar.gz" -C staging bin lib share include

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

If `postgres:` is missing or in the wrong position, fix the insertion.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: add postgres job mirroring zonkyio bundles for PG 17/18"
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

Add this step in the `release` job's `steps:` block, **immediately after** the existing `download-artifact` step that downloads `php-mac-*` (currently lines 172–175). Insert before the `Prepare release assets` step.

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

Inside the existing `Prepare release assets` step's `run:` block, find the validation block that ends with the FrankenPHP↔PHP-CLI matching check, and add postgres validation + staging.

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

Then add postgres staging at the end of the `mkdir -p release` block. Find this anchor (existing code):

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

Expected: 3 lines — one for the array assignment, one for the count check, one for the staging loop.

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
sleep 5  # let GitHub register the dispatch
gh run list --workflow=build-artifacts.yml --branch=feat/postgres-artifacts --limit 1
```

Note the run ID, then:

```bash
gh run watch <RUN_ID>
```

Expected: `build` matrix and `postgres` matrix both complete successfully. `release` is skipped (gated on `refs/heads/main`).

If `postgres` fails: inspect logs with `gh run view <RUN_ID> --log-failed` and iterate. Common failure modes:
- Maven URL changed → fix URL in workflow.
- zonkyio published a malformed bundle → sanity check `test -f` fails → check the latest version manually with `tar -tJf unpacked/postgres-darwin-arm_64.txz`.
- YAML indentation issue → re-validate locally with `python3 -c "import yaml; …"`.

- [ ] **Step 4: Inspect the workflow artifacts**

```bash
gh run download <RUN_ID> --name postgres-mac-arm64-17
gh run download <RUN_ID> --name postgres-mac-arm64-18
ls -lh postgres-mac-arm64-*.tar.gz
tar -tzf postgres-mac-arm64-17.tar.gz | head -20
tar -tzf postgres-mac-arm64-17.tar.gz | grep -E 'bin/postgres$|lib/libssl' | head
```

Expected:
- File sizes ~30–35 MB each.
- `tar -tzf` shows `bin/`, `lib/`, `share/`, `include/` at the root (flat structure).
- `bin/postgres` and `lib/libssl.3.dylib` are present.

- [ ] **Step 5: Clean up downloaded test artifacts**

```bash
rm -f postgres-mac-arm64-17.tar.gz postgres-mac-arm64-18.tar.gz
```

---

## Task 6: Open a PR

**Files:** none (GitHub PR creation)

- [ ] **Step 1: Open the PR**

```bash
gh pr create --title "ci: add postgres artifacts pipeline (PG 17/18, macOS arm64)" --body "$(cat <<'EOF'
## Summary
- Adds a new `postgres` job to `build-artifacts.yml` that mirrors zonkyio's PostgreSQL bundles (PG 17 + 18, macOS arm64) from Maven Central
- Bundles are stripped of docs and repacked as `postgres-mac-arm64-{17,18}.tar.gz`
- Released to the existing rolling `artifacts` tag on `prvious/pv` alongside FrankenPHP + PHP CLI assets
- Weekly cron + manual dispatch, mirrors existing FrankenPHP pipeline shape

Spec: `docs/superpowers/specs/2026-04-29-postgres-artifacts-design.md`

## Test plan
- [x] Local Maven resolution one-liner returns valid versions for both PG 17 and 18
- [x] YAML validates with `python3 -c "import yaml; yaml.safe_load(...)"`
- [x] Feature-branch workflow dispatch: `build` and `postgres` jobs both succeed; `release` correctly skipped
- [x] Workflow artifacts inspected: ~30–35 MB tarballs, flat root, `bin/postgres` + `lib/libssl.3.dylib` present
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

Expected: all three build-side jobs (`resolve-version`, `build`, `postgres`) succeed, then `release` runs and uploads.

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

Expected: ~30–35 MB, flat root with `bin/`, `lib/`, `share/`, `include/`.

If everything verifies, the artifacts pipeline is live and the weekly cron will keep it fresh from here on. The pv-side consumer code (downloader, `internal/postgresenv/`, shims, daemon integration) is the next plan.
