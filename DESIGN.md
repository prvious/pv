# PV

PV has laravel style commands where commmands under the same category/famility are prefixed with the namespace. eg: 'pv php:update'

`pv` is a Laravel-first local desired-state control plane (More framework support to come later when laravel is stable).

`pv` gets you a complete local PHP environment in one shot:

- FrankenPHP — the server (Caddy + embedded PHP, no Apache/Nginx)
- PHP — managed per-version, no homebrew/apt needed
- Mysql, Postgresql, Redis, Composer, Mailpit, Rustfs all Ready to go

Per-project versions are supported too — add a pv.yml file with php: "8.4" in your project root. Multiple PHP versions run simultaneously, each project served by its own FrankenPHP process.

## High-Level Features

- Control-plane foundation.
- Machine-owned store(Sqlite), filesystem layout, and migration guardrails.
- Daemon and resource-agnostic supervisor.
- Managed resources: Mailpit, Postgres, MySQL, Redis, RustFS, and more to come...
- Gateway .test HTTPS routing and `pv open`.
- Status UX across desired and observed state.

## Platform Scope

PV v1 is macOS-only. Linux and Windows support are deferred and not guaranteed.

PV v1 targets macOS 13 and newer.

The design may use macOS-specific primitives where they materially improve the v1 experience, including launch agents for daemon startup and the macOS System keychain for local CA trust.

## Implementation Language

PV's CLI and daemon are implemented in Rust. Rust owns the local control plane, desired/observed state access, daemon socket protocol, internal DNS resolver, reconciliation, process supervision, config generation, and command UX.

PV ships as one Rust binary. The same `pv` executable handles user-facing CLI commands and daemon mode for the LaunchAgent.

The LaunchAgent runs the same binary through a hidden internal entrypoint, `pv daemon:run`. Public daemon lifecycle commands remain `pv daemon:enable`, `pv daemon:disable`, and `pv daemon:restart`. `pv daemon:run` is hidden from normal help output.

Hidden internal commands may still appear in generated shell completions when `clap` exposes hidden subcommands there. PV should prefer keeping command routing simple and centralized over adding custom completion filtering solely to hide internal commands from completions.

PV uses Tokio as its Rust async runtime for the daemon, Unix socket server, internal DNS resolver, child-process supervision, concurrent downloads, timers, and file watching.

PV uses `clap` for Rust CLI parsing, including nested command namespaces, aliases, validation, help output, and shell completions.

PV supports generated shell completions in v1 through `pv completions <shell>`. Supported completion shells match `pv env`: `zsh`, `bash`, and `fish`. PV does not auto-install completions in v1.

`pv completions <shell>` rejects unsupported shell names with a clear error.

PV's public CLI uses literal colon command names, such as `pv php:install`, matching Laravel-style command namespaces. Internal `clap` modeling can use whatever structure is simplest as long as the public command shape remains colon-based.

PV v1 does not support space-separated command aliases such as `pv php install`.

PV respects the `NO_COLOR` environment variable and supports a global `--no-color` flag for deterministic plain output across commands.

PV uses `rusqlite` for SQLite access to `pv.db`. Database work is local and transactional, so synchronous queries are acceptable; daemon paths can use `spawn_blocking` when needed to avoid blocking Tokio runtime tasks.

PV embeds SQLite migrations into the Rust binary and runs them automatically before accessing `pv.db`. Migrations run transactionally and are tracked in a machine-owned migrations table.

PV database migrations must remain backward-compatible with the immediately previous PV application version so self-update rollback can safely restore the previous binary without restoring `pv.db`. Destructive or incompatible schema cleanup should be delayed until a later release after the previous binary no longer needs to read the old shape.

Before applying migrations, PV creates a timestamped `pv.db` backup such as `~/.pv/pv.db.20260522-143012.bak`. PV keeps migration backup retention simple, such as the last 5 backups.

If a `pv.db` migration fails, PV refuses to run commands that depend on `pv.db`, reports a clear migration error, and points to logs. PV does not continue against partially migrated state.

PV does not automatically roll back from migration backups. Transactional migrations should leave the database unchanged on failure; backups exist for manual recovery and diagnostics.

Managed Resources remain external binaries/artifacts managed by PV rather than Rust code embedded into the PV binary.

Initial PV distribution is a standalone install script/direct binary download. Homebrew support can be added after the release flow stabilizes. A signed `.pkg` is deferred unless macOS trust/onboarding requires it.

The install script verifies the downloaded PV application binary against a published SHA-256 checksum before installing it. If checksum verification fails, installation deletes the bad download and stops before installing files, editing shell profiles, or running setup.

The stable installer URL serves a generated installer script based on PV app release metadata. The bash installer does not need to parse the JSON PV app update manifest. The installer script may embed or otherwise receive the resolved current PV version, platform asset URLs, and SHA-256 checksums from the server-side installer generation flow. The JSON PV app update manifest is used by the Rust self-updater.

The generated installer script installs the current stable PV release only in v1. Installing a specific historical PV version through the installer is out of v1 scope.

PV v1 has one stable installer/update channel for both the generated installer and `pv update`. Installer channel query parameters such as `?channel=preview`, nightly channels, and multi-channel update selection are out of v1 scope.

By default, the install script installs the PV binary under `~/.pv/bin/releases/<version>/pv`, creates or updates the active `~/.pv/bin/pv` symlink, and then runs `pv setup` automatically. Before editing shell profiles, admin prompts, or large Managed Resource downloads, interactive installs ask for confirmation. Installer flags use the same prompt semantics as `pv setup`: `--yes` skips PV's own confirmations for automation, but does not bypass macOS authentication; `sudo` may still prompt for admin credentials. `--non-interactive` implies PV confirmations are accepted, disables all prompts, and fails if input, sudo authentication, or shell profile confirmation is required. A `--no-setup` flag installs only the PV binary without running setup. A `--no-path` flag skips automatic shell profile edits.

The install script may create PV shell integration in the user's shell profile automatically with a clearly delimited PV-managed shell block. `pv setup` may repair that same block later. Shell profile edits must be idempotent and backed up first. If shell detection fails, the installer prints manual shell integration instructions instead of editing profile files.

When automatic setup is enabled, the installer creates or repairs the `PV ENV` shell profile block before running `pv setup`. The installer still invokes setup through the absolute `~/.pv/bin/pv` path in the current process.

If the installer cannot edit the shell profile, it warns, prints manual shell integration instructions, and continues automatic setup unless strict non-interactive behavior requires failing instead.

If shell detection finds an unsupported shell, the installer skips shell profile edits, prints manual shell integration instructions, and continues setup. Unsupported shell integration does not block DNS, ports, CA trust, daemon registration, or Managed Resource installation.

The installer detects the user's shell from `$SHELL`. If `$SHELL` is missing or unsupported, the installer skips profile edits and prints manual shell integration instructions.

Shell profile backups created by PV append a timestamp and `.pv.bak`, such as `~/.zprofile.20260522-143012.pv.bak`.

The installer edits only the detected shell's profile file for PATH setup: `~/.zprofile` for zsh, `~/.bash_profile` for bash, and `~/.config/fish/config.fish` for fish. It does not edit multiple shell profile files at once.

If the detected shell profile file does not exist, the installer may create it with only the `PV ENV` block after confirmation. No backup is needed when creating a new file, but the action is reported.

The installer uses `PV ENV` delimiters for shell profile edits. The installer-managed block loads `pv env` so PV shims and Composer work in new shells. It should call the PV binary by absolute path and pass an explicit `--shell <shell>` for the detected profile so shell startup works even before `~/.pv/bin` has been added to PATH and does not rely on runtime shell detection. For POSIX-style shells, the block is:

```sh
# >>> PV ENV
if [ -x "$HOME/.pv/bin/pv" ]; then
  eval "$("$HOME/.pv/bin/pv" env --shell zsh)"
fi
# <<< PV ENV
```

Fish uses equivalent syntax with the same `PV ENV` delimiter labels.

`pv setup` may repair a stale installer-managed `PV ENV` shell profile block, but only after confirmation because shell profiles are user-owned. `pv setup --yes` consents to this repair without prompting. `pv setup --non-interactive` fails instead of prompting for confirmation, shell profile repair, or sudo authentication. `pv setup --no-path` disables shell profile edits, including stale `PV ENV` block repair, but still prints manual shell integration instructions. When repairing the `PV ENV` block, PV replaces the block wholesale and does not preserve user edits inside it.

The installer does not try to source the updated shell profile into the current parent shell. After editing, it tells the user to open a new terminal or run the shown `pv env` command for the current session.

If binary installation succeeds but automatic setup fails, the install script keeps the binary installed, reports the setup failure clearly, and tells the user to rerun `pv setup` after fixing the issue.

## Hostname Resolution

PV v1 uses an internal lightweight DNS resolver for `.test` hostname resolution instead of managing `dnsmasq`, CoreDNS, or per-Project `/etc/hosts` entries.

The DNS resolver runs inside the PV daemon process as a dedicated internal task/thread, not as a separate child process or Managed Resource.

macOS is configured once to send `.test` lookups to PV's resolver. The resolver is managed by the PV daemon/supervisor and is part of whole-system PV status.

PV configures macOS with `/etc/resolver/test`, pointing `.test` lookups at PV's internal resolver. Creating or removing this file requires admin privileges, but the resolver itself listens on high loopback ports so the PV daemon does not need to run as root. PV prefers DNS port `35353`. If that port is occupied by another process, PV chooses an available high port, stores it in `pv.db`, and writes that port into `/etc/resolver/test`.

If `/etc/resolver/test` already exists and is not PV-owned, `pv dns:install` and `pv setup` fail safely, report the resolver conflict, and print manual repair instructions instead of overwriting it.

PV marks `/etc/resolver/test` with a clear ownership comment such as `# Managed by PV`. PV only repairs or removes the resolver file when the ownership marker is present.

The internal DNS resolver supports both UDP and TCP DNS on the chosen DNS port.

The resolver answers all `.test` hostnames with IPv4 and IPv6 loopback records: `127.0.0.1` and `::1`. The Gateway decides whether a hostname maps to a linked Project.

Gateway/DNS dual-loopback support is the design intent. IPv4 loopback is the hard v1 requirement; if macOS `pf` IPv6 redirect handling is problematic, PV may degrade to IPv4-only with a clear status warning.

For `.test` queries, the resolver answers A and AAAA records only. Other record types return NODATA/NOERROR. The resolver does not proxy DNS queries upstream.

DNS responses use a low TTL of 5 seconds.

## Low-Port Routing

PV uses macOS `pf` redirect rules with a PV-owned anchor to preserve normal `http://` and `https://` URLs without running the daemon or Gateway as root.

PV manages only its own `pf` anchor, such as `com.prvious.pv`, and the minimal anchor reference required to load it. PV does not rewrite the global `pf` config wholesale and must preserve non-PV `pf` rules.

PV marks its `pf` anchor file and any anchor reference it adds with clear ownership comments such as `# Managed by PV`. PV only repairs or removes `pf` lines/files it can identify as PV-owned.

If `/etc/pf.conf` has been customized and PV cannot safely add or remove only its anchor reference, PV fails safely and prints manual instructions instead of attempting a best-effort global edit.

The Gateway listens as the user on high loopback ports only. PV prefers uncommon defaults `48080` for HTTP and `48443` for HTTPS. If preferred ports are free or already owned by PV, PV uses them. If either preferred port is occupied by another process, PV chooses available high ports and stores the chosen ports in `pv.db`. `pv setup` installs `pf` rules that redirect loopback traffic from ports `80` and `443` to the stored Gateway ports. `pv status` checks that the rules are loaded and reports the chosen ports. `pv uninstall` removes the rules.

PV v1 does not expose Projects on the LAN or through tunnels. LAN access or tunnel integrations such as Cloudflare Tunnels may be considered later.

If another process is already listening on loopback port `80` or `443`, `pv setup` and `pv ports:install` fail with a clear conflict instead of silently taking over traffic. When detectable, PV reports the process that owns the port.

A friendly browser page for unknown Project hostnames is useful, but is post-v1 polish rather than required v1 scope.

## Setup

`pv setup` performs PV's required macOS bootstrap steps:

- Create `/etc/resolver/test` so macOS sends `.test` lookups to PV's internal DNS resolver.
- Install macOS `pf` redirect rules so loopback ports `80` and `443` reach the unprivileged Gateway.
- Trust PV's local CA in the macOS System keychain.
- Register the PV daemon as a per-user `launchd` LaunchAgent with `KeepAlive` so macOS starts it after login and can restart it after crashes.
- Start the PV daemon immediately after registration.
- Record desired state for the default Managed Resource versions, then request daemon reconciliation.

