# Per-version `php.ini` for pv

## Problem

A freshly installed pv runs PHP with no `php.ini` at all. `php -i` reports
`Loaded Configuration File: (none)`, the baked-in search path is the
static-php-cli default (`/usr/local/etc/php`, which doesn't exist on user
machines), and path-style directives (`session.save_path`, `sys_temp_dir`,
`upload_tmp_dir`, `openssl.cafile`, `curl.cainfo`) come up blank. PHP still
runs because most blanks fall back to system defaults at runtime, but:

- The user can't introspect or tune their install via the standard
  `php.ini` workflow.
- There's no surface for pv (or a future user feature) to drop in
  per-extension config such as Xdebug.
- It's a bad first impression: `php -i` looking empty is the visible
  symptom that prompted this work.

The PHP CLI and FrankenPHP for a given version are produced by the same
`static-php-cli --build-cli` invocation, so they must share a single
`php.ini` — not duplicate copies that can drift.

## Goals

1. Each installed PHP version has a writable `php.ini` and a `conf.d`
   scan directory under `~/.pv/php/<version>/`.
2. The PHP CLI shim and FrankenPHP (main + per-version) load that ini and
   conf.d for their version, with no per-machine path baking required.
3. The default ini is upstream `php.ini-development` verbatim, so it
   matches standard PHP documentation.
4. pv's own opinionated path defaults live in a separate, pv-managed
   `conf.d/00-pv.ini` that the user can override without editing pv files.
5. User edits to `php.ini` survive `pv php:update`. pv's managed
   `00-pv.ini` is regenerated on update.
6. Existing installs (PHP installed before this feature) get the new
   layout backfilled idempotently the next time the daemon starts or
   `php:install`/`php:update` runs.

## Non-goals

- `openssl.cafile` / `curl.cainfo` defaults. Needs a CA bundle source
  decision; deferred to a follow-up.
- A `php.ini-production` variant or `--production` switch. Easy to add
  later because the production template ships in the same tarball.
- Per-project ini overrides via `pv.yml`. The conf.d seam makes this
  cheap to add later (symlink a project ini into the scan dir at
  link-time), but it's out of scope here.
- Build-time INI baking via `--with-config-file-path` or
  `--with-hardcoded-ini`. Considered and rejected — see Approach.

## Approach

### Why runtime env, not build-time baking

static-php-cli accepts `--with-config-file-path=DIR` and
`--with-config-file-scan-dir=DIR`, which bake an absolute path into the
binary. Both fail for pv because the right path is
`~/.pv/php/<version>/etc`, where `~` resolves on the *user's* machine,
not the CI runner where the build happens. There's no way to make the
baked path relative to the executable.

Standard PHP also honors two runtime env vars:

- `PHPRC` — overrides the directory PHP searches for `php.ini`.
- `PHP_INI_SCAN_DIR` — overrides the conf.d scan directory.

Both are honored by the static PHP CLI and by FrankenPHP (it embeds the
same SAPI built from the same php-src). pv controls the two places
where each is launched — the bash shim at `~/.pv/bin/php` and the
`exec.Command` in `internal/server/frankenphp.go` — so we set both env
vars there. No build changes needed for the binaries themselves.

### Filesystem layout

```
~/.pv/php/<version>/
├── php                       ; static PHP CLI binary (existing)
├── frankenphp                ; FrankenPHP binary (existing)
├── etc/
│   ├── php.ini               ; per-version main ini, user-owned
│   └── php.ini-development   ; upstream reference, never modified after install
└── conf.d/
    └── 00-pv.ini             ; pv-managed path defaults, regenerated on update
```

Plus, created and owned by `EnsureIniLayout` for the per-version paths
referenced from `00-pv.ini`:

```
~/.pv/data/sessions/<version>/
~/.pv/data/tmp/<version>/
```

### Where the upstream template comes from

