## Daemon Mode Implementation Plan

### Task 1: Refactor `pv start` Command Flags

Current `pv start` runs in foreground. Add two flags:

- `pv start` → default, foreground (same as today)
- `pv start --background` → installs and starts launchd service
- `pv start --foreground` → explicit foreground flag (launchd uses this internally, users can too)

Both foreground and `--foreground` do the same thing. The distinction exists so the plist can explicitly call `--foreground` and the intent is clear in process listings.

### Task 2: Plist Template

Create an `internal/daemon/plist.go` that generates the launchd plist.

Needs to be dynamic because paths depend on the user:

```go
type PlistConfig struct {
    Label         string   // "dev.prvious.pv"
    PvBinaryPath  string   // ~/.pv/bin/pv
    LogDir        string   // ~/.pv/logs
    HomeDir       string   // ~/.pv
    EnvVars       map[string]string
}
```

Template renders to `~/Library/LaunchAgents/dev.prvious.pv.plist`. The plist calls `pv start --foreground` as its program arguments.

Include these plist keys:

- `Label` → `dev.prvious.pv`
- `ProgramArguments` → `[~/.pv/bin/pv, start, --foreground]`
- `RunAtLoad` → `false` initially (Task 8 adds auto-start)
- `KeepAlive` → `true` (restart on crash)
- `StandardOutPath` → `~/.pv/logs/pv.log`
- `StandardErrorPath` → `~/.pv/logs/pv.err.log`
- `EnvironmentVariables` → `XDG_DATA_HOME`, `PATH` (so pv can find its own binaries)
- `WorkingDirectory` → `~/.pv`

### Task 3: Launchctl Wrapper

Create `internal/daemon/launchctl.go` with helpers that shell out to `launchctl`:

- **`Install()`** → write plist to `~/Library/LaunchAgents/`
- **`Uninstall()`** → remove plist file
- **`Load()`** → `launchctl load <plist path>`
- **`Unload()`** → `launchctl unload <plist path>`
- **`Restart()`** → `launchctl kickstart -k gui/<uid>/dev.prvious.pv`
- **`IsLoaded()`** → `launchctl list dev.prvious.pv`, check exit code
- **`GetPID()`** → parse `launchctl list dev.prvious.pv` output for PID

Get the UID via `os.Getuid()` for the `gui/<uid>/` domain target.

Error handling matters here — every launchctl call can fail, and the error messages are often cryptic. Wrap them with human-readable errors like "pv server is not running" instead of "Could not find service dev.prvious.pv in domain for port".

### Task 4: Log Management

When running as daemon, stdout/stderr go to files instead of the terminal. Set up:

- `~/.pv/logs/pv.log` — stdout from the pv process
- `~/.pv/logs/pv.err.log` — stderr
- `~/.pv/logs/caddy.log` — FrankenPHP/Caddy access log (already exists from your Caddyfile config)

Add `pv log` command that tails the logs:

- `pv log` → `tail -f ~/.pv/logs/pv.log`
- `pv log --error` → `tail -f ~/.pv/logs/pv.err.log`
- `pv log --access` → `tail -f ~/.pv/logs/caddy.log`
- `pv log --all` → tails all three interleaved

This is essential — without it, daemon mode is a black box when something goes wrong.

### Task 5: Wire Up `pv start --background`

The actual flow when user runs `pv start --background`:

1. Check if already running (`IsLoaded()` + `GetPID()`)
2. If running → print "pv is already running (PID 12345)" and exit
3. Generate plist with current paths and config
4. Write plist to `~/Library/LaunchAgents/dev.prvious.pv.plist`
5. Run `launchctl load <plist>`
6. Wait up to 3 seconds, polling `GetPID()` until process appears
7. Run a quick health check (HTTP request to `https://localhost:8443` or DNS query to the embedded server)
8. Print "pv is running in the background (PID 12345)"
9. Print "Run `pv log` to view logs"

### Task 6: Wire Up `pv stop`

Needs to handle both foreground and daemon modes:

1. Check if launchd service is loaded (`IsLoaded()`)
2. If loaded → `launchctl unload <plist>` (this sends SIGTERM to the process)
3. If not loaded → check for PID file from foreground mode, send SIGTERM
4. Wait for process to exit (poll PID, timeout after 5 seconds)
5. If still alive after timeout → SIGKILL
6. Clean up PID files
7. Print "pv stopped"

Don't remove the plist file on stop — just unload it. This way `pv start --background` can just `load` again without regenerating.

### Task 7: Wire Up `pv restart`

Two paths:

- **Daemon mode** (launchd is loaded) → `launchctl kickstart -k gui/<uid>/dev.prvious.pv` which tells launchd to kill and restart the process in one atomic operation
- **Foreground mode** → not really restartable from another terminal, so just print "pv is running in foreground, Ctrl+C and start again" or send SIGUSR1 to trigger a graceful internal restart

For Caddyfile-only changes (like after `pv link`), you don't need a full restart — just reload FrankenPHP via its admin API or signal. Only binary-level changes (like `pv use php:8.3`) need a full restart.