The default setup install set includes the manifest default tracks for FrankenPHP/PHP, MySQL, PostgreSQL, Redis, Mailpit, and RustFS, plus Composer track `2`. Downloads should run in parallel where possible. `pv setup` does not install every track listed in the manifest.

For PHP tracks, setup/install installs both standalone PHP artifacts for CLI/PATH shims and FrankenPHP artifacts for Gateway/workers.

`pv php:install <track>` installs both standalone PHP and FrankenPHP artifacts for that PHP track.

Default Managed Resources installed by `pv setup` are not started until a linked Project needs them. The Gateway and DNS resolver are core PV infrastructure and run even when no Project needs a backing Managed Resource.

Default tool/resource installation is owned by the daemon. `pv setup` records desired install state, starts the daemon, requests reconciliation, and waits for that reconciliation job to finish.

One-off CLI commands communicate with the daemon through a Unix domain socket at `~/.pv/run/pv.sock` using newline-delimited JSON messages. Each request is one JSON line and includes a daemon protocol version field. The immediate response is one JSON line. For long-running work, the daemon then emits NDJSON progress events over the same connection. Event types include `job_started`, `progress`, `log`, `job_completed`, and `job_failed`. `pv setup` listens to the progress stream, renders progress, and exits when the reconciliation job completes or fails.

If the CLI and daemon protocol versions are incompatible, commands print a clear repair command such as `pv daemon:restart` rather than automatically restarting the daemon. `pv update` may handle daemon restart explicitly as part of the update flow.

After completing system setup, `pv setup` reconciles already-linked Projects so Projects linked before setup can become reachable.

Default tool/resource installation allows partial success. The daemon installs what it can, records and reports failures, causes `pv setup` to exit non-zero if any default install failed, and keeps setup safe to rerun to repair missing pieces.

Setup may require an admin prompt for system-owned configuration, but the PV daemon runs as the logged-in user and should not need to run as root.

`pv setup` is idempotent and repair-oriented. It verifies resolver configuration, CA trust, and LaunchAgent registration, fixes anything missing or stale, and prints what changed.

`pv setup` creates the required base directory structure under `~/.pv`, including `bin/`, `run/`, `logs/`, `downloads/`, `config/`, `certificates/`, `composer/`, and `resources/`, with correct permissions before starting the daemon or installing Managed Resources.

`pv setup` may edit the user's shell profile only for the PV-managed `PV ENV` shell integration block, using the same confirmation and `--yes` / `--non-interactive` / `--no-path` behavior as the installer. It does not silently modify shell profiles.

After successful setup, PV prints concise shell integration next steps for `pv env`, using the detected shell where possible, plus optional shell completion generation instructions such as `pv completions zsh`. PV does not auto-install shell completions.

`pv setup` fails fast when required system integration steps fail, such as resolver configuration, CA trust, or LaunchAgent registration. Completed prior steps remain in place, and rerunning setup repairs drift or missing pieces.

## Daemon Lifecycle

PV registers its daemon as a per-user macOS LaunchAgent with `KeepAlive`.

PV uses a predictable LaunchAgent label, such as `com.prvious.pv.daemon`, and generated LaunchAgent metadata so `pv daemon:*`, `pv setup`, and `pv uninstall` only manage PV-owned daemon registration.

PV installs the LaunchAgent plist at `~/Library/LaunchAgents/com.prvious.pv.daemon.plist`.

The LaunchAgent plist sets `StandardOutPath` and `StandardErrorPath` to PV-owned files under `~/.pv/logs/`, such as `launchd.out.log` and `launchd.err.log`, so daemon startup failures are diagnosable before structured daemon logging starts.

If a LaunchAgent with PV's expected label already exists but is not PV-owned, `pv daemon:enable` and `pv setup` fail safely and report the conflict instead of overwriting it.

The daemon runs as the logged-in user, owns reconciliation, and manages PV child processes. macOS starts it after login and restarts it after crashes.

The daemon restarts crashed desired child processes through reconciliation. Commands and file watchers request reconciliation, and the daemon also runs a lightweight periodic health tick every 30 seconds to detect drift. The tick enqueues targeted reconciliation when it finds something wrong rather than running full system reconciliation every time.

If a PV-managed child process crashes repeatedly, the daemon applies restart backoff instead of restarting it forever in a tight loop. If the process keeps crashing, PV marks the affected runtime as failed or degraded in observed state. The periodic health tick may retry later, and `pv restart` gives users an explicit manual recovery path.

Crash restart backoff retries a desired child process up to 3 times with increasing delays, such as 1 second, 5 seconds, and 15 seconds. After that, PV marks the process failed or degraded until a later health tick or explicit `pv restart` retries it.

PV resets a child process crash counter after the process stays healthy for 60 seconds.

Crash-loop failures are scoped to the affected runtime. A failed Managed Resource track degrades only Projects that need that resource track. A failed PHP-track worker affects only Projects on that PHP track. Gateway failure is system-wide because all Project routing depends on it.

Backing Managed Resource failures do not remove Project routes. PV keeps serving the web app in a degraded state when the Gateway and PHP-track worker are healthy.

PV writes pid files under `~/.pv/run/` for the Gateway, Project-serving workers, and Managed Resource tracks. After daemon restart, PV may use these pid files to discover existing PV-owned child processes, but it must verify ownership before acting by checking the process command/path matches the expected PV-managed binary and config. PV never kills a process based on PID alone.

PV writes a small JSON runtime metadata file next to each pid file with the expected binary path, config path, resource name, track, and start time. Runtime metadata supports ownership verification and diagnostics, while `pv.db` remains the source of truth.

PV writes pid and runtime metadata files atomically by writing temporary files and renaming them into place after the child process starts successfully.

PV uses resource-specific readiness checks after starting child processes instead of treating a running PID as ready. Examples: the Gateway responds on its high HTTP/HTTPS ports, the internal DNS resolver answers a `.test` query, MySQL/Postgres accept connections, Redis responds to ping, and Mailpit/RustFS respond on their HTTP ports.

PV uses a default 15-second readiness timeout per child process, with resource-specific overrides only if a Managed Resource consistently needs longer. If readiness fails, PV marks that runtime failed or degraded and includes the readiness failure in observed state and logs.

If a pid file points to no process, or to a process that fails PV ownership verification, PV ignores it for control purposes and removes the stale pid file.

After daemon restart, PV adopts already-running PV-owned child processes when ownership verification passes and their binary/config/version match desired state. Reconciliation restarts processes that are stale, mismatched, or no longer desired.

PV starts each supervised runtime in its own process group so it can stop the entire PV-owned process tree safely. PV only signals a process group after ownership verification.

PV stops child process groups with graceful termination first, waits up to 10 seconds, then force-kills only PV-owned process groups that do not exit within that timeout.

The health tick may check privileged integrations such as `/etc/resolver/test` and `pf` rules read-only. It records repair-required status but does not prompt or mutate privileged system config. The health tick does not refresh the remote artifact manifest or make routine background network calls.

The daemon watches linked Project config files and automatically reconciles when they change.

On startup, the daemon detects privileged system drift such as stale DNS resolver ports or stale `pf` redirects, but it does not prompt for admin privileges or mutate system configuration from the background. It records repair-required observed status instead.

If a linked Project config becomes invalid, the daemon keeps serving the last valid desired state, records the config error in observed status, stops updating `.env` from the invalid config, and surfaces the error in `pv list` and `pv status`. PV does not tear down working resources because of a transient invalid edit.

Project config changes restart or reload only affected runtime processes. PHP version or routing changes may restart/reassign FrankenPHP serving for the affected Project. Env-only changes update `.env` without restarting the Gateway.

When a Project's PHP track changes, PV reconfigures only affected Project-serving workers. It may stop an old PHP worker if no Projects remain on that track and start or reload the new track's worker. Unrelated PHP workers are not touched.

`pv setup` is the friendly first-time bootstrap path and includes daemon registration. `pv daemon:*` commands remain available as lower-level lifecycle and troubleshooting commands.

`pv daemon:enable` registers the LaunchAgent, starts the daemon immediately, waits up to 15 seconds for the Unix socket and daemon health, enqueues reconciliation, and exits non-zero if the daemon does not become healthy. It does not wait for the triggered reconciliation to finish.

`pv daemon:disable` gracefully stops PV-managed child processes, waits up to 10 seconds, force-kills remaining PV-owned child processes when needed, reports what happened, stops the running daemon, and disables/unregisters the LaunchAgent so it does not start on next login.

`pv daemon:disable` does not remove DNS resolver config, `pf` rules, or CA trust. Those integrations are managed by `pv dns:*`, `pv ports:*`, `pv ca:*`, `pv setup`, and `pv uninstall`.

`pv ca:*` commands remain available as lower-level trust inspection and repair commands even though `pv setup` handles first-time CA trust.

`pv ca:trust` generates PV's local CA if it is missing, then trusts it in the macOS System keychain.

`pv ca:untrust` removes trust from the macOS System keychain but does not delete local CA files. Full CA file deletion belongs to `pv uninstall --prune`.

PV's local CA files are user-specific and live under `~/.pv/certificates/`. Trust is installed into the macOS System keychain so browsers trust Project certificates.

`pv dns:*` commands remain available as lower-level resolver inspection and repair commands even though `pv setup` handles first-time resolver configuration.

`pv dns:install` installs or repairs `/etc/resolver/test` and ensures the PV daemon is running so the resolver can answer `.test` lookups. If the LaunchAgent is not registered, it tells the user to run `pv setup` or `pv daemon:enable`.

`pv ports:*` commands remain available as lower-level `pf` redirect inspection and repair commands even though `pv setup` handles first-time low-port routing.

`pv ports:install` enables `pf` if needed and installs only PV-owned anchor/rules. `pv ports:uninstall` removes PV-owned rules but does not disable `pf` globally because other software may rely on it.

Privileged self-healing only happens from foreground commands such as `pv setup`, `pv dns:install`, and `pv ports:install`, where an admin prompt is expected. PV mutates stored port choices in `pv.db` only after the corresponding privileged system configuration repair succeeds.

Foreground commands use `sudo` for v1 privileged operations. PV should run the smallest possible privileged commands and print clear explanations before prompting.

PV generates privileged config files under `~/.pv` first, validates them where possible, then installs them with a minimal privileged copy/write step. If validation fails, PV does not touch system files. For `pf` rules, PV should use a reliable `pfctl` parse/dry-run mode when available; if that is unavailable or unreliable, PV validates expected file shape and fails safely before touching active rules.

# More Info

## How pv exposes it's managed binaries to the user's environment:

For `pv env`, the user would add this line `eval "$(pv env)"` in their bashrc or zshrc file and that env command prints something like this:

```bash
export PATH="/Users/<user>/.pv/bin":"/Users/<user>/.pv/composer/vendor/bin":"$PATH";
export COMPOSER_HOME="/Users/<user></user>/.pv/composer";
export COMPOSER_CACHE_DIR="/Users/<user>/.pv/composer/cache";
```

`pv env` supports zsh, bash, and fish for macOS v1. zsh and bash use POSIX-style shell output. It detects the current shell when possible and also accepts an explicit `--shell <shell>` override.

`pv env --shell <shell>` rejects unsupported shell names with a clear error instead of falling back silently.

`pv env` prints only global shell integration values such as PATH and Composer environment variables. It does not print Project-specific ports, credentials, or Resource allocation values.

`pv env` is safe to run in shell startup files. It is fast, local, does not require the daemon, and works even when setup is incomplete.

`pv env` output is idempotent: it adds `~/.pv/bin` and Composer global bin paths only when they are not already present in PATH, so repeated shell startup does not duplicate entries.

`pv env` prepends PV paths to PATH so PV-managed shims win when the user has opted into PV shell integration.

`pv env` includes `~/.pv/composer/vendor/bin` even if Composer is not installed yet. Missing PATH directories are harmless and keep shell integration stable before and after setup.

`pv env` only prints shell code. It does not create directories or otherwise mutate filesystem state during shell startup.

`pv project:env` prints the generated Project environment values PV would render into the PV-managed `.env` block, without editing `.env`. With no argument, it resolves the current directory's Project. With a hostname argument, it resolves that Project hostname, including additional hostnames declared in `hostnames:`. It prints actual rendered values, including secrets. Broad status commands should avoid printing secrets.

## Multi-version PHP