`php.ini-development` is a file in the php-src tree at
`dist/static-php-cli/source/php-src/php.ini-development` during the
build. Today `build-artifacts.yml` packages only the `php` binary into
`php-mac-${ARCH}-php${ver}.tar.gz`. We extend that step to copy
`php.ini-development` into the staging dir before `tar`, so the tarball
becomes `{php, php.ini-development}`.

This binds the ini template to the binary it was built from. The
alternative (`go:embed` in pv) would force pv to ship N templates for N
PHP versions and would drift any time php-src updates them.

### Install / update / uninstall lifecycle

`pv php:install <ver>`:
1. Download FrankenPHP and PHP CLI tarballs (existing).
2. Extract `php` → `~/.pv/php/<ver>/php` and `php.ini-development` →
   `~/.pv/php/<ver>/etc/php.ini-development`.
3. Call `phpenv.EnsureIniLayout(<ver>)`.

`EnsureIniLayout` is the one piece of new behaviour:
- `mkdir -p` of `etc/`, `conf.d/`, `~/.pv/data/sessions/<ver>/`,
  `~/.pv/data/tmp/<ver>/`.
- If `etc/php.ini` does **not** exist, copy
  `etc/php.ini-development` to it. If it exists, leave it alone.
- Always (re)write `conf.d/00-pv.ini` to the canonical content
  generated from the version. This is pv-managed — overwriting on each
  call is the contract.

`pv php:update`:
- Re-runs install. Step 2 re-extracts `php.ini-development` over the
  existing copy (so the reference file in `etc/` always tracks current
  php-src). Step 3's `EnsureIniLayout` preserves user `php.ini` and
  refreshes `00-pv.ini`. Net effect: binaries and pv-managed config
  update; user edits to `etc/php.ini` survive. `etc/php.ini-development`
  is treated as a read-only reference that pv owns — users who want a
  custom template should edit `etc/php.ini`, not `etc/php.ini-development`.

`pv php:uninstall <ver>`:
- Existing `os.RemoveAll(PhpVersionDir(version))` already removes
  `etc/` and `conf.d/`. No change. The `~/.pv/data/sessions/<ver>/` and
  `~/.pv/data/tmp/<ver>/` dirs are intentionally left in place — they
  hold runtime state (session blobs, upload tmpfiles) that may belong to
  a still-running process; cleaning them is outside this feature's
  scope.

**Backfill** for installs predating this feature: at daemon start, walk
`phpenv.InstalledVersions()` and call `EnsureIniLayout` for each. The
function is idempotent and cheap. Also called from
`phpenv.EnsureInstalled` after a successful install, so a fresh install
in a long-running daemon doesn't have to wait for restart.

### `00-pv.ini` content

Generated from a Go string template, written under
`~/.pv/php/<ver>/conf.d/00-pv.ini`. Paths are resolved at write-time
using `config.PvDir()` and the version arg into literal absolute paths
(PHP does not expand `~`). For example, on a machine where `$HOME` is
`/Users/jane` and the version is `8.4`, the file contains:

```ini
; Managed by pv — regenerated on every `pv php:install` / `pv php:update`.
; For your own overrides, create a sibling file like 99-local.ini —
; conf.d files load alphabetically and later files win.

date.timezone = UTC

session.save_path = "/Users/jane/.pv/data/sessions/8.4"
sys_temp_dir     = "/Users/jane/.pv/data/tmp/8.4"
upload_tmp_dir   = "/Users/jane/.pv/data/tmp/8.4"
```

### Wiring the env vars

New helpers in `internal/config/paths.go`:

```go
func PhpEtcDir(version string) string  { return filepath.Join(PhpVersionDir(version), "etc") }
func PhpConfDDir(version string) string { return filepath.Join(PhpVersionDir(version), "conf.d") }

// PhpEnv returns env vars that point a PHP/FrankenPHP process at the
// per-version php.ini and conf.d. Caller must pass a non-empty version.
func PhpEnv(version string) []string {
    return []string{
        "PHPRC=" + PhpEtcDir(version),
        "PHP_INI_SCAN_DIR=" + PhpConfDDir(version),
    }
}
```