### Task 8: Auto-Start on Login

Add `pv daemon install` and `pv daemon uninstall`:

- **`pv daemon install`** → write plist with `RunAtLoad: true`, load it. pv starts on every login automatically.
- **`pv daemon uninstall`** → unload, remove plist. No more auto-start.

This is separate from `pv start --background` because auto-start is a one-time preference. The user explicitly opts into "I want pv always running".

Could also be a flag: `pv daemon install --start-on-login` vs without.

### Task 9: `pv status` Enhancement

Update to show daemon state:

```
pv server: running (PID 12345, daemon mode)
  Uptime: 3 days, 2 hours
  PHP 8.4 (main) on :8443
  PHP 8.2 on :8420 → app-one

Projects:
  app-one.test     laravel-octane  PHP 8.2
  app-two.test     laravel         PHP 8.4
```

Pull PID and uptime from launchctl. Pull project info from registry. If not running, show that too with a hint to run `pv start`.

### Task 10: Plist Regeneration

The plist needs to be regenerated when certain things change:

- `pv use php:[version]` → main binary path changes
- pv binary itself gets updated
- Environment variables change

Add an `internal/daemon/sync.go` that compares current plist on disk vs what would be generated. If they differ, rewrite and reload. Call this from `pv use`, `pv link`, and anywhere else that could affect the plist.

### Task 11: Unit Tests (Go)

Add to `internal/daemon/` — runs on any OS, no launchd needed.

**`internal/daemon/plist_test.go`**:

- **Plist XML correctness** — render template with a `PlistConfig`, assert the XML contains:
    - Correct `Label` (`dev.prvious.pv`)
    - `ProgramArguments` array with the binary path + `start` + `--foreground`
    - `KeepAlive` set to `true`
    - `RunAtLoad` set to `false` (default) and `true` (when auto-start enabled)
    - `StandardOutPath` / `StandardErrorPath` pointing to `~/.pv/logs/`
    - `EnvironmentVariables` containing `PATH` and `XDG_DATA_HOME`
    - `WorkingDirectory` set to `~/.pv`
- **Dynamic paths** — assert rendered paths use the actual `HOME` dir, not hardcoded values
- **Env vars** — pass custom env vars in `PlistConfig.EnvVars`, assert they appear in output

**`internal/daemon/sync_test.go`**:

- **Plist diff detection** — generate a plist, write to temp dir, change a config value (e.g. PHP version), assert `NeedsSync()` returns true
- **No-op when identical** — generate twice with same config, assert `NeedsSync()` returns false

Use `t.Setenv("HOME", t.TempDir())` for isolation, same as existing tests.

### Task 12: E2E Tests — Daemon Mode (Bash Scripts)

Three focused scripts in `scripts/e2e/`. All follow existing conventions: `set -euo pipefail`, `source helpers.sh`, assertions via `assert_contains` / `assert_fails`.

The existing foreground e2e tests (`start-curl.sh`, `restart.sh`, `log.sh`, `stop.sh`) already cover DNS, HTTP, restart, and log behavior — those don't need daemon-specific duplicates. These three scripts test what's unique to daemon mode: launchd integration, crash recovery, and the full daemon stack.

**`scripts/e2e/daemon-start-stop.sh`** — Start and stop via launchd, verify the full lifecycle:

```bash
# Start in background mode
pv start --background
sleep 5

# Verify launchd has the service loaded
launchctl list dev.prvious.pv
echo "OK: launchd service loaded"

# Verify pv status reports daemon mode
STATUS=$(pv status)
echo "$STATUS"
assert_contains "$STATUS" "running" "server not running in daemon mode"
assert_contains "$STATUS" "daemon" "status doesn't show daemon mode"

# Verify plist was written
ls ~/Library/LaunchAgents/dev.prvious.pv.plist
echo "OK: plist file exists"

# Verify log files are being written
sleep 2
ls ~/.pv/logs/pv.log
ls ~/.pv/logs/pv.err.log
echo "OK: daemon log files exist"

# Stop daemon
pv stop
sleep 3

# Verify launchd service is unloaded
if launchctl list dev.prvious.pv 2>/dev/null; then
  echo "FAIL: launchd service still loaded after stop"
  exit 1
fi
echo "OK: launchd service unloaded"

# Verify pv status reports stopped
STATUS=$(pv status)
echo "$STATUS"
assert_contains "$STATUS" "stopped" "server not stopped"

# Verify plist file is preserved (not deleted — just unloaded)
ls ~/Library/LaunchAgents/dev.prvious.pv.plist
echo "OK: plist file preserved after stop"

# Stop when not running — should not error
OUTPUT=$(pv stop 2>&1)
echo "$OUTPUT"
echo "OK: stop when not running is safe"
```

**`scripts/e2e/daemon-crash-recovery.sh`** — Kill the process, verify launchd restarts it (the definitive daemon test):