The Gateway is a Managed Resource role implemented by a PV-managed FrankenPHP/Caddy process, not an HTTP server implemented inside the PV daemon. The PV daemon provisions a Gateway that listens on high loopback ports; macOS `pf` redirects external loopback ports `80` and `443` to the Gateway.
Projects using a different PHP version are proxied to secondary FrankenPHP processes running on high ports.

The Gateway is always-on core PV infrastructure after setup. It only routes/proxies and does not serve Projects directly. Version-specific Project-serving FrankenPHP processes run only when at least one linked Project needs that PHP version.

Each Project-serving FrankenPHP worker serves all Projects assigned to one PHP track. PV does not run one worker per Project.

Project-serving FrankenPHP workers bind only to loopback high ports. They are internal to PV behind the Gateway.

The Gateway terminates TLS for Project hostnames. Project-serving FrankenPHP workers receive proxied plain HTTP traffic over loopback high ports.

The Gateway supports HTTP and HTTPS for Project hostnames. HTTP requests redirect to HTTPS by default.

When proxying to Project-serving workers, the Gateway preserves the original `Host` header and sets forwarding headers such as `X-Forwarded-Host`, `X-Forwarded-Proto`, and `X-Forwarded-For`.

PV generates a Gateway root config that imports per-Project generated config files. Splitting Project config keeps debugging easier and reduces config-generation blast radius.

For each PHP track, PV generates a worker root config that imports per-Project generated config files for Projects on that track.

When PHP-track worker config changes, PV reloads the worker where supported and restarts it only if reload fails or is unavailable.

Project-serving worker logs are captured per PHP track, with Project hostname included in access logs where feasible. PV v1 does not create per-Project log files. Caddy/FrankenPHP log rotation directives should be used where practical.

Project-serving worker logs are split by PHP track, such as `~/.pv/logs/workers/php-8.4.log`, because one worker serves all Projects assigned to that PHP track.

Gateway access logs are enabled by default, stored locally under `~/.pv/logs/`, and rotated. Gateway logs are split into access and error logs, such as `~/.pv/logs/gateway/access.log` and `~/.pv/logs/gateway/error.log`, when FrankenPHP/Caddy supports that cleanly. Structured/JSON logs should be used when Caddy/FrankenPHP supports them cleanly.

When routing or Gateway config changes, PV reloads the Gateway config using Caddy/FrankenPHP reload capabilities where possible. PV restarts the Gateway only if reload fails or is unavailable.

PV owns one local CA and passes that CA to the Gateway's FrankenPHP/Caddy configuration. FrankenPHP/Caddy generates and manages Project certificates from that CA as needed for hostnames in PV's desired routing table: primary Project hostnames plus additional `hostnames:` from valid Project config. The Gateway selects certificates by SNI.

The Gateway does not automatically route `*.project.test` to a Project. Subdomain routing must be explicitly requested in Project config, which allows `acme.test` and `api.acme.test` to belong to different Projects.

For unknown `.test` hostnames, the Gateway should return a simple self-contained HTML response explaining that no PV Project is linked for the hostname and suggesting `pv link` when technically feasible.

PV v1 avoids PHP extension management. PHP and FrankenPHP Managed Resource artifacts are distributed as prebuilt macOS binaries with a fixed, common extension set baked in. PV does not expose extension install, uninstall, or per-Project extension configuration in v1.

PV v1 builds standalone PHP and FrankenPHP as single-binary/static-style artifacts with fixed compiled-in extensions. These artifacts must not depend on Homebrew or local package-manager libraries. PV v1 does not support dynamic PHP extension loading, `phpize`, or PECL-installed extensions.

Standalone PHP artifacts include the `php` executable and runtime files needed by that build. They do not include `phpize` or `php-config` in v1 because user-built extensions are not supported.

The v1 fixed PHP extension set is Laravel-first and shared across supported PHP tracks: `bcmath`, `ctype`, `curl`, `dom`, `fileinfo`, `filter`, `hash`, `iconv`, `intl`, `json`, `libxml`, `mbstring`, `openssl`, `pcntl`, `pcre`, `pdo`, `pdo_mysql`, `pdo_pgsql`, `pdo_sqlite`, `phar`, `posix`, `redis`, `session`, `simplexml`, `sodium`, `sqlite3`, `tokenizer`, `xml`, `xmlreader`, `xmlwriter`, `zip`, and `zlib`.

For a given PHP track, standalone PHP and FrankenPHP must expose the same compiled-in PHP extension set so CLI and browser execution do not drift.

For a given PHP track, standalone PHP and FrankenPHP must use the exact same PHP patch version. For example, if the `8.4` track resolves to PHP `8.4.8`, both the standalone PHP artifact and the FrankenPHP artifact for that track use PHP `8.4.8`.

PV v1 ships one PHP build flavor per PHP track. Xdebug is not included in the default v1 PHP build. Extra-extension flavors, such as builds with `xdebug`, `imagick`, `swoole`, or `mongodb`, are out of v1 scope until PV deliberately designs multi-flavor PHP artifacts.

PV builds its own FrankenPHP artifacts for the PHP tracks it supports because upstream FrankenPHP releases do not provide the exact PV-required build matrix. The initial PV-managed FrankenPHP/PHP tracks are `8.2`, `8.3`, and `8.4`.

PV v1 does not support custom PHP ini settings in Project config.

- If there are 5 Projects and all of them use the same PHP version, PV provisions 1 Project-serving FrankenPHP process.
- If 2 Projects use PHP 8.3, 2 use PHP 8.4, and 1 uses PHP 8.5, PV provisions 3 Project-serving FrankenPHP processes. The Gateway proxies each Project hostname to the worker for that Project's PHP version.

User commands describe what should exist. The daemon reconciles the machine toward that desired state and records observed status when reality does not match.

PHP version resolution: Project config `php` field → global default.

PV does not infer PHP versions from `composer.json`. Composer constraints can be complex and are not always present, so Projects that need a specific PHP version should declare it in Project config.

If Project config asks for a PHP version that is not installed, daemon reconciliation installs it automatically.

`pv php:default <track>` sets the global default PHP track. The global default is used outside linked Projects and by linked Projects without `php` in Project config.

If `pv php:default <track>` targets a PHP track that is not installed, daemon reconciliation installs the missing standalone PHP and FrankenPHP artifacts for that track.

`pv php:default <track>` streams progress and waits when installation is required.

## Desired State and Daemon Availability

Commands that change desired state write that state even when the daemon is not running.

After writing desired state, the command requests reconciliation. If the daemon is not running, the command warns that reconciliation is pending and exits successfully. The command exits non-zero only when recording desired state fails or the command input is invalid.

Desired-state commands wait for reconciliation only when their contract implies readiness, such as `pv setup` and explicit install/update commands. Fast intent commands such as `pv link` and `pv unlink` write desired state, request reconciliation, and return after the daemon accepts the job.

Non-waiting commands such as `pv link` exit zero once desired state is recorded and the daemon has accepted reconciliation, even if that reconciliation later fails. Waiting commands such as setup, install, update, and restart exit non-zero when their daemon job fails.

Install and update commands submit daemon jobs, stream progress over the socket, and exit when the job completes or fails.

`pv update` updates both the PV application and all installed Managed Resource tracks. Resource-specific update commands, such as `pv mysql:update`, update only installed tracks of that Managed Resource.

`pv update` self-updates the PV application before Managed Resources so the latest daemon owns resource update logic and manifest interpretation. It downloads the new `pv` binary, verifies its SHA-256 checksum, installs it under `~/.pv/bin/releases/<version>/pv`, atomically points `~/.pv/bin/pv` at the new release, coordinates daemon restart, reconnects to the daemon, refreshes the artifact manifest, and then updates installed Managed Resources as needed. If checksum verification, binary replacement, or daemon restart fails, `pv update` stops and reports the failure instead of continuing to Managed Resource updates.

`pv update` does not prompt for confirmation before applying available PV application or Managed Resource updates. Running the command is the explicit user intent to update. Safety comes from checksum verification, atomic release installation, rollback, and non-destructive data handling.

`pv update --check` refreshes the PV app update manifest and Managed Resource artifact manifest, reports available PV application updates, reports updates for installed Managed Resource tracks, and exits without applying changes. It checks PV application updates even when no Managed Resources are installed.

`pv update --check` exits zero when the update check succeeds, even if updates are available. It exits non-zero only when the check itself fails.

`pv update --check` requires the PV daemon to be running. If the daemon is not available, it fails with a clear message suggesting `pv daemon:restart` or `pv setup`.

`pv update --check` is read-only and does not take the self-update lock. If a real update is already in progress, it fails clearly instead of waiting or mutating state.

`pv update --check --json` is supported in v1 and reports machine-readable PV application update availability plus installed Managed Resource track update availability. Non-check update progress does not need JSON output in v1.

`pv update --check` reports both PV application update availability and Managed Resource update availability when possible. If Managed Resource update metadata requires a newer PV application version, the check reports the available PV application update and clearly marks Managed Resource update availability as blocked until PV is updated.

PV does not auto-check for updates in the background. Update-related network checks happen only when users run `pv update`, `pv update --check`, setup/install commands that need manifests, or explicit install/update commands for Managed Resources.

Resource-specific update commands do not support `--check` in v1. Update preview is available only through top-level `pv update --check`.

PV application self-update metadata comes from a PV app update manifest that is separate from the Managed Resource artifact manifest. The app update manifest includes PV application version metadata, platform-specific download URLs, SHA-256 checksums, and compatibility fields needed by the self-updater.

The PV app update manifest is published at a stable PV-owned URL used by the Rust self-updater. Initially, that stable URL may be backed by GitHub Releases, such as a versioned `pv-app-manifest.json` release asset plus a stable latest manifest URL. The human-facing installer URL is separate and serves a generated installer script based on the same PV app release metadata.

PV v1 relies on HTTPS/GitHub trust for the PV app update manifest itself. The app update manifest format should allow signatures to be added later without breaking compatibility.

For `pv update`, the CLI fetches the PV app update manifest and performs PV binary self-update before handing Managed Resource update work to the daemon. The daemon owns Managed Resource manifest refresh, install, update, and runtime reconciliation.

If the PV application is already current, `pv update` still continues to the daemon-owned Managed Resource update phase. Top-level update always checks installed Managed Resource tracks after the PV application self-update phase.

Top-level `pv update` updates all installed Managed Resource tracks, not only tracks currently needed by linked Projects.

For `pv update --check`, the CLI fetches the PV app update manifest and computes PV application update availability, then asks the daemon to refresh the Managed Resource artifact manifest and report installed Managed Resource track update availability.

PV self-update keeps the previous PV application binary for rollback. If the updated binary cannot restart the daemon and report healthy, `pv update` restores the previous binary, restarts the daemon again, and reports that the app update was rolled back.

PV application rollback applies only to PV app update failures or post-update daemon health failure. If the PV app self-update succeeds but later Managed Resource updates fail, PV keeps the newer app binary installed and reports the Managed Resource update failure.

PV applies database migrations after swapping to the new PV binary and restarting with that binary. The new binary owns its embedded migrations; rollback remains safe because migrations are required to be backward-compatible with the immediately previous PV version.

If the new PV binary's database migration fails during self-update, `pv update` rolls back to the previous PV binary, restarts the daemon with the previous binary, and reports the migration failure. Transactional migrations should leave `pv.db` unchanged on failure.

`pv update` does not create an extra `pv.db` backup before every PV application self-update. The embedded migration system creates a timestamped backup only when migrations are about to run.

PV application self-update restarts the daemon/control plane but does not stop currently running Gateway, Project-serving workers, or backing Managed Resource processes before swapping the PV binary. The updated daemon adopts existing PV-owned child processes where ownership verification passes and then reconciles/update resources as needed.

PV self-update holds an OS-level filesystem lock at `~/.pv/run/update.lock` for the binary swap and daemon transition. The foreground `pv update` process owns the lock, and both old and new daemon processes check the same lock before accepting mutating work. While the lock is held, concurrent mutating commands fail clearly with an update-in-progress message. The daemon rejects mutating requests while the lock is held; it does not queue them for later execution. Simple local read-only commands that do not require daemon protocol compatibility, such as `pv env`, may still run. Read-only commands that need daemon state, such as `pv status`, fail clearly during the transition. The lock file may remain on disk after the update; the active OS lock is released automatically when the owning process closes it or exits.

After a successful PV application self-update, PV keeps the current app release plus one previous app release under `~/.pv/bin/releases/`. Older app releases are pruned.