**Consumer 1 — `php` shim** (`internal/tools/shims.go`,
`writePhpShim`). The existing template resolves `$VERSION` per-call via
`pv php:current`. We append two lines before the `exec`:

```bash
export PHPRC="$PV_PHP_DIR/$VERSION/etc"
export PHP_INI_SCAN_DIR="$PV_PHP_DIR/$VERSION/conf.d"
exec "$BINARY" "$@"
```

This works whether `php` is invoked from a terminal, a Make target,
Composer, or anything else — the env vars are exported in the shim's
own environment before exec.

**Consumer 2 — FrankenPHP launcher**
(`internal/server/frankenphp.go`). Both `StartFrankenPHP` and
`StartVersionFrankenPHP` already do
`cmd.Env = append(os.Environ(), config.CaddyEnv()...)`. We add
`config.PhpEnv(version)` to that list.

`StartVersionFrankenPHP` already has `version` in scope.
`StartFrankenPHP` (the main/global instance) currently passes
`version=""`. We resolve the global version from `config.LoadSettings()`
(`Defaults.PHP`) inside `StartFrankenPHP` and pass it through. If
settings are unreadable or empty, we skip the env var rather than
failing — pv has other paths that already error on missing global PHP,
so we don't add a second one here.

## Components and contracts

| Unit | Responsibility | Inputs | Outputs |
|---|---|---|---|
| `config.PhpEtcDir(ver)` | Per-version etc/ path | version string | path string |
| `config.PhpConfDDir(ver)` | Per-version conf.d/ path | version string | path string |
| `config.PhpEnv(ver)` | env vars for PHP/FrankenPHP | version string | `[]string` of two `KEY=VALUE` entries |
| `phpenv.EnsureIniLayout(ver)` | Creates dirs, copies ini-development to php.ini if absent, regenerates 00-pv.ini | version string | error |
| `phpenv.Install` / `phpenv.InstallProgress` (extended) | Extracts both `php` and `php.ini-development` from the tarball, then calls `EnsureIniLayout` | http client, version | error |
| `tools.writePhpShim` (extended) | Shim sets PHPRC and PHP_INI_SCAN_DIR before exec | (none) | error |
| `server.StartFrankenPHP` (extended) | Resolves global version, passes `config.PhpEnv` into `cmd.Env` | (none) | `*FrankenPHP, error` |
| `server.StartVersionFrankenPHP` (existing signature) | Passes `config.PhpEnv(version)` into `cmd.Env` | version | `*FrankenPHP, error` |

## Build-artifacts.yml change

In the `frankenphp` job, the `Package PHP CLI` step currently does:

```bash
mkdir -p dist/cli-staging
cp "$PHP_BIN" dist/cli-staging/php
tar -C dist/cli-staging -czf "dist/php-mac-${ARCH}-php${{ matrix.php }}.tar.gz" php
```

Extended to:

```bash
mkdir -p dist/cli-staging
cp "$PHP_BIN" dist/cli-staging/php
INI_SRC="dist/static-php-cli/source/php-src/php.ini-development"
test -f "$INI_SRC" || { echo "::error::missing $INI_SRC"; exit 1; }
cp "$INI_SRC" dist/cli-staging/php.ini-development
tar -C dist/cli-staging -czf "dist/php-mac-${ARCH}-php${{ matrix.php }}.tar.gz" php php.ini-development
```

The `test -f` is load-bearing: silent absence would ship a binary
without its template, surfacing only when a user runs `php:install` and
hits a missing-file error. Failing the build instead.

A follow-on sanity step extracts the produced tarball to a tmpdir and
asserts both files are present and `php` is executable.

## Testing

**Unit (`internal/config/paths_test.go`)** — table-test that
`PhpEtcDir`, `PhpConfDDir`, and `PhpEnv` produce the expected
HOME-relative results under `t.Setenv("HOME", t.TempDir())`.