```bash
# Start in daemon mode
pv start --background
sleep 5

# Get current PID
OLD_PID=$(launchctl list dev.prvious.pv | awk 'NR==1{print $1}')
echo "Current PID: $OLD_PID"

# Kill it rudely
kill -9 "$OLD_PID"

# Wait for launchd to restart (KeepAlive: true)
sleep 8

# Confirm new PID exists and is different
NEW_PID=$(launchctl list dev.prvious.pv | awk 'NR==1{print $1}')
echo "New PID: $NEW_PID"
[ "$NEW_PID" != "$OLD_PID" ] || { echo "FAIL: PID did not change after kill"; exit 1; }
[ -n "$NEW_PID" ] || { echo "FAIL: no PID after crash recovery"; exit 1; }
echo "OK: launchd restarted process ($OLD_PID → $NEW_PID)"

# Verify it's actually healthy after recovery
setup_curl
sleep 3
curl_site "e2e-php.test" "php works"
echo "OK: site works after crash recovery"

# Clean stop
pv stop
sleep 3
```

**`scripts/e2e/daemon-full-stack.sh`** — Link a project, start daemon, curl it, tear down:

```bash
# Create a test project
mkdir -p /tmp/e2e-daemon/public
cat > /tmp/e2e-daemon/composer.json << 'EOF'
{"require":{"php":"^8.4"}}
EOF
cat > /tmp/e2e-daemon/public/index.php << 'PHPEOF'
<?php
ignore_user_abort(true);
$handler = static function () { echo "daemon works"; };
for (;;) {
    if (!\frankenphp_handle_request($handler)) break;
}
PHPEOF

# Link and start in daemon mode
pv link /tmp/e2e-daemon
pv start --background
sleep 5

# Setup curl with the new domain
CACERT="${HOME}/.pv/caddy/pki/authorities/local/root.crt"
RESOLVE="--resolve e2e-daemon.test:443:127.0.0.1"
export CACERT RESOLVE

# Verify the site works through the daemon
curl_site "e2e-daemon.test" "daemon works"
echo "OK: full stack works with daemon mode"

# Clean up
pv stop
sleep 3
pv unlink e2e-daemon
rm -rf /tmp/e2e-daemon
```

### Task 13: Update CI Workflow & Diagnostics

Update `.github/workflows/e2e.yml` to include daemon test phases. These run **after** the existing foreground tests complete (after "Stop server"), as a separate daemon test block before "PHP Version Lifecycle":

```yaml
# ── Daemon Mode Tests ─────────────────────────────────────────
- name: Daemon — start and stop lifecycle
  timeout-minutes: 2
  run: scripts/e2e/daemon-start-stop.sh

- name: Daemon — crash recovery
  timeout-minutes: 2
  run: scripts/e2e/daemon-crash-recovery.sh

- name: Daemon — full stack
  timeout-minutes: 2
  run: scripts/e2e/daemon-full-stack.sh
```

Update `scripts/e2e/helpers.sh` to add the `e2e-daemon.test` domain to the `RESOLVE` variable.

Update `scripts/e2e/diagnostics.sh` to also dump:

- `~/.pv/logs/pv.log` (daemon stdout)
- `~/.pv/logs/pv.err.log` (daemon stderr)
- `~/Library/LaunchAgents/dev.prvious.pv.plist` contents
- `launchctl list dev.prvious.pv` output

Update the CI cleanup step:

```yaml
- name: Cleanup
  if: always()
  run: |
      launchctl unload ~/Library/LaunchAgents/dev.prvious.pv.plist 2>/dev/null || true
      rm -f ~/Library/LaunchAgents/dev.prvious.pv.plist
      sudo -E pv stop 2>/dev/null || true
```

---

### Test Coverage Summary

| What                                    | Where        | Script / File                          |
| --------------------------------------- | ------------ | -------------------------------------- |
| Plist XML correctness                   | Go unit test | `internal/daemon/plist_test.go`        |
| Plist sync/diff detection               | Go unit test | `internal/daemon/sync_test.go`         |
| Daemon start + stop (launchd lifecycle) | E2E bash     | `scripts/e2e/daemon-start-stop.sh`     |
| Crash recovery (KeepAlive)              | E2E bash     | `scripts/e2e/daemon-crash-recovery.sh` |
| Full stack (link → daemon → curl)       | E2E bash     | `scripts/e2e/daemon-full-stack.sh`     |
| DNS + HTTP serving                      | E2E bash     | Covered by existing `start-curl.sh`    |
| Restart behavior                        | E2E bash     | Covered by existing `restart.sh`       |
| Log output                              | E2E bash     | Covered by existing `log.sh`           |
| Auto-start on login (RunAtLoad)         | Manual only  | Not testable in CI                     |

---

**Order:** 2 → 3 → 11 → 4 → 1 → 5 → 6 → 7 → 9 → 8 → 10 → 12 → 13

Start with plist template and launchctl wrapper, then immediately write unit tests for them (Task 11) to validate before wiring up commands. Logs next for debugging. Wire up commands (Tasks 1, 5–7, 9, 8, 10). Then write e2e tests (Task 12) once the commands work. CI integration (Task 13) comes last.