`pv restart` asks the daemon to restart all currently running PV-managed runtime processes, including the Gateway, Project-serving workers, and running Managed Resource processes, then reconcile desired state. Desired-but-stopped runtime processes are started during that reconciliation. It streams progress and exits when restart/reconciliation completes or fails. `pv daemon:restart` is the lower-level command for restarting the daemon/LaunchAgent itself.

`pv restart` may restart daemon-owned runtime tasks such as the internal DNS resolver without restarting the daemon process itself. If DNS repair requires privileged system config changes, PV reports repair required instead of mutating privileged config in the background.

If `pv restart` is run while the daemon is not running, it fails with a clear message suggesting `pv daemon:restart` or `pv setup`.

Managed Resource list commands, such as `pv php:list` and `pv mysql:list`, list installed tracks by default. For backing Managed Resources, list output shows whether each track is running, its assigned port when running, and linked Project usage counts. It does not show secrets.

`pv php:list` marks the global default PHP track and may show Project usage counts for each installed track.

PV supports Managed Resource aliases for ergonomics: `pg` for Postgres, `mail` for Mailpit, and `s3` for RustFS. Command help output and documentation show canonical resource namespaces first, such as `postgres:*`, while documenting aliases as secondary.

The canonical Managed Resource name is `postgres` for commands, filesystem paths, Project config storage, and internal state. Project config accepts registered Managed Resource aliases, including `postgresql`, and normalizes them to canonical names.

Project config, command namespaces, filesystem paths, and internal state use canonical lowercase Managed Resource names: `mysql`, `postgres`, `redis`, `mailpit`, and `rustfs`. Prose may use display names such as MySQL, Postgres, Redis, Mailpit, and RustFS.

Managed Resource uninstall commands remove installed binaries and runtime metadata by default. They delete Managed Resource data only when `--prune` is provided. `--prune` requires interactive confirmation unless `--force` is also provided.

PV refuses to uninstall a Managed Resource track currently needed by a linked Project unless `--force` is provided. Forced uninstall marks affected Projects failed or pending; reconciliation may reinstall the track if Project config still declares it.

`pv uninstall` is safe by default. It stops and unregisters the LaunchAgent, removes `/etc/resolver/test`, removes PV's `pf` redirect rules, removes PV local CA trust, removes the installer-managed `PV ENV` shell profile block when present, stops PV-managed processes, and removes PV app binaries, shims, runtime metadata, sockets, generated configs, and installed Managed Resource binaries. Before editing a shell profile during uninstall, PV creates a backup.

By default, `pv uninstall` preserves logs, `pv.db`, certificates, Composer home/cache, Managed Resource data, and Project `.env` blocks. `pv uninstall --prune` removes all PV-owned state under `~/.pv` and PV-owned system integration files/trust. Prune deletes local PV-owned data trees rather than attempting logical cleanup inside Managed Resources first. Shell profile backups created by PV are user safety artifacts and are not removed by `--prune`. `--prune` requires interactive confirmation unless `--force` is also provided.

## Filesystem Layout

PV stores machine state and installed assets under `~/.pv`:

```text
~/.pv/
  pv.db
  bin/
    releases/
  run/
  logs/
  downloads/
  config/
  certificates/
  composer/
  resources/
    php/
    frankenphp/
    composer/
    mysql/
    postgres/
    redis/
    mailpit/
    rustfs/
```

PV enforces user-only filesystem permissions. `~/.pv` should be `0700`; sensitive files such as `pv.db`, CA private keys, and generated secret material should be `0600`.

PV detects unsafe permissions on user-owned files under `~/.pv`. Foreground repair commands such as `pv setup` may repair permissions; daemon startup records repair-required status when it cannot safely repair in the background. `pv doctor` remains strictly read-only and suggests repair commands instead of mutating state.

`pv setup` repairs unsafe permissions on PV-owned user-local files automatically and reports what changed.

`resources/` owns PV-managed binaries, versions, and resource-specific runtime/data layout. `bin/`, `run/`, `logs/`, `downloads/`, `config/`, `certificates/`, `composer/`, and `pv.db` are top-level because they belong to PV itself rather than one Managed Resource.

Generated config files live under `~/.pv/config/`, with subdirectories for Gateway, `pf`, resolver, and LaunchAgent config.

Generated config files are disposable outputs regenerated from `pv.db`, Project config, and the artifact manifest. They are not source of truth and may be overwritten during reconciliation.

During reconciliation and `pv restart`, the daemon regenerates and validates Gateway/worker configs before reloading or restarting runtime processes. If config generation or validation fails, PV keeps currently working processes running and reports the failure.

Gateway/worker config validation uses the managed FrankenPHP/Caddy binary's config validation command against the generated config before reload or restart. If validation fails, PV keeps the previous active config/processes and surfaces the validation error in observed state and logs.

Generated Gateway/worker config writes are atomic. PV writes new config to temporary files, validates them, then atomically renames them into place so runtime processes never read partial config files.

PV keeps the previous active generated Gateway/worker config until the new config validates and reloads successfully. If reload fails after validation, PV restores or keeps the previous config and reports the failure.

PV v1 does not support user-editable Caddy snippets or custom Gateway/worker config. Generated Gateway and worker config is fully PV-owned.

`pv.db` is PV's only machine-owned source-of-truth store. PV avoids storing source-of-truth machine state in separate JSON or YAML files.

`pv.db` stores both desired state and observed state. Desired state records what PV should make true; observed state records the daemon's latest view of reality, including health, pending work, and failures.

PV enables SQLite WAL mode for `pv.db` to improve concurrent read/write behavior between CLI commands and the daemon. Transactions should stay short.

PV enables SQLite foreign key enforcement for every `pv.db` connection.

Observed state in `pv.db` stores the current/latest status only. Historical detail belongs in logs, not database event history, for v1.

PV writes structured JSONL logs under `~/.pv/logs/` for daemon, reconciliation, and Managed Resource events. CLI output remains human-readable.

Managed Resource process logs are split by resource and track, such as `~/.pv/logs/resources/mysql/8.0.log` and `~/.pv/logs/resources/redis/8.6.log`, because multiple tracks can run simultaneously.

PV rotates logs by size and retains a small fixed number of rotated files. PV v1 does not need compression or complex retention policy.

CLI commands may write desired state directly to `pv.db`, including when the daemon is down. CLI and daemon code must use the same state library and SQLite transactions. When the daemon is running, commands notify it over the Unix socket after committing desired state changes.

Concurrent writes use SQLite transactions with a short busy timeout. If a command cannot acquire the write lock quickly, it fails with a clear message that PV state is busy.

The daemon accepts multiple reconciliation requests but runs reconciliation jobs one at a time in a queue for v1. Internal work inside a job, such as artifact downloads, may still run in parallel.

Daemon reconciliation job metadata and final status are persisted in `pv.db`. Live progress streaming is kept in memory. If the daemon crashes mid-job, the next daemon startup marks interrupted jobs failed or abandoned and reconciles desired state again.

Once the daemon accepts a long-running job, the job continues even if the initiating CLI process disconnects. The CLI socket stream is a progress subscriber, not the owner of the work.

PV v1 does not support reattaching to an active job's progress stream. Users inspect active/recent work through `pv jobs`, `pv status`, and logs.

PV v1 does not support cancelling active daemon jobs. Jobs run to completion or failure; `pv daemon:restart` remains the blunt recovery option for stuck work.

When `pv daemon:restart` interrupts active jobs, PV marks those jobs abandoned or failed in job history. On startup, the daemon reconciles desired state again to repair any interrupted work.

`pv daemon:restart` waits for the LaunchAgent to restart, for the Unix socket to become available, and for the daemon to report healthy. If the daemon does not become healthy within 15 seconds, the command exits non-zero. After the daemon is healthy, it enqueues reconciliation and returns without waiting for full runtime reconciliation.

PV keeps a fixed recent daemon job history in `pv.db`, such as the last 100 jobs. Detailed history belongs in logs.

Filesystem watcher events are briefly debounced. Queued reconciliation requests are coalesced by scope where possible, and each job reconciles from current `pv.db` and Project config rather than stale event payloads.

Reconciliation scopes are `system`, `project:<id>`, and `resource:<name>:<track>`. Whole-system setup/update uses `system`; Project config changes use `project:<id>`; explicit Managed Resource install/update work uses `resource:<name>:<track>`. If dependencies overlap in a way that is hard to isolate safely, the daemon may promote work to `system` scope.

`~/.pv/bin/` contains the active `pv` application symlink plus PV-managed shims and symlinks. PV application self-update stores versioned app binaries under `~/.pv/bin/releases/<version>/pv` and atomically swaps the active `~/.pv/bin/pv` symlink to point at the selected release. Actual Managed Resource versioned binaries and assets live under `~/.pv/resources/`, which keeps upgrades and multi-version binaries easier to manage.

PV v1 exposes generic shims only. It does not create versioned shims like `php8.4` or `mysql8.0`; exact versioned binaries remain available under `~/.pv/resources/` for advanced use.

The `php` shim is Project-aware, similar to version managers such as `fnm` or `nvm`. When run inside a linked Project, it uses that Project's resolved PHP track. Outside a linked Project, it uses the global default PHP track.

Composer is split by responsibility: the Composer PHAR and version metadata live under `~/.pv/resources/composer/`, the Composer shim lives under `~/.pv/bin/`, and `~/.pv/composer/` is the user-facing `COMPOSER_HOME` for global packages and cache.

The Composer shim invokes the Composer PHAR through PV's `php` shim so Composer inherits Project-aware PHP selection. Inside a linked Project, Composer uses that Project's PHP track; outside, it uses the global default PHP track.

Composer uses the same artifact track model as other Managed Resources, but v1 exposes only one Composer track: `2`. PV installs and updates the latest non-revoked Composer artifact in the `2` track. Composer 1 compatibility is out of v1 scope.

Composer commands keep the user-facing UX simple in v1. `pv composer:install` resolves internally to Composer track `2` and does not accept a version argument while only one track exists. If Composer 3 or another supported Composer track is added later, PV may expose an explicit Composer version argument then.

PV does not package Composer as a platform-specific binary in v1. Composer remains a managed Composer 2 PHAR invoked through PV's `php` shim. For artifact lifecycle consistency, PV distributes Composer as a PV-owned `.tar.gz` artifact containing `composer.phar` and license metadata rather than downloading the raw PHAR directly.

Other Managed Resource CLI shims, such as `mysql`, `psql`, `redis-cli`, or `rustfs`, use global/default installed tracks in v1. They are not Project-aware. When multiple tracks are installed, these shims use the manifest default track if installed, otherwise the highest installed track according to manifest ordering. If the choice is ambiguous, the shim errors and lists installed tracks.

PV v1 has no global default commands for non-PHP Managed Resources, such as `pv mysql:default`.

Managed Resource data directories live inside the owning resource tree. For example, MySQL data lives under `~/.pv/resources/mysql/<version>/data/`.

Managed Resources that require initialized data directories, such as MySQL and Postgres, have an idempotent init step before process start. Reconciliation initializes missing per-track data directories and never reinitializes data that already exists.

Database-style Managed Resource initialization happens only when a resource track is first needed to run. `pv setup` installs default artifacts but does not initialize backing resource data directories unless a linked Project needs that track.

PV application releases are separate from Managed Resource artifact releases. The PV app update manifest is separate from the Managed Resource artifact manifest. Managed Resource artifacts are PV-owned rolling releases that can be rebuilt on their own cadence, such as weekly or when upstream dependencies change.

PV discovers available Managed Resource artifacts through a PV-owned remote artifact manifest. The manifest lists artifact metadata: resources, tracks, versions, platforms, download URLs, checksums, sizes, publication timestamps, default versions, manifest schema version, and minimum supported PV version. The manifest and artifact archives are published to PV-owned object storage/CDN endpoints, such as Cloudflare R2 behind a PV-owned HTTPS domain. PV does not scrape GitHub release asset names at runtime and does not hardcode artifact versions in the app binary.

The Managed Resource artifact manifest points only to PV-owned packaged artifacts, not raw upstream archives or local build recipes. PV never builds Managed Resource binaries on the user's machine during setup, install, update, or reconciliation.

The PV artifact release pipeline may either wrap suitable upstream binaries or build missing binaries from source, but it always produces a normalized PV artifact archive before publishing. For example, if Redis does not publish the macOS binary shape PV needs, the release pipeline builds Redis ahead of time and publishes the resulting PV-owned Redis artifact. The release pipeline is expected to run in hosted automation such as GitHub Actions, not on user machines.