**Unit (`internal/phpenv/install_test.go`)** — new tests for
`EnsureIniLayout`:
- Idempotent: call twice, second is no-op (no errors, content unchanged).
- Doesn't clobber existing `etc/php.ini` (write `; user content` to it,
  call EnsureIniLayout, expect file content unchanged).
- Always regenerates `conf.d/00-pv.ini` (write `; stale content` to it,
  call EnsureIniLayout, expect canonical content back).
- Creates `etc/`, `conf.d/`, `~/.pv/data/sessions/<ver>/`,
  `~/.pv/data/tmp/<ver>/` when missing.
- Uses a small `php.ini-development` fixture in
  `internal/phpenv/testdata/` — the real one is opaque to our logic; we
  only `cp` it.

**Unit (`internal/tools/tool_test.go`)** — extend the existing php-shim
test to assert the generated shim contains:
- An `export PHPRC=...` line referencing `$VERSION`.
- An `export PHP_INI_SCAN_DIR=...` line referencing `$VERSION`.
- The `exec "$BINARY" "$@"` line is unchanged and last.

**Unit (`internal/server/frankenphp_test.go`)** — without spawning
FrankenPHP, build the `*exec.Cmd` produced by the start helper and
assert `cmd.Env` contains both `PHPRC=...etc` and `PHP_INI_SCAN_DIR=...conf.d`
for the right version. Cover both the main (global) and per-version
launchers.

**E2E (`scripts/e2e/`)** — a new phase, called from
`.github/workflows/e2e.yml`:
1. `pv php:install 8.4` (or whichever version is already exercised in
   e2e — reuse to keep matrix size flat).
2. Assert `~/.pv/php/8.4/etc/php.ini` exists and is non-empty.
3. Run `~/.pv/bin/php --ri core | grep "Configuration File"` and check
   the path resolves to `~/.pv/php/8.4/etc/php.ini`.
4. Drop `99-local.ini` containing `memory_limit = 42M` into
   `~/.pv/php/8.4/conf.d/`. Run `~/.pv/bin/php -r 'echo
   ini_get("memory_limit");'` and assert output is `42M`.
5. Hit a linked project's `phpinfo()` endpoint through FrankenPHP and
   assert the same `memory_limit = 42M` value, proving the FrankenPHP
   launcher honors the same env path.

**CI dispatch** — for verifying the build-artifacts.yml change:

```bash
gh workflow run build-artifacts.yml --ref <branch> \
  -f skip_postgres=true -f skip_mysql=true
```

Per CLAUDE.md: scoped to FrankenPHP only since that's the family that
produces the changed artifact.

## Risks and edge cases

- **User has manually edited `~/.pv/php/<ver>/etc/php.ini`.** Preserved
  by the "copy only if missing" rule on install/update. Documented in
  the comment header of `00-pv.ini` so users know which file is theirs.
- **User has manually edited `00-pv.ini`.** Overwritten on next
  install/update. The header comment warns about this and points them
  at `99-local.ini`.
- **Daemon backfill races with a concurrent `php:install`.**
  `EnsureIniLayout` is idempotent and uses `os.MkdirAll` /
  copy-if-absent / overwrite — no destructive ordering. Worst case is
  `00-pv.ini` is written twice with the same content.
- **Old pv binary launching a new layout, or vice versa.** The new
  layout is purely additive on disk. An older pv binary won't set the
  env vars and will fall back to today's behaviour (no ini loaded);
  nothing breaks. A new pv binary against a pre-feature install hits
  the daemon backfill and self-heals.
- **`php.ini-development` missing from the upstream php-src tree
  someday.** The build step's `test -f` fails the build loudly rather
  than silently shipping a broken tarball.
- **Global PHP version unset when daemon starts.** `StartFrankenPHP`
  resolves it from settings; if absent, skip adding the env vars —
  there's no version to point at, and other startup paths already error
  on missing global PHP.