Artifact recipes prefer wrapping official upstream binaries when those binaries pass PV's relocation, validation, and smoke-test requirements. Recipes build from source when upstream binaries are unavailable, do not match PV's required build matrix, cannot be made relocatable safely, or fail PV's smoke tests.

Managed Resource artifact build recipes, scripts, patches, and expected archive layouts live in the PV repository, such as under `release/artifacts/`, so artifact production changes are reviewed with PV adapter and manifest compatibility changes. Deployment secrets and storage credentials stay in CI/provider configuration, not in the repository.

Artifact recipes may apply small versioned build or packaging patches when required for macOS portability, static-style PHP/FrankenPHP builds, relocatable artifacts, or reproducible packaging. PV avoids long-lived behavior-changing forks of upstream Managed Resources. Any patch that changes runtime behavior rather than build/packaging behavior requires an explicit design decision before publication.

PV v1 keeps Managed Resource artifact build/release automation in the same repository and CI system as the PV application. A separate artifact-build repository is deferred until coordination or security needs justify the split.

Managed Resource artifact build and publication workflows are separate from PV application binary build and release workflows. Normal PV application CI/release does not rebuild Managed Resource artifacts. Artifact publication is an explicit release workflow with resource, track, upstream version, PV build revision, and target platform inputs.

Shared artifact release metadata validation and manifest generation are implemented in Rust as internal repository tooling, such as an `xtask` or `pv-release` crate. Resource-specific build recipes remain shell scripts because they mostly orchestrate upstream tools such as `configure`, `make`, `cmake`, `spc`, `go build`, `cargo build`, `codesign`, `otool`, and `tar`.

PV repackages even usable upstream binaries into a consistent artifact layout instead of exposing raw upstream archive layouts to the client. This keeps install, validation, rollback, and adapter behavior stable when upstream packaging changes.

Each Managed Resource artifact is distributed as a single `.tar.gz` archive per resource, track, upstream version, PV build revision, and platform. PV downloads one archive, verifies it against the remote artifact manifest, unpacks it into a temporary directory, validates the expected adapter-specific files, and then atomically installs it.

Each Managed Resource artifact archive expands into exactly one top-level directory named from the artifact identity, such as `redis-7.2.5-pv1-darwin-arm64/`. The archive must not place files directly at the extraction root.

Each Managed Resource artifact archive includes upstream license and notice files where required by the redistributed resource and bundled dependencies. PV v1 does not provide a dedicated licenses command.

License and notice validation happens in the artifact release pipeline, not in PV's client-side resource adapters. Runtime adapters validate files required to install and run the resource, while publication checks enforce licensing metadata before an artifact appears in the public manifest.

Managed Resource artifacts must be relocatable before publishing. Any shebang, rpath, install-name, or embedded path fixes happen in the release pipeline before final signing and checksum generation. User machines do not patch Managed Resource binaries during install.

Relocation validation scans every Mach-O executable and dynamic library in a candidate artifact before publication. The release pipeline fails artifacts that reference build-machine paths, absolute Homebrew paths such as `/opt/homebrew` or `/usr/local/Cellar`, `/Users/runner`, or non-system dynamic libraries outside the artifact root. macOS system libraries under `/usr/lib` and `/System/Library` are allowed.

For v1, PV ad-hoc signs Managed Resource Mach-O binaries in the release pipeline after any binary path fixes. Paid Developer ID signing and notarization for Managed Resource artifacts are deferred unless macOS Gatekeeper or quarantine behavior requires them for a reliable v1 install experience. Checksums are computed only after final signing and packaging.

The remote Managed Resource artifact manifest is the only manifest in v1. PV-owned artifact archives do not contain per-archive manifest files in v1; validation comes from the remote manifest plus the compiled-in resource adapter rules.

Published Managed Resource artifact archive URLs are immutable. Artifact object keys include enough identity to distinguish the resource, track, upstream version, PV build revision, platform, and content. If a published artifact is bad, PV publishes a new build revision and updates the manifest to point at the new artifact instead of replacing the existing object in place.

Managed Resource artifact identity includes both the upstream resource version and a PV build revision. For example, a Redis artifact may represent upstream Redis `7.2.5` with PV build revision `pv1`; if PV changes packaging, patches, build flags, or validation for the same upstream version, it publishes `pv2` rather than mutating `pv1`.

The artifact manifest stores upstream version and PV build revision as separate fields, such as `upstream_version` and `pv_build_revision`, plus a derived display/install identity such as `artifact_version: "7.2.5-pv1"`. Each artifact also records `published_at`, the timestamp when that artifact became installable through the public manifest. PV uses the separate version fields and `published_at` for update logic and diagnostics while showing the combined artifact version where concise output is useful.

The artifact manifest may include artifact provenance metadata such as upstream source URL, upstream checksum, applied patch identifiers, PV repository commit SHA, recipe path/version, build run ID, and build timestamp. Provenance metadata is for diagnostics, audit, and release operations; it is not a client-side build instruction set.

The artifact manifest does not define Managed Resource lifecycle behavior or resource-specific archive layout requirements. Install, start, init, readiness, allocation, reconciliation behavior, and required file/path validation live in PV's resource adapters because each Managed Resource has different lifecycle rules. For example, the Redis adapter knows it needs `bin/redis-server`, while the Postgres adapter knows it needs `bin/postgres`, `bin/initdb`, and supporting `share/postgresql` files.

PV resource adapters are compiled into the Rust binary. PV will not support plugin resource adapters; all control-plane and adapter behavior lives in the single `pv` binary. Managed Resources remain external binaries/artifacts managed by PV.

If the artifact manifest schema is unsupported, or the manifest requires a newer PV version than the installed PV application, commands that need artifact metadata fail clearly and tell the user to run `pv update`.

Manifest incompatibility does not stop already-installed local runtime from working. Existing linked Projects, Gateway, DNS, installed Managed Resources, and desired state continue using local `pv.db` and installed artifacts. Only commands that need artifact metadata fail.

The artifact manifest defines resource-specific update tracks. Project config versions select a track, not necessarily a full upstream semantic version. `pv update` and resource-specific update commands update installed Managed Resources only within their existing tracks.

Examples: MySQL `8.0` tracks update within `8.0.x`, MySQL `8.4` tracks update within `8.4.x`, PostgreSQL `17` tracks update within `17.x`, and PostgreSQL `18` tracks update within `18.x`.

Install commands and Project config versions resolve to the latest artifact in the requested track. "Latest" means the non-revoked artifact with the newest `published_at` timestamp after platform selection. If two candidate artifacts for the same resource, track, and platform have the same `published_at`, the manifest is ambiguous and invalid. For example, `pv mysql:install 8.0` installs the latest available MySQL artifact in the `8.0` track.

`latest` is accepted as a version alias that resolves to the manifest's default track for that Managed Resource. PV stores the resolved track, not `latest`, so existing Projects do not float when manifest defaults change later.

PV does not rewrite Project config to replace `latest` with the resolved track. The resolved track is stored internally in `pv.db` and shown in status/list output.

PV v1 supports track-based versions only. It does not support exact artifact pinning in Project config.

Projects attach to Managed Resource tracks. When PV updates an installed track to a newer artifact, Projects using that track automatically use the updated artifact.

When updating a running Managed Resource track, PV restarts it immediately as part of the explicit update command. If restart fails, PV preserves the previous artifact when possible and reports the failure.

PV caches the last successfully fetched artifact manifest under `~/.pv/downloads/manifest.json`. When offline, PV can use cached metadata and already-downloaded or installed resources, but cannot install versions missing from the cache/downloads. Offline or stale-manifest failures should be reported clearly.

`pv update` refreshes the artifact manifest every time. Setup and install commands try to fetch the latest manifest and fall back to the cached manifest when offline. PV v1 does not need a manifest cache TTL.

`pv setup` fails if it cannot fetch the artifact manifest and no cached manifest exists, because default Managed Resource installation cannot be planned without artifact metadata. Completed system integration steps remain in place and setup remains safe to rerun.

PV always verifies each downloaded Managed Resource artifact against the manifest-provided SHA-256 before unpacking or installing. If verification fails, PV deletes the bad download, fails the job, surfaces the error, and does not perform install/unpack side effects for that artifact.

Checksum verification failure is always a hard stop for the current operation. PV deletes the bad download, records expected/actual checksum details in logs, and does not continue past failed checksum verification.

Commands that download artifacts attempt each download up to 2 times with a 300ms backoff before failing. Checksum verification is not retried: a checksum mismatch deletes the bad download and fails the current operation immediately.

Parallel artifact downloads are limited to 4 concurrent downloads.

PV v1 does not support resumable artifact downloads. Failed partial downloads are deleted and retried from scratch.

Successfully downloaded artifacts are cached under `~/.pv/downloads/` after installation so reinstall and repair operations can avoid network when the cached artifact checksum still matches.

Managed Resource installation is atomic. PV unpacks/installs into a temporary directory, verifies expected files, then renames into the final resource track location. If installation fails, PV deletes the temporary directory and leaves any previous installed version intact.

Each Managed Resource track separates immutable artifact releases from mutable data/config. For example, `~/.pv/resources/mysql/8.0/releases/<artifact-version>/` contains unpacked binaries/assets, `~/.pv/resources/mysql/8.0/current` points to the active artifact revision, and mutable data/config stays outside `releases/` under the track directory.

Managed Resource updates are atomic. PV installs the new artifact side-by-side, validates it, updates the track pointer only after validation succeeds, and keeps the previous artifact available for rollback if restart fails.

After a successful Managed Resource update, PV keeps the current artifact revision plus one previous artifact revision per track for rollback. Older non-current artifact revisions are pruned. Mutable data/config outside `releases/` is never pruned by update cleanup.

PV v1 relies on HTTPS trust for the artifact manifest itself plus SHA-256 verification for each downloaded artifact archive. PV v1 does not require cryptographic manifest signatures. The manifest format should allow signatures to be added later without breaking compatibility.

Public v1 should support separate Managed Resource artifacts for both Apple Silicon and Intel macOS: `darwin-arm64` and `darwin-amd64`. PV v1 does not use universal macOS Managed Resource artifacts. If build complexity blocks progress, Apple Silicon-only is acceptable for an initial preview, but not as the intended public v1 scope.

Managed Resource artifact recipes set an explicit macOS deployment target of macOS 13.0 unless a later design decision raises PV's minimum supported macOS version. Recipes must not silently inherit a newer GitHub runner deployment target.

The artifact manifest may use `platform: "any"` only for truly portable artifacts that do not contain platform-specific binaries. Composer is the expected v1 `platform: "any"` artifact because PV packages `composer.phar` inside a PV-owned archive. Native Managed Resource artifacts use explicit platform values such as `darwin-arm64` or `darwin-amd64`.

When resolving artifacts, PV prefers an exact platform match over `platform: "any"`. PV uses `platform: "any"` only when no exact platform-specific artifact exists for the selected resource, track, and artifact version.

The artifact release pipeline should build and validate `darwin-arm64` and `darwin-amd64` artifacts on native macOS runners for each architecture when available. Cross-compilation is acceptable only for resources where target-architecture smoke tests prove the artifact works. For database/runtime artifacts such as Postgres, MySQL, and FrankenPHP, target-architecture validation is required before publication.

For macOS v1 artifacts, recipes rely on GitHub-hosted macOS runners plus recipe-managed build dependency setup rather than containerized builds. Recipes should pin build tool versions where practical. Homebrew may be used as a CI build-time dependency source, but published artifacts must not retain unmanaged Homebrew runtime dependencies or absolute Homebrew paths.

Maintainer-local macOS artifact builds also run natively on macOS. Docker is not a supported path for producing or validating macOS Managed Resource artifacts because it cannot exercise native Mach-O linking, signing, rpaths, or runtime behavior.

Strict byte-for-byte reproducible Managed Resource builds are not a v1 requirement. Artifact recipes should still record provenance and pin source/dependency/tool inputs where practical, but v1 does not block publication on deterministic rebuild verification.

The artifact release pipeline must pass adapter-specific smoke tests before publishing a Managed Resource artifact. Redis starts `redis-server`, checks `redis-cli ping` returns `PONG`, and stops cleanly. Postgres runs `initdb`, starts `postgres`, runs `psql SELECT 1`, and stops cleanly. MySQL initializes a temporary data directory, starts the server, connects as admin, runs `SELECT 1`, and stops cleanly. Mailpit starts the server, checks the HTTP UI and SMTP port bind, and stops cleanly. RustFS starts the server, checks S3 API readiness, creates or lists a test bucket, and stops cleanly. FrankenPHP/PHP runs `php -v`, verifies the expected PHP version and fixed extension set, serves a tiny PHP site through FrankenPHP over loopback, and stops cleanly. PHP extension validation compares the actual compiled extension list against PV's declared v1 set for both standalone PHP and FrankenPHP; missing required extensions or unexpected extra extensions fail publication unless explicitly allowed by a later design decision.

Artifact object upload and public artifact availability are separate steps. The release pipeline may upload immutable candidate artifact archives after they pass their own build checks, but PV clients only see artifacts referenced by the published artifact manifest. The public manifest references only artifacts that passed required smoke tests. Partial manifest publication is allowed only for intentionally supported platforms/resources; public v1 should not mark a resource track generally available until both `darwin-arm64` and `darwin-amd64` artifacts pass.

Artifact manifest publication is atomic from the client's perspective. The release pipeline generates and validates a complete manifest, uploads it under a versioned immutable key, then updates the stable manifest entrypoint last. PV clients must never observe a half-written manifest. If the storage backend cannot provide sufficiently atomic replacement for the stable manifest object, the stable entrypoint may be a small index file that points to the current versioned manifest.

The public artifact manifest is generated from structured artifact release metadata and must not be edited by hand. Artifact publication records metadata such as resource, track, versions, platform, URLs, checksums, sizes, provenance, and revocation state, then generates and validates the manifest from that source data.

Structured artifact release metadata is stored as PV-owned immutable records in the artifact object storage, not only in git. The repository owns build recipes, patches, expected layouts, and metadata schemas; artifact publication writes release records to storage and regenerates the public manifest from those records. This allows manifest publication, revocation, and repair workflows without requiring a repository commit for every metadata operation.

Artifact release records are immutable. Artifact revocation is recorded as a separate append-only metadata record that references the artifact identity, reason, timestamp, and replacement artifact when available. The manifest generator combines immutable release records and append-only revocation records to produce the current public artifact manifest.

PV retains artifact archives referenced by any still-supported artifact manifest version indefinitely. Unreferenced candidate artifacts, failed builds, and superseded artifacts that were never referenced by a published manifest may be pruned on a fixed retention window, such as 30-90 days. PV must not delete an artifact archive while an older supported manifest could still point to it.

The artifact manifest supports emergency artifact revocation with a reason. Fresh installs refuse revoked artifacts. Already-installed revoked artifacts may continue running so existing local development is not abruptly broken, but `pv status` and `pv update --check` warn clearly. `pv update` moves installed revoked artifacts to a non-revoked replacement when one is available.

If the newest artifact in a requested track is revoked, install and update commands may fall back to the newest non-revoked artifact in the same track when the manifest explicitly lists that artifact as installable. PV warns that the newest artifact was revoked and identifies the installed fallback artifact. PV never falls back across tracks automatically.

MySQL, PostgreSQL, Redis, Mailpit, and RustFS run as shared machine-level Managed Resource instances per resource/track. Multiple tracks of the same Managed Resource can run simultaneously. PV v1 does not create isolated per-Project service instances.

Backing Managed Resources bind only to IPv4 loopback (`127.0.0.1`) by default in v1.

Backing Managed Resources use TCP connectivity only in v1. PV does not expose Unix socket connection paths for Managed Resources in v1.

For backing Managed Resource env placeholders, `${host}` renders `127.0.0.1` in v1.

For Mailpit, SMTP host placeholders render `127.0.0.1`, while dashboard URL placeholders render a full HTTP URL using the assigned Mailpit UI port, such as `http://127.0.0.1:<ui_port>`.

For RustFS/S3 env placeholders, `${endpoint}` renders the S3 API endpoint as a full URL such as `http://127.0.0.1:<port>`. `${url}` renders the browser/public object base URL when RustFS exposes one cleanly. Separate `${host}` and `${port}` placeholders may still exist when needed.

Managed Resource runtime data is version-scoped. For example, MySQL 8.4 data lives under `~/.pv/resources/mysql/8.4/data/`.

For Managed Resources other than the Gateway, the daemon assigns runtime ports by first trying the resource's conventional default port, then incrementing until it finds an available port. For example, MySQL may run on `3307` if `3306` is already used by a process PV does not manage.

Assigned backing Managed Resource ports are persisted in `pv.db` per resource track. PV reuses the same port across restarts when available. If the stored port is occupied by a non-PV process, PV chooses a new free port, updates `pv.db`, restarts or reconfigures dependent runtime state, and updates PV-managed `.env` blocks during reconciliation.

PV does not need a separate port reservation system in v1. Reconciliation chooses a candidate free port, attempts to start the process, and if startup fails because the port was taken, chooses another free port, persists it, and retries within the same reconciliation.

When PV needs fallback high ports, it uses the `45000-48999` range. Backing Managed Resources still try conventional default ports first, then fall back into the PV high-port range.

Gateway, DNS, Project-serving workers, and backing Managed Resources all draw fallback ports from the same `45000-48999` range. PV relies on persisted assignments and collision checks rather than partitioning the range by runtime type.

Fallback port selection is deterministic. PV scans sequentially within `45000-48999`, starting from the preferred/default port when applicable, then persists the chosen port.

Before assigning a fallback port, PV checks existing port assignments in `pv.db` and avoids reusing a port already assigned to another desired runtime, even if that runtime is not currently running.

PV releases persisted port assignments for runtimes that are no longer desired so those ports can be reused later.

PV tries up to 10 candidate ports for a runtime during one reconciliation. If no candidate works, PV fails that runtime with a clear no-available-port error.

When a backing Managed Resource port changes, PV regenerates PV-managed `.env` blocks for all linked Projects that opt into env rendering and depend on that resource track.

## Project Configuration and Environment

Projects may opt in to Project-specific Managed Resource requirements and environment variable rendering through Project config (`pv.yml`). PV also accepts `pv.yaml`, but documentation should prefer `pv.yml`. Project config is read only from the Project root. PV does not search parent directories. If both files exist, Project config validation fails with a clear conflict. Symlinked Project config files are allowed only when the resolved file remains inside the canonical Project root. PV v1 does not support JSON Project config.

An empty Project config is valid and means no Project-specific overrides. PV uses defaults and does not touch `.env`.

Empty string values for meaningful config fields, such as `php` or Managed Resource `version`, are invalid.

Version/track fields may be YAML strings or numbers. PV normalizes them to strings during validation.

Project config can request Managed Resource tracks and define environment variable mappings for a Project. The mappings may use PV-provided placeholder values such as resource username, password, database, bucket, prefix, endpoint, and assigned port.

Project config can declare additional Project hostnames with `hostnames:`. These hostnames are routed to the same Project and included in that Project's certificate SANs. `hostnames:` is additive and does not include or redefine the primary Project hostname, which comes from `pv link --hostname` or the directory-derived default. Additional hostnames must be full `.test` hostnames; PV v1 rejects non-`.test` hostnames and wildcard hostnames.

All hostnames in PV's desired routing table are unique across primary and additional hostnames. If an additional hostname conflicts with another Project's primary or additional hostname, the Project config is invalid. If `pv link --hostname` tries to use a hostname that is already primary or additional for another Project, it fails with a clear collision error. PV keeps serving the last valid desired state and surfaces conflicts in `pv list` and `pv status`.

Project config `hostnames:` cannot include the Project's own primary hostname.

Project config `hostnames:` cannot contain duplicates after normalization.

Project config can override the served document root with `document_root:`. The value must be relative to the Project root; `.` is allowed and means the Project root. PV rejects absolute paths, document roots that escape the Project directory, or paths that do not exist as directories. PV validates document roots using canonicalized paths and rejects symlink-resolved paths that escape the canonical Project root.

Project config validation rejects unknown top-level keys and unknown nested keys with clear errors. Typos in resource, env, or allocation sections fail validation and keep the last valid desired state active.

Project config accepts YAML anchors, aliases, and merge keys as YAML syntax. PV resolves them before validation. Helper keys are not a PV feature; unknown keys that remain after YAML merge and alias resolution fail validation.

If Project config asks for a Managed Resource track that is not installed, daemon reconciliation installs it automatically.

Declaring a Managed Resource in Project config means the Project needs that resource. Reconciliation installs and starts the selected track even when no env mappings or Resource allocations are declared.

If no linked Projects need a running Managed Resource track anymore, the daemon stops that process. Installed Managed Resource assets remain on disk unless explicitly uninstalled.

Project config can also request that PV create Resource allocations inside shared machine-level Managed Resource instances. Examples include databases, buckets, credentials, prefixes, or similar resource-specific objects.

MySQL and Postgres Resource allocations create databases only in v1. They do not create dedicated per-allocation users/passwords.

SQL database creation uses the database provider defaults in v1. PV does not customize MySQL charset/collation or Postgres locale/encoding settings.

For SQL Resource allocations, PV only ensures the database exists and is reachable. PV does not inspect schemas, run migrations, or manage application database contents. Application schema and framework setup are user-owned.

PV creates and checks SQL Resource allocation databases through `sqlx` for MySQL and Postgres rather than shelling out to managed `mysql` or `psql` binaries. PV uses `sqlx` only for PV-owned admin operations such as readiness checks and database creation, not for application schema or migrations.

PV uses runtime/dynamic `sqlx` queries for these admin operations. It does not require `sqlx` offline query metadata in v1.

MySQL and Postgres use one PV-managed root/superuser credential per Managed Resource instance/track for local Project access. `${username}` and `${password}` come from the Managed Resource instance context, while `${database}` comes from the Resource allocation context.

SQL root/superuser passwords are randomly generated once per Managed Resource instance/track and stored in `pv.db`.

For SQL database names, allocation names are normalized to underscore-style identifiers: hyphens are converted to underscores. Project config allocation names may still use hyphens.

SQL database names use the same readable hostname-based naming approach as RustFS buckets, but with underscores: `<hostname_slug>_<allocation_name>`. The hostname slug includes the `.test` suffix, with dots and hyphens converted to underscores. For example, primary Project hostname `acme.test` and allocation `app-db` creates database `acme_test_app_db`.

SQL database names are generated when the Resource allocation is first created and then stored in `pv.db`. If the Project's primary hostname changes later, PV keeps using the existing stored database name instead of renaming the database or creating a new database.

Generated local development secrets are stored plainly in the user-owned SQLite database for v1. PV relies on filesystem permissions rather than macOS Keychain encryption at rest.

Generated credentials are stable. PV creates them once for the relevant Managed Resource instance/track or Resource allocation and does not rotate them during reconciliation or update. Credentials change only when the owning resource/allocation data is explicitly pruned or PV-owned state is removed.

PV v1 does not support credential rotation commands.

Redis Resource allocations create generated key prefixes only in v1. PV does not manage Redis logical DB indexes or ACL users in v1. Redis prefix values use `<hostname-slug>-<allocation>-`, where the primary Project hostname at allocation creation time is slugged by replacing dots with hyphens and includes the `.test` suffix. For example, primary hostname `acme.test` and allocation `cache` renders `acme-test-cache-`. Redis prefixes are generated when the Resource allocation is first created and then stored in `pv.db`. If the Project's primary hostname changes later, PV keeps using the existing stored prefix instead of switching to a new key namespace.

For Redis prefixes, allocation names are normalized the same way as RustFS bucket allocation segments: underscores are converted to hyphens.

Mailpit does not support Resource allocations in v1. It is a shared capture service that may expose resource-level env values such as SMTP host, SMTP port, and dashboard URL.

RustFS uses one randomly generated PV-managed root/access credential per Managed Resource instance/track so PV can manage and access the local RustFS instance. RustFS Resource allocations create per-Project buckets and render the shared instance access credentials plus bucket name. PV v1 does not create dedicated per-allocation RustFS access keys.

RustFS Resource allocation bucket names use the Project hostname slug and allocation name: `<hostname-slug>-<allocation_name>`. The hostname slug includes the `.test` suffix, with dots replaced by hyphens. For example, primary Project hostname `acme.test` and allocation `uploads` creates bucket `acme-test-uploads`.

For RustFS bucket names, allocation names are normalized to bucket-safe lowercase DNS-style labels: underscores are converted to hyphens. Project config allocation names may still use underscores.

For resources that normalize allocation names when generating underlying Resource allocation identifiers, such as SQL database names, Redis prefixes, and RustFS buckets, PV rejects Project config when two allocation names for the same resource normalize to the same generated name.

RustFS bucket names are generated when the Resource allocation is first created and then stored in `pv.db`. If the Project's primary hostname changes later, PV keeps using the existing stored bucket name instead of renaming the bucket or creating a new bucket.

PV manages RustFS buckets through S3-compatible APIs from PV's Rust code. PV should try the `object_store` crate first if it supports the bucket create/check operations PV needs against RustFS. If `object_store` cannot perform the required RustFS operations cleanly, PV may fall back to the AWS SDK for Rust. PV v1 does not include `mc` as a Managed Resource just to manage RustFS.

PV uses path-style S3 addressing for local RustFS endpoints, such as `http://127.0.0.1:<port>/<bucket>`, instead of virtual-hosted bucket subdomains. This avoids extra local hostname, certificate, and Gateway routing requirements for buckets.

After creating or confirming a RustFS Resource allocation bucket, PV checks that the bucket exists and is accessible with the credentials PV will render for the Project.

Each Managed Resource uses a generic `allocations:` map for Project-specific Resource allocations. Allocation names are scoped to the Project and Managed Resource. PV does not require an allocation `type` field in v1.

Allocation names must match `^[a-z][a-z0-9_-]*$`.

Resource allocations are reconciled even when they do not declare env mappings.

Empty allocation configs such as `app: {}` are valid; the allocation name alone requests creation.

If a Resource allocation is removed from Project config, PV stops reconciling it but leaves the underlying database, bucket, prefix, credentials, or other resource-specific objects in place.

PV v1 does not automatically garbage-collect orphaned Resource allocations.

PV uses readable hostname-based generated names for user-visible Resource allocation objects. SQL database names use `<hostname_slug>_<allocation_name>`, RustFS bucket names use `<hostname-slug>-<allocation_name>`, and Redis prefixes use `<hostname-slug>-<allocation>-`. These names are generated at first allocation creation and stored in `pv.db` so later Project hostname changes do not silently switch backing data.

PV applies resource-specific maximum name length rules to generated Resource allocation object names. If truncation is required, PV appends a short hash so names remain stable and collision-resistant.

If a Resource allocation is removed from Project config and later re-added with the same name for the same Project and Managed Resource, PV reuses the same stored generated Resource allocation object name and reconnects to the existing underlying object when it still exists.

If an underlying object for a desired Resource allocation is manually deleted outside PV, reconciliation recreates it and records the drift repair.

For existing Resource allocation objects with unexpected permissions or configuration, PV repairs only what it owns and understands. Ambiguous drift is reported instead of aggressively rewritten. V1 repair focuses on existence and basic access.

Resource allocation creation is best effort across multiple allocations and Managed Resources. PV creates what it can, records failures, and does not render `.env` until the full Project reconciliation succeeds.

If Resource allocation reconciliation fails but Project serving can still be configured, PV keeps serving the Project where possible and marks the Project degraded or failed for resources. It does not update `.env` with incomplete values.

Project config supports three environment mapping scopes:

- Root-level `env:` for Project-level values such as `APP_URL`.
- Managed Resource-level `env:` for shared service values such as host, port, or dashboard URL.
- Allocation-level `env:` for Resource allocation values such as database credentials or bucket names.

Env mapping precedence is root-level `env:`, then allocation-level `env:`, then Managed Resource-level `env:`. Root-level mappings are the most explicit Project-level override.

Project config env values support PV's simple placeholder syntax: `${name}`. PV replaces placeholders with values from the current Project, Managed Resource, or Resource allocation context.

`$$` escapes a literal dollar sign in env values. For example, `$${name}` renders `${name}`, and `$$${name}` renders `$` followed by the resolved value of `${name}`.

Placeholder names must use lowercase snake_case, such as `${project_url}`, `${access_key}`, `${secret_key}`, and `${smtp_port}`.

`${project_url}` renders the URL for the primary Project hostname, such as `https://acme.test`. It does not vary by additional hostnames.

Unknown placeholders fail Project config validation. PV keeps serving the last valid desired state and surfaces the validation error in `pv list` and `pv status`.

Placeholders resolve only from PV-provided context values. They do not reference other generated env keys.

Env mappings may also use literal values with no placeholders, such as `APP_ENV: local`.

Env mapping values may be YAML strings, numbers, or booleans. PV normalizes them to strings before rendering.

Env mapping values must be scalar. Arrays and objects are invalid.

Generated env keys must use uppercase shell-style names matching `^[A-Z_][A-Z0-9_]*$`.

PV does not provide default env mappings for Managed Resources in v1. Every generated `.env` key must be explicitly declared in Project config.

Example Project config:

```yaml
php: "8.4"

document_root: public

hostnames:
  - api.acme.test
  - admin.acme.test

env:
  APP_URL: "${project_url}"

mysql:
  version: "8.0"
  env:
    DB_HOST: "${host}"
  allocations:
    app:
      env:
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
        DB_PORT: "${port}"
    analytics:
      env:
        ANALYTICS_DB_DATABASE: "${database}"
        ANALYTICS_DB_USERNAME: "${username}"
        ANALYTICS_DB_PASSWORD: "${password}"
        ANALYTICS_DB_PORT: "${port}"

rustfs:
  version: "latest"
  env:
    AWS_ENDPOINT: "${endpoint}"
  allocations:
    uploads:
      env:
        AWS_BUCKET: "${bucket}"
        AWS_ACCESS_KEY_ID: "${access_key}"
        AWS_SECRET_ACCESS_KEY: "${secret_key}"
```

Any `env:` mapping in Project config is explicit opt-in to PV-managed `.env` rendering, including root-level `env:` without Managed Resource mappings.

When a Project opts in with environment mappings, the daemon reconciles the requested Managed Resources and updates only a PV-owned delimited block inside the Project's `.env` file. PV never rewrites user-owned `.env` lines outside that block. If the Project's `.env` file does not exist, PV may create it with the PV-owned block.

PV renders `.env` only after required Managed Resource ports and Resource allocations are known. Env rendering is all-or-nothing for the full Project config. If a required allocation or resource reconciliation fails, PV keeps the last valid managed block and records the failure instead of rendering incomplete values.

PV v1 only renders to `.env`. It does not support `.env.local` or configurable env file targets.

When creating a missing `.env`, PV creates a file containing only the PV-owned block. It does not copy `.env.example`.

PV uses these exact `.env` delimiters:

```env
# >>> PV MANAGED
APP_URL=https://acme.test
# <<< PV MANAGED
```

If the PV-managed block already exists, PV replaces only the content between the delimiters. The block is fully regenerated on each reconciliation; user edits inside the PV-managed block are overwritten.

PV appends the managed block at the end of `.env` and preserves surrounding formatting, including final newline, where practical.

During `.env` rendering, PV warns if generated env keys already exist outside the PV-managed block. It still writes the managed block, does not remove user-owned keys, and records the duplicate-key warning in observed state/logs.

Duplicate env key warnings appear as compact Project warnings in `pv list`, with details in `pv status` and logs. `pv project:env` also warns when duplicates exist, while still printing generated values.

PV writes `.env` values unquoted when safe and quotes/escapes values when necessary, such as values containing spaces, `#`, quotes, or newlines.

If a Project has no Project config, or its Project config has no environment mappings, PV does not touch the Project's `.env` file. If a previously generated PV-managed block exists, PV leaves the last generated values in place and stops updating the block. The Project is still served through the Gateway using the default PHP version, or the `php` version requested in Project config when present.

PV does not watch `.env` files. It only writes the PV-managed block during reconciliation when Project config or Managed Resource state requires an update.

PV v1 does not include `pv init`, does not create sample Project config files during setup, and does not create Project config during `pv link`.

## Project Linking

`pv link [path]` registers a Project as desired state and immediately requests daemon reconciliation.

If `path` is omitted, `pv link` uses the current directory.

The target path must exist and be a directory. `pv link` fails for missing paths or non-directory paths.

PV allows linking any directory. It is Laravel-first in defaults and UX, but `pv link` does not require Laravel/PHP detection. If a linked Project cannot be served, observed status reports the failure.

PV allows nested linked Projects. Current-directory resolution uses the nearest linked Project ancestor, and hostname uniqueness prevents routing ambiguity.

By default, PV serves the Project's `public/` directory when it exists, otherwise it serves the Project root.

When the selected document root contains `index.php`, PV uses front-controller-friendly PHP routing so clean URLs route through `index.php`. This is framework-friendly rather than Laravel-specific. PV v1 does not support Laravel Octane.

If the selected document root has no `index.php`, PV serves static files normally.

PV v1 does not manage Project background processes such as Laravel queues, scheduler, Horizon, Reverb, or similar long-running Project commands.

PV v1 does not run Project package manager commands, such as `composer install`, automatically.

PV v1 does not run Laravel application commands, such as `php artisan key:generate` or migrations, automatically.

PV v1 does not diagnose Laravel application state, such as missing `APP_KEY`.

The command succeeds when PV has recorded the desired Project and submitted the reconciliation request. If the daemon is running and reconciliation succeeds, the Project should be reachable at its `.test` URL by the end of the command.

`pv link` can run before `pv setup`. It records desired state and warns that setup is incomplete, so routing will not work until `pv setup` completes.

By default, PV derives the Project hostname from the Project directory basename, normalized to a DNS-safe slug. For example, `/Users/me/Code/Acme Store` becomes `acme-store.test`.

Project hostnames are normalized to lowercase and validated as DNS-safe `.test` hostnames. PV accepts and trims one trailing DNS dot, such as `acme.test.` to `acme.test`. Hostname uniqueness checks are case-insensitive because PV stores normalized lowercase hostnames.

`pv.test` is reserved for PV diagnostics or future internal UI and cannot be assigned to a Project.

PV identifies a Project by its canonical absolute path. Generated names and Project hostnames are unique attributes, but they are not Project identity. Running `pv link` more than once for the same canonical path updates the existing Project record.

PV stores both the original linked path string and the canonical absolute path. The canonical path is used for identity, routing, and equality; the original path is display/debug metadata.

PV also assigns each Project a random stable Project ID at first link and stores it in `pv.db`. The Project ID is PV's stable internal reference for the Project and does not change when the Project hostname changes. Project IDs should be short random URL-safe IDs, roughly 8-12 characters, to avoid local collisions while keeping diagnostics readable.

If a Project directory moves and is linked again at the new path, PV v1 treats it as a new Project. Path-move semantics may be added later if needed.

If a linked Project path no longer exists, PV marks the Project failed or missing in observed state but keeps desired state. PV does not auto-unlink missing Projects.

Missing Projects continue to own their Project hostnames until the user unlinks or repairs them. If technically feasible, the Gateway returns a specific missing-Project response for linked hostnames whose Project path is missing.

If the derived Project hostname is already assigned to another Project, `pv link` fails with a clear collision error. The user can pass an explicit Project hostname with `--hostname <hostname>` to resolve the collision.

`--hostname` accepts either a bare label or a full `.test` hostname. A bare label always normalizes to `<label>.test`; for example, `--hostname acme` and `--hostname acme.test` both normalize to `acme.test`. Multi-label hostnames must be provided in full, such as `api.acme.test`. PV v1 rejects non-`.test` hostnames.

Primary Project hostnames may contain multiple labels as long as they end in `.test`, such as `api.acme.test`.

If `pv link` is run for an already linked Project with a different `--hostname`, PV updates that Project's hostname after checking for collisions, then requests reconciliation.

Changing a Project's primary hostname triggers full Project reconciliation, including additional hostname validation, Gateway routing updates, certificate configuration updates, and PV-managed `.env` updates when the Project has opted into env rendering.

If `pv link` is run for an already linked Project with the same hostname, it is idempotent: PV refreshes desired state, requests reconciliation, reports that the Project was already linked, and exits successfully.

If `pv link` is run for an already linked Project without `--hostname`, PV preserves the existing Project hostname, including any previously configured custom hostname.

## Project Unlinking

`pv unlink` with no argument unlinks the Project resolved from the current directory, using the nearest linked Project ancestor rule.

`pv unlink <hostname>` unlinks the Project resolved by that hostname. The hostname argument accepts the same forms as `pv link --hostname`, so `acme` and `acme.test` both resolve to `acme.test`. Additional hostnames declared in `hostnames:` may also resolve the owning Project. Output should identify the owning primary Project hostname before unlinking.

`pv unlink <additional-hostname>` does not require confirmation in v1 because unlink is non-destructive, but output must make the resolved primary Project explicit.

`pv unlink` exits non-zero if the target cannot be resolved to a linked Project.

`pv unlink` removes the Project from desired state and requests reconciliation. It never deletes the Project directory. Resource allocations, databases, Redis data, RustFS data, and other managed service data remain unless a separate destructive command is introduced. PV stops reconciling and watching them for that Project.

If PV previously generated a Project `.env` block, `pv unlink` leaves the block in place and stops updating it.

## Opening Projects

`pv open [hostname]` opens a Project hostname in the user's browser from desired state. It does not require observed state to confirm that the Project is currently reachable.

With a hostname argument, `pv open <hostname>` opens that Project directly. With no argument, it opens the current Project or falls back to the picker.

When opening a Project without a hostname argument, PV opens the primary Project hostname even if the Project has additional hostnames.

The hostname argument accepts the same normalized forms as `pv unlink`, so `acme` and `acme.test` both resolve to `acme.test`.

If the hostname argument matches an additional hostname from a Project's `hostnames:`, `pv open` opens that exact hostname.

If no current Project can be resolved and the terminal is non-interactive, `pv open` exits non-zero unless a hostname argument was provided.

Current-directory Project resolution walks up from the current directory to linked Project roots, stopping at the filesystem root. If linked Projects are nested, PV chooses the nearest linked Project ancestor. This applies to commands that resolve a Project from the current directory.

`pv open` is Project-focused and does not open Managed Resource dashboards.

`pv mailpit:open` / `pv mail:open` opens the Mailpit dashboard only when Mailpit is already running. It does not start Mailpit or change desired state.

`pv rustfs:open` / `pv s3:open` opens the RustFS console only when RustFS is already running. It does not start RustFS or change desired state.

If `pv open` is run outside a linked Project, it shows a picker of linked Projects and opens the selected Project in the user's browser.

The picker displays each Project's primary hostname first, followed by its canonical absolute path. For example: `acme.test  /Users/me/Code/acme`. Additional hostnames are not separate picker entries in v1.

The picker sorts Projects by primary Project hostname.

## Listing Projects

`pv list` lists desired Projects and enriches them with observed status when available. At minimum, each row should include Project hostname, canonical absolute path, resolved PHP version, serving status, resource status, and env rendering status.

Status values use words such as `ok`, `pending`, `failed`, `degraded`, or `unknown` by default. TTY output may add color or icons, but words remain present.

If the daemon has not reconciled a Project yet, the Project still appears with pending or unknown observed status.

`pv list` shows the primary Project hostname by default and may show a compact indicator for additional hostnames, such as a count. Full additional hostname detail belongs in broader status/detail output.

`pv list` does not show Resource allocation details by default. It stays focused on Project-level serving status.

`pv list` sorts Projects by primary Project hostname by default.

## System Status

`pv status` reports whole-system PV status. It is not scoped to the current Project.

The status output should include daemon state, Gateway state, DNS resolver state, `pf` redirect state, CA trust, installed Managed Resources, and any failed or pending Projects. It should not duplicate the full `pv list` Project table.

`pv status` shows the log directory and a summary of the most recent daemon or reconciliation errors without dumping full logs by default.

`pv status` may show Managed Resource health and ports, but it must not print credentials or secrets.

`pv logs` shows daemon logs by default. Daemon logs include structured daemon logs plus LaunchAgent stdout/stderr logs so startup failures are visible. Flags may include `--follow`, `--gateway`, `--worker <php-track>`, and `--all` for broader log streams.

`pv logs` shows the last 100 lines by default. Users can change the number of lines with `-n <lines>`.

`pv logs -n` rejects negative values and caps the maximum at 5000 lines to avoid accidentally dumping huge logs into the terminal.

When showing the last N lines, `pv logs` includes recent rotated log files if the active log file has fewer than N lines.

`pv logs --follow` shows the last N lines first, then streams new lines. `pv logs --follow -n 0` streams only new lines.

`pv logs --follow` uses rotated files only for the initial last-N output. After startup, it follows active log files only.

When `pv logs --follow` streams multiple files, PV prefixes each line with the source, such as `daemon`, `launchd:stdout`, or `launchd:stderr`.

`pv logs` may colorize source prefixes when output is an interactive TTY. Color is disabled automatically when output is piped or `NO_COLOR` is set.

`pv logs --all --follow` includes every PV-owned log stream, including daemon, LaunchAgent, Gateway, Project-serving workers, and Managed Resource logs, with source prefixes.

`pv logs --gateway` shows both Gateway access and error logs by default. When following both streams, PV prefixes lines with sources such as `gateway:access` and `gateway:error`.

`pv logs --worker <php-track>` accepts explicit PHP tracks and `latest`. `latest` resolves to the manifest default PHP track. If the resolved track has no log file, PV prints a clear message that no logs exist for that PHP track.

`pv logs` supports Managed Resource log filtering with flags such as `--resource mysql --track 8.0`, matching the resource/track log layout.

If `pv logs --resource <name>` is used without `--track`, PV infers the track only when one track is installed for that Managed Resource. If multiple tracks are installed, PV requires `--track` and lists available tracks.

`pv logs --resource <name> --track latest` resolves `latest` to the manifest default track for that Managed Resource.

`pv logs --resource` accepts the same aliases as resource command namespaces, such as `pg`, `mail`, and `s3`, and normalizes them internally to canonical names: `postgres`, `mailpit`, and `rustfs`.

`pv doctor` is a deeper read-only diagnostic than `pv status`. It checks expected files, permissions, ports, resolver behavior, `pf` rules, LaunchAgent registration, manifest cache, and common conflicts, then suggests repair commands.

`pv jobs` is a read-only diagnostic command that lists recent daemon jobs, including setup, install, update, restart, and reconciliation jobs. It shows status, scope, start/end time, and failure summary. Live progress remains attached to the command that started the job.

Read/status commands support `--json` output in v1, including `pv status`, `pv list`, `pv project:env`, `pv jobs`, `pv update --check`, and Managed Resource list commands. Mutating progress-stream commands do not need JSON output in v1 unless it is cheap to provide.

`pv status --json` and broad status/list JSON outputs do not include secrets. `pv project:env --json` includes actual generated env values, including secrets, because it is the explicit Project env command.

`pv doctor` exits zero when all required checks pass and non-zero when any required check fails. Warnings do not fail the command if PV can still operate.

`pv status` exits non-zero for clear failure states such as daemon down after setup, Gateway failed, DNS or ports repair required, or failed reconciliation. It exits zero for healthy or pending-but-not-failed states.

If the daemon is intentionally disabled while DNS, ports, or CA integrations remain installed, `pv status` reports the daemon as `disabled` and PV as not running, but does not treat DNS, ports, or CA as broken. It suggests `pv daemon:enable` or `pv setup`.

# Commands

## CORE

| command                  | what it does                                                                                                      |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| pv link [path] [--hostname <hostname>] | Register a Project and request daemon reconciliation                                                    |
| pv open [hostname]       | Opens a Project in the browser                                                                                    |
| pv project:env [hostname] | Print generated Project environment values without editing `.env`                                                |
| pv list                  | List linked projects with which php version they using                                                            |
| pv logs [--follow]       | Show PV daemon/reconciliation logs                                                                                |
| pv status                | Show whole-system PV status                                                                                       |
| pv setup [--yes] [--non-interactive] [--no-path] | Configure macOS resolver, `pf` redirects, CA trust, daemon registration, and default Managed Resources |
| pv uninstall [--prune] [--force] | Uninstall PV, preserving data by default                                                                  |
| pv unlink [hostname]     | Unlink a Project by current directory or Project hostname                                                         |
| pv update [--check]      | Update the PV application and installed Managed Resources to their latest versions, or report available updates with `--check` |
| pv restart               | Restart PV-managed runtime processes and reconcile desired state                                                   |
| pv env [--shell <shell>] | Print shell exports for PV-managed binaries and Composer environment                                              |
| pv completions <shell>   | Generate shell completions                                                                                        |

## Daemon

Run pv as a background LaunchAgent that starts on login. This daemon is responsible for orchestrating all PV-managed processes.
| command | what it does |
| --- | --- |
| pv daemon:disable | Disable the pv login daemon |
| pv daemon:enable | Enable pv as a login daemon (starts on boot) |
| pv daemon:restart | Restart the pv daemon |

## Diagnostics

| command   | what it does                                                                                  |
| --------- | --------------------------------------------------------------------------------------------- |
| pv doctor | Run read-only diagnostics for setup, DNS, ports, CA, daemon, Gateway, manifest cache, conflicts |
| pv jobs [--json] | List recent daemon jobs and their final status                                           |

## CA

| command       | what it does                                        |
| ------------- | --------------------------------------------------- |
| pv ca:status  | Show pv local CA trust status                       |
| pv ca:trust   | Trust pv's local CA in the macOS System keychain    |
| pv ca:untrust | Remove pv's local CA from the macOS System keychain |

## DNS

| command          | what it does                                      |
| ---------------- | ------------------------------------------------- |
| pv dns:status    | Show PV `.test` resolver configuration status     |
| pv dns:install   | Install or repair `/etc/resolver/test`            |
| pv dns:uninstall | Remove PV's `/etc/resolver/test` configuration    |

## Ports

| command            | what it does                                          |
| ------------------ | ----------------------------------------------------- |
| pv ports:status    | Show PV `pf` redirect status for loopback `80`/`443`  |
| pv ports:install   | Install or repair PV's `pf` redirect rules            |
| pv ports:uninstall | Remove PV's `pf` redirect rules                       |

## PHP + Frankenphp

| command                    | what it does                                                                  |
| -------------------------- | ----------------------------------------------------------------------------- |
| pv php:install [version]   | Install a PHP track (e.g., pv php:install 8.4). Uses the manifest default track if omitted. |
| pv php:default <version>   | Set the global default PHP track                                             |
| pv php:update              | Update all installed PHP versions                                             |
| pv php:uninstall <version> [--prune] | Uninstall a PHP track.                                                             |
| pv php:list                | List installed PHP tracks                                                     |

## Composer

| command               | what it does                                    |
| --------------------- | ----------------------------------------------- |
| pv composer:install   | Install Composer track `2`                      |
| pv composer:uninstall [--prune] | Remove Composer PHAR and shim. Preserve Composer home/cache unless `--prune` is provided. |
| pv composer:update    | Update Composer track `2` to the latest non-revoked artifact |

## Postgres (Alias: pg)

| command                                 | what it does                                                                      |
| --------------------------------------- | --------------------------------------------------------------------------------- |
| pv {postgres or pg}:install [version]   | Install a Postgres track. Uses the manifest default track if omitted. |
| pv {postgres or pg}:uninstall <version> [--prune] | Uninstalls a postgres version.                                          |
| pv {postgres or pg}:update              | Update all installed Postgres tracks                                              |
| pv {postgres or pg}:list                | List installed Postgres tracks                                                    |

## Mysql

| command                      | what it does                                                                   |
| ---------------------------- | ------------------------------------------------------------------------------ |
| pv mysql:install [version]   | Install a MySQL track. Uses the manifest default track if omitted. |
| pv mysql:uninstall <version> [--prune] | Uninstalls a mysql version.                                             |
| pv mysql:update              | Update all installed MySQL tracks                                               |
| pv mysql:list                | List installed MySQL tracks                                                     |

## Mailpit (Alias: mail)

| command                                  | what it does                                                                     |
| ---------------------------------------- | -------------------------------------------------------------------------------- |
| pv {mailpit or mail}:install [version]   | Install a Mailpit track. Uses the manifest default track if omitted. |
| pv {mailpit or mail}:uninstall <version> [--prune] | Uninstalls a mailpit version.                                          |
| pv {mailpit or mail}:update              | Update all installed Mailpit tracks                                              |
| pv {mailpit or mail}:list                | List installed Mailpit tracks                                                    |
| pv {mailpit or mail}:open                | Open the running Mailpit dashboard                                               |

## Rustfs (Alias: s3)

| command                               | what it does                                                                    |
| ------------------------------------- | ------------------------------------------------------------------------------- |
| pv {rustfs or s3}:install [version]   | Install a RustFS track. Uses the manifest default track if omitted. |
| pv {rustfs or s3}:uninstall <version> [--prune] | Uninstalls a rustfs version.                                          |
| pv {rustfs or s3}:update              | Update all installed RustFS tracks                                               |
| pv {rustfs or s3}:list                | List installed RustFS tracks                                                     |
| pv {rustfs or s3}:open                | Open the running RustFS console                                                 |

## Redis

| command                      | what it does                                                                   |
| ---------------------------- | ------------------------------------------------------------------------------ |
| pv redis:install [version]   | Install a Redis track. Uses the manifest default track if omitted. |
| pv redis:uninstall <version> [--prune] | Uninstalls a redis version.                                             |
| pv redis:update              | Update all installed Redis tracks                                               |
| pv redis:list                | List installed Redis tracks                                                     |
