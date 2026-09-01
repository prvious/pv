# PV

PV has laravel style commands where commmands under the same category/famility are prefixed with the namespace. eg: 'pv php:update'

`pv` is a Laravel-first local desired-state control plane (More framework support to come later when laravel is stable).

`pv` gets you a complete local PHP environment in one shot:

- Caddy — the standalone Gateway for `.test` routing and TLS
- FrankenPHP — the Project-serving PHP worker runtime, managed per PHP track
- PHP — managed per-version, no homebrew/apt needed
- Mysql, Postgresql, Redis, Composer, Mailpit, Rustfs all Ready to go

Per-project versions are supported too — add a pv.yml file with php: "8.4" in your project root. Multiple PHP versions run simultaneously, with Projects routed through workers grouped by PHP runtime identity.

## High-Level Features

- Control-plane foundation.
- Machine-owned store(Sqlite), filesystem layout, and migration guardrails.
- Daemon and resource-agnostic supervisor.
- Managed resources: Caddy, Mailpit, Postgres, MySQL, Redis, RustFS, and more to come...
- Standalone Caddy Gateway .test HTTPS routing and `pv open`.
- Status UX across desired and observed state.

## Platform Scope

PV v1 supports macOS 14 and newer. Stabilizing the macOS application remains the immediate product priority.

macOS 13 may continue to run PV when the application and Managed Resource binaries remain compatible, but it is untested and unsupported. Dropping support does not by itself require raising binary deployment targets or republishing otherwise compatible Managed Resource artifacts. Before PV deliberately ships an application binary that cannot run on macOS 13, the application update manifest and updater must prevent an incompatible update from being activated there.

The full macOS CI quality and behavior suite covers every supported macOS major version and both supported architectures across a representative matrix rather than every version/architecture combination. The initial matrix is macOS 14 on Apple Silicon, macOS 15 on Intel, and macOS 26 on Apple Silicon. Private-interface acceptance tests such as listener inspection run as part of every matrix lane. New supported macOS major versions must be added to the matrix before PV relies on behavior there.

Linux and Windows are committed subsequent platforms. During macOS stabilization, the installed application and runtime crates compile natively on macOS, Linux, and Windows so new system boundaries do not create unnecessary portability blockers.

Native compile support and explicit unsupported behavior do not make Linux or Windows supported distributions, add them to v1, or authorize publishing Linux or Windows binaries.

The design may use macOS-specific primitives where they materially improve the v1 experience, including launch agents for daemon startup and the macOS System keychain for local CA trust.

PV uses a host platform boundary in the Rust workspace. Application crates such as `cli`, `daemon`, `state`, `config`, and `resources` should not depend directly on macOS implementation APIs. The v1 concrete host platform implementation is macOS-only; unfinished Linux and Windows implementations return explicit unsupported results, and app-facing code should call the `platform` crate for host integration concerns.

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

Current command help is generated by `clap`. Public commands accept `--help`, include the global `--no-color` flag in help, and show supported command-specific flags. Hidden internal commands such as `daemon:run` stay out of normal top-level help.

PV respects the `NO_COLOR` environment variable and supports a global `--no-color` flag for deterministic plain output across commands.

PV uses `rusqlite` for SQLite access to `pv.db`. Database work is local and transactional, so synchronous queries are acceptable; daemon paths can use `spawn_blocking` when needed to avoid blocking Tokio runtime tasks.

PV embeds SQLite migrations into the Rust binary and runs them automatically before accessing `pv.db`. Migrations run transactionally and are tracked in a machine-owned migrations table.

PV database migrations must remain backward-compatible with the immediately previous PV application version so self-update rollback can safely restore the previous binary without restoring `pv.db`. Destructive or incompatible schema cleanup should be delayed until a later release after the previous binary no longer needs to read the old shape.

Before applying migrations, PV creates a timestamped `pv.db` backup such as `~/.pv/pv.db.20260522-143012.bak`. PV keeps migration backup retention simple, such as the last 5 backups.

If a `pv.db` migration fails, PV refuses to run commands that depend on `pv.db`, reports a clear migration error, and points to logs. PV does not continue against partially migrated state.

PV does not automatically roll back from migration backups. Transactional migrations should leave the database unchanged on failure; backups exist for manual recovery and diagnostics.

Managed Resources remain external binaries/artifacts managed by PV rather than Rust code embedded into the PV binary.

Initial PV distribution is a standalone install script/direct binary download. Homebrew support can be added after the release flow stabilizes. A signed `.pkg` is deferred unless macOS trust/onboarding requires it.

The install script downloads the PV application and its separate privileged helper artifact, verifies each against its published SHA-256 checksum, and installs both plus the required adjacent `pv-helper.json` release metadata into the user-owned release directory. Setup and update require that metadata rather than relying on a checksum compiled into the application. If either verification fails, installation deletes the bad download and stops before editing shell profiles or running setup. Dogfood helper artifacts use ad-hoc code signing; Developer ID signing, notarization, and a signed package are deferred until public release.

The stable installer URL serves a generated installer script based on PV app release metadata. The bash installer does not need to parse the JSON PV app update manifest. The installer script may embed or otherwise receive the resolved current PV version, platform asset URLs, and SHA-256 checksums from the server-side installer generation flow. The JSON PV app update manifest is used by the Rust self-updater.

The generated installer script installs the current stable PV release only in v1. Installing a specific historical PV version through the installer is out of v1 scope.

PV v1 has one stable installer/update channel for both the generated installer and `pv update`. Installer channel query parameters such as `?channel=preview`, nightly channels, and multi-channel update selection are out of v1 scope.

By default, the install script installs `pv` and `pv-helper` under `~/.pv/bin/releases/<version>/`, creates or updates the active `~/.pv/bin/pv` symlink, asks once through the controlling terminal for consent to run setup, and then invokes `pv setup --yes`. That accepted installer confirmation covers PV's shell-profile, privileged-helper, and Managed Resource setup changes but does not bypass macOS authentication. `--yes` skips the installer confirmation for automation. `--non-interactive` implies PV confirmations are accepted, disables all prompts, and fails if input, helper lifecycle authentication, or shell profile confirmation is required. A `--no-setup` flag installs both user-owned release binaries without registering the root helper. A `--no-path` flag is forwarded to setup and skips automatic shell profile edits.

`pv setup`, including when invoked by the installer, may create or repair PV shell integration in the user's shell profile with a clearly delimited PV-managed shell block. Shell profile edits must be idempotent and backed up first. If shell detection fails, setup prints manual shell integration instructions instead of editing profile files.

When automatic setup is enabled, the installer invokes setup through the absolute `~/.pv/bin/pv` path in the current process. The installer does not implement a second shell-profile editing path.

Setup reports shell-profile failures directly and provides manual integration guidance when shell detection cannot choose a supported profile.

If shell detection finds an unsupported shell, setup skips shell profile edits, prints manual shell integration instructions, and continues. Unsupported shell integration does not block DNS, ports, CA trust, daemon registration, or Managed Resource installation.

Setup detects the user's shell from `$SHELL`. If `$SHELL` is missing or unsupported, setup skips profile edits and prints manual shell integration instructions.

Shell profile backups created by setup append a timestamp and `.pv.bak`, such as `~/.zprofile.20260522-143012.pv.bak`.

Setup edits only the detected shell's profile file for PATH setup: `~/.zprofile` for zsh, `~/.bash_profile` for bash, and `~/.config/fish/config.fish` for fish. It does not edit multiple shell profile files at once.

If the detected shell profile file does not exist, setup may create it with only the `PV ENV` block after confirmation. No backup is needed when creating a new file, but the action is reported.

Setup uses `PV ENV` delimiters for shell profile edits. The PV-managed block loads `pv env` so PV shims and Composer work in new shells. It calls the PV binary by absolute path and passes an explicit `--shell <shell>` for the detected profile so shell startup works even before `~/.pv/bin` has been added to PATH and does not rely on runtime shell detection. For POSIX-style shells, the block is:

```sh
# >>> PV ENV
if [ -x "$HOME/.pv/bin/pv" ]; then
  eval "$("$HOME/.pv/bin/pv" env --shell zsh)"
fi
# <<< PV ENV
```

Fish uses equivalent syntax with the same `PV ENV` delimiter labels.

`pv setup` may repair a stale PV-managed `PV ENV` shell profile block, but only after confirmation because shell profiles are user-owned. `pv setup --yes` consents to this repair without prompting. `pv setup --non-interactive` fails instead of prompting for confirmation, shell profile repair, or privileged-helper lifecycle authentication. `pv setup --no-path` disables shell profile edits, including stale `PV ENV` block repair, but still prints manual shell integration instructions. When repairing the `PV ENV` block, PV replaces the block wholesale and does not preserve user edits inside it.

Setup does not try to source the updated shell profile into the current parent shell. After editing, it tells the user to open a new terminal or run the shown `pv env` command for the current session.

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

### PF Health and Recovery

PV reports one canonical low-port routing state across readiness and diagnostics: `active`, `inactive`, `drifted`, or `unknown`. `active` means the expected redirects to the currently assigned Gateway ports were verified. `inactive` means authoritative inspection verified that no loaded redirect targets either assigned Gateway backend port. `drifted` means prepared files, installed system files, or loaded rules differ from the current expected redirect configuration, except when loaded-rule inspection proves the redirects are fully absent and therefore `inactive`. `unknown` is reserved for a current-looking file configuration whose loaded state cannot be inspected and whose behavior cannot be verified end to end. Prepared or installed files alone never prove that redirects are active.

An unprivileged successful `pfctl` inspection is authoritative for loaded-rule state, with an exhaustive classification order. Confirmed absence of any loaded redirect targeting the assigned Gateway backend ports produces `inactive`, even when prepared or installed files are missing or stale. Otherwise, exact expected rules with current prepared and installed files produce `active`; exact rules with non-current files and all differing or partial loaded-rule configurations produce `drifted`. Therefore stale files plus exact loaded rules are `drifted`, while stale files plus confirmed complete loaded-rule absence are `inactive`. Background and read-only diagnostics do not invoke `sudo`, including noninteractive `sudo`; failure or denial from unprivileged `pfctl` falls back to end-to-end probes instead of being reported as inactive.

The fallback probe sends short, bounded requests through both public loopback ports `80` and `443` and must verify that they reached the current PV Gateway, using a PV-owned response identity and the expected TLS identity where applicable. A generic TCP connection is insufficient. When prepared and installed PV configuration is current, successful HTTP and HTTPS probes provide authoritative behavioral evidence for `active`. If either probe is inconclusive or fails while loaded rules cannot be read, the state is `unknown`. When any prepared or installed file is non-current, the state is `drifted` whether the probes succeed or fail. A failed probe without readable loaded-rule evidence never proves `inactive`.

Gateway readiness depends on that state. For `active`, readiness uses the public ports so the check exercises the real redirect path. For verified `inactive`, direct high-port readiness is allowed only after loaded-rule inspection also confirms that no active redirect targets either assigned backend port; low-port routing remains repair-required even if the Gateway process itself is ready. For `drifted`, readiness uses only bounded public probes because partially active rules may still target a backend port. A successful public probe may verify the Gateway runtime, but the integration remains `drifted`. For `unknown`, readiness retries the bounded public probes; success reclassifies the state as `active`, while failure preserves `unknown`. `drifted` or `unknown` readiness never falls back to direct backend connections and never turns routing uncertainty alone into a Gateway stop/restart loop. PV keeps an owned Gateway process running and records low-port repair-required state.

macOS updates or restarts may unload active rules while leaving PV-owned prepared and system files intact. Readable loaded-rule evidence classifies this as `inactive`; unavailable inspection plus failed public probes classifies it as `unknown`. Both cases direct the user to foreground `pv ports:install`. The daemon and periodic health tick may inspect files, run unprivileged `pfctl`, and perform bounded probes, but they never prompt, call `sudo`, reload `pf`, or mutate privileged files.

`pv ports:install` remains the only focused repair path. It uses the installed typed helper and never invokes `sudo` or prompts directly; `pv setup` may first use foreground sudo to install or repair the helper lifecycle. It reloads the PV-owned rules, then verifies the exact loaded rules and public HTTP/HTTPS behavior before reporting success. If the running Gateway passes those probes, PV records a fresh healthy Gateway observation. If the Gateway is not running or cannot yet be verified, PV invalidates only stale PF-derived Gateway readiness observations to `pending` and requests reconciliation; it does not clear unrelated Gateway config, process, or TLS failures. `pv setup` may perform the same foreground repair as part of setup.

`pv status`, `pv doctor`, and `pv ports:status` use the same four state values and the same `pv ports:install` repair advice. Their JSON forms expose the stable lowercase `state` value plus evidence (`pfctl`, `probe`, or `unavailable`), expected redirect ports, any readable active ports, and the observation timestamp; they do not infer `active` from file state. Plain output uses the same words. `pv ports:status` exits zero only for `active` and non-zero for `inactive`, `drifted`, or `unknown`. `pv doctor` treats every non-active state as a failed required check after setup. `pv status` treats a non-active state as a failure whenever low-port routing is required, while preserving the intentional-daemon-disabled behavior where installed integrations are reported but not considered broken.

A friendly browser page for unknown Project hostnames is useful, but is post-v1 polish rather than required v1 scope.

## Setup

`pv setup` performs PV's required macOS bootstrap steps:

- Create `/etc/resolver/test` so macOS sends `.test` lookups to PV's internal DNS resolver.
- Install macOS `pf` redirect rules so loopback ports `80` and `443` reach the unprivileged Gateway.
- Trust PV's local CA in the macOS System keychain.
- Register the PV daemon as a per-user `launchd` LaunchAgent with `KeepAlive` so macOS starts it after login and can restart it after crashes.
- Start the PV daemon immediately after registration.
- Record desired state for the default Managed Resource versions, then request daemon reconciliation.

The default setup install set includes Caddy track `2` independently, the manifest default PHP/FrankenPHP pair, MySQL, PostgreSQL, Redis, Mailpit, and RustFS, plus Composer track `2`. Downloads should run in parallel where possible. `pv setup` does not install every track listed in the manifest.

For PHP tracks, setup/install installs both standalone PHP artifacts for CLI/PATH shims and FrankenPHP artifacts for Project-serving workers. Caddy is a separate core resource and is not part of a PHP track pair.

`pv php:install <track>` installs both standalone PHP and FrankenPHP artifacts for that PHP track.

Default Managed Resources installed by `pv setup` are not started until a linked Project needs them. The standalone Caddy Gateway and DNS resolver are core PV infrastructure and run even when no Project needs a backing Managed Resource.

Default tool/resource installation is owned by the daemon. `pv setup` records desired install state, starts the daemon, requests reconciliation, and waits for that reconciliation job to finish.

One-off CLI commands communicate with the daemon through a Unix domain socket at `~/.pv/run/pv.sock` using newline-delimited JSON messages. Each request is one JSON line and includes a daemon protocol version field. The immediate response is one JSON line. For long-running work, the daemon then emits best-effort NDJSON progress snapshots over the same connection. Event types include `job_started`, `progress`, `download_progress`, `log`, `job_completed`, and `job_failed`. `pv setup` listens to the progress stream, renders download bars only when stdout is a terminal, clears those bars before final command output, and exits when the reconciliation job completes or fails.

If the CLI and daemon protocol versions are incompatible, commands print a clear repair command such as `pv daemon:restart` rather than automatically restarting the daemon. `pv update` may handle daemon restart explicitly as part of the update flow.

After completing system setup, `pv setup` reconciles already-linked Projects so Projects linked before setup can become reachable.

Default tool/resource installation allows partial success. The daemon installs what it can, records and reports failures, causes `pv setup` to exit non-zero if any default install failed, and keeps setup safe to rerun to repair missing pieces.

Setup may require an admin prompt for system-owned configuration, but the PV daemon runs as the logged-in user and should not need to run as root.

`pv setup` is idempotent and repair-oriented. It verifies resolver configuration, CA trust, and LaunchAgent registration, fixes anything missing or stale, and prints what changed.

`pv setup` creates the required base directory structure under `~/.pv`, including `bin/`, `run/`, `logs/`, `downloads/`, `config/`, `certificates/`, `composer/`, and `resources/`, with correct permissions before starting the daemon or installing Managed Resources.

`pv setup` may edit the user's shell profile only for the PV-managed `PV ENV` shell integration block. `--yes` accepts the PV-owned edit, `--non-interactive` fails if an edit would be required, and `--no-path` skips shell profile integration. Setup does not silently modify shell profiles.

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

Crash-loop failures are scoped to the affected runtime. A failed Managed Resource track degrades only Projects that need that resource track. A failed PHP runtime worker affects only Projects on that PHP runtime identity. Gateway failure is system-wide because all Project routing depends on it.

Backing Managed Resource failures do not remove Project routes. PV keeps serving the web app in a degraded state when the Gateway and PHP runtime worker are healthy.

PV writes pid files under `~/.pv/run/` for the Gateway, Project-serving workers, and Managed Resource tracks. After daemon restart, PV may use these pid files to discover existing PV-owned child processes, but it must verify ownership before acting by checking the process command/path matches the expected PV-managed binary and config. PV never kills a process based on PID alone.

PV writes a small JSON runtime metadata file next to each pid file with the expected binary path, config path, resource name, track, and start time. Runtime metadata supports ownership verification and diagnostics, while `pv.db` remains the source of truth.

PV writes pid and runtime metadata files atomically by writing temporary files and renaming them into place after the child process starts successfully.

PV uses resource-specific readiness checks after starting child processes instead of treating a running PID as ready. The Gateway follows the state-selected readiness policy in PF Health and Recovery: public identity probes for `active`, `drifted`, and `unknown`, and direct high-port probes only for verified `inactive`. Other examples include the internal DNS resolver answering a `.test` query, MySQL/Postgres accepting connections, Redis responding to ping, and Mailpit/RustFS responding on their HTTP ports.

PV uses a default 15-second readiness timeout per child process, with resource-specific overrides only if a Managed Resource consistently needs longer. If readiness fails, PV marks that runtime failed or degraded and includes the readiness failure in observed state and logs.

If a pid file points to no process, or to a process that fails PV ownership verification, PV ignores it for control purposes and removes the stale pid file.

After daemon restart, PV adopts already-running PV-owned child processes when ownership verification passes and their binary/config/version match desired state. Reconciliation restarts processes that are stale, mismatched, or no longer desired.

PV starts each supervised runtime in its own process group so it can stop the entire PV-owned process tree safely. PV only signals a process group after ownership verification.

PV stops child process groups with graceful termination first, waits up to 10 seconds, then force-kills only PV-owned process groups that do not exit within that timeout.

The health tick may check privileged integrations such as `/etc/resolver/test` and `pf` rules read-only. It records repair-required status but does not prompt or mutate privileged system config. The health tick does not refresh the remote artifact manifest or make routine background network calls.

The daemon watches linked Project config files and automatically reconciles when they change.

On startup, the daemon detects privileged system drift such as stale DNS resolver ports or stale `pf` redirects, but it does not prompt for admin privileges or mutate system configuration from the background. It records repair-required observed status instead.

If a linked Project config becomes invalid, the daemon keeps the last valid served or resource-only desired state, records the config error in observed status, stops updating the configured env file from the invalid config, and surfaces the error in `pv list` and `pv status`. PV does not tear down working routes or resources because of a transient invalid edit.

Project config changes update only affected runtime processes. Config-only changes load the affected Caddy gateway or FrankenPHP worker through its admin API; PHP version or routing changes may replace or reassign FrankenPHP serving for the affected Project. Env-only changes update the configured env file without restarting the Gateway.

When a Project's PHP track or optional extension set changes, PV reconfigures only affected Project-serving workers. It may stop an old PHP worker if no Projects remain on that runtime identity and start a new worker for the changed identity. Unrelated PHP workers are not touched.

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

Privileged repair happens only from foreground commands such as `pv setup`, `pv dns:install`, and `pv ports:install`. PV mutates stored port choices in `pv.db` only after the corresponding privileged system configuration repair succeeds. The daemon may report drift but does not ask the helper to mutate system state.

PV uses a separate minimal `pv-helper` executable registered as the system launchd job `com.prvious.pv.helper`. Launchd creates the restricted `/var/run/com.prvious.pv.helper.sock` and activates the helper on demand. Once activated, the helper serves sequential connections until launchd stops it, avoiding launchd restart throttling between related operations. Its LaunchDaemon writes startup and uncaptured child-process diagnostics to `/var/log/com.prvious.pv.helper.err.log`. The daemon, Gateway, DNS resolver, and all Managed Resources remain unprivileged. The helper supports one installing macOS account per machine. Helper removal is restricted to the original installing UID, so that account must remove it before setup by a replacement account; an unavailable owner requires manual administrator recovery.

The helper accepts only versioned typed requests for status, DNS inspection/apply/removal, PF inspection/apply/reload/removal, and CA trust inspection/apply/removal. Requests cannot provide commands, executable paths, destination paths, raw configuration, environment behavior, or network operations. DNS and PF content is generated internally for fixed system destinations. `CaApply` reads only the installing account's fixed PV CA path, validates PV CA metadata and the requested fingerprint, stages that exact certificate in the fixed root work path, revalidates it, and trusts it. The helper revalidates ownership, conflicts, arguments, and expected state before each mutation. `pv.db` remains the only desired-state source of truth; root-owned helper metadata contains only the installing UID, helper version, and protocol version.

The socket is restricted to the installing account, and the helper verifies the Unix peer UID before decoding and dispatching a bounded request. Protocol version is validated independently from app and helper versions. Install and replacement readiness uses a small protocol-neutral lifecycle probe, separate from the versioned operational request schema, so an app may verify a newly installed helper before activating a matching protocol update. Missing or incompatible helpers produce repair guidance to run `pv setup`; `pv doctor` reports cross-account authentication failures with guidance to use the original installing account or perform manual administrator recovery. Normal DNS, PF, and CA commands never fall back to `sudo`.

`sudo` is used only by foreground helper lifecycle operations: initial install, replacement, repair, and removal. Initial setup therefore requires one administrator authentication, while later typed DNS, PF, and CA repairs continue without new prompts after sudo timestamp expiry or reboot. The helper installation path verifies the candidate checksum and ad-hoc code signature before replacing the root-owned executable and launchd registration.

Helper install, replacement, and removal hold a persistent-file advisory lock under `/Library/PrivilegedHelperTools` for the full root-owned lifecycle transaction. This machine-wide lock serializes the fixed candidate, rollback, executable, metadata, plist, and launchd paths across macOS accounts. Setup, update, and uninstall also hold a persistent per-account lock at `~/.pv-helper-lifecycle.lock` across helper-dependent integration and state changes; keeping it outside `~/.pv` lets `pv uninstall --prune` remain serialized while removing that tree.

`pv doctor` remains unprivileged and read-only. It reports helper availability, helper version, and helper protocol, uses narrowly scoped helper inspections when direct DNS, PF, or CA inspection is blocked, and continues reporting all other checks when the helper is unavailable.

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

`pv project:env` prints the generated Project environment values PV would render into the PV-managed block, without editing the configured env file. With no argument, it resolves the current directory's Project. With a selector argument, it resolves an exact Project slug or Project hostname, including additional hostnames declared in `hostnames:`. Bare slug/normalized-hostname ambiguity fails and suggests the full `.test` hostname. It prints actual rendered values, including secrets. Broad status commands should avoid printing secrets.

## Multi-version PHP

The Gateway is a core Managed Resource role implemented by a PV-managed standalone Caddy track `2`, not an HTTP server implemented inside the PV daemon. The PV daemon provisions a Caddy Gateway that listens on high loopback ports; macOS `pf` redirects external loopback ports `80` and `443` to the Gateway.
Projects using a different PHP runtime are proxied to secondary FrankenPHP worker processes running on high ports.

The Gateway is always-on core PV infrastructure after setup. It only routes/proxies and does not serve Projects directly. Runtime-specific Project-serving FrankenPHP processes run only when at least one linked Project needs that PHP runtime identity.

Each Project-serving FrankenPHP worker serves all Projects assigned to one PHP runtime identity. The runtime identity is the resolved PHP track plus the sorted available optional extension set. PV does not run one worker per Project.

Resource-only Projects do not contribute Gateway routes or Project-serving worker demand. The core Gateway remains running and healthy when every linked Project is resource-only. A successful served-to-resource-only transition removes stale generated route and worker config for that Project.

Project-serving FrankenPHP workers bind only to loopback high ports. They are internal to PV behind the Gateway.

The Gateway terminates TLS for Project hostnames. Project-serving FrankenPHP workers receive proxied plain HTTP traffic over loopback high ports.

The Gateway supports HTTP and HTTPS for Project hostnames. HTTP requests redirect to HTTPS by default.

When proxying to Project-serving workers, the Gateway preserves the original `Host` header and sets forwarding headers such as `X-Forwarded-Host`, `X-Forwarded-Proto`, and `X-Forwarded-For`.

PV generates a Gateway root config that imports per-Project generated config files. Splitting Project config keeps debugging easier and reduces config-generation blast radius.

For each PHP runtime identity, PV generates a worker root config that imports per-Project generated config files for Projects on that runtime.

### Gateway and worker reload contract

PV gives the standalone Caddy Gateway and every FrankenPHP worker a deterministic Unix-domain admin socket under `~/.pv/run/`. The Gateway uses `gateway-admin.sock`; workers use `worker-admin-<runtime-hash>.sock`, where the hash is derived from the full PHP runtime identity so socket paths remain short. Admin endpoints are not allocated TCP ports and are not persisted in `pv.db`. The state migration removes obsolete Gateway and worker admin-port rows.

Every generated Gateway and worker root Caddyfile includes the following global options, using its own absolute socket path:

```caddyfile
admin "unix/<absolute-socket-path>|0600"
persist_config off
```

The `~/.pv/run/` directory is owner-only (`0700`), and Caddy creates each admin socket as owner-only (`0600`). After connecting, the daemon verifies the peer process group against the managed runtime's root PID on that same socket before sending any HTTP bytes. There is no TCP admin fallback. A recorded runtime whose active config does not use its desired Unix socket, including a pre-migration TCP or `admin off` runtime, is stopped and replaced before any admin request.

PV-generated config files and `pv.db` remain authoritative. Caddy must not create a competing autosave config. The daemon reloads a runtime only by sending the promoted active root Caddyfile, byte-for-byte including its trailing newlines, as a whole-config `POST /load` request with `Content-Type: text/caddyfile` to that process's admin socket. Unix reload signals and `caddy reload` commands are not part of PV.

When no matching PV-owned process exists, PV validates the candidate with the correct managed binary, promotes the active config, starts the process, and requires both admin readiness through `GET /config/` and the existing Gateway or worker readiness checks before committing the config. A process replacement is allowed when the process is absent or its recorded runtime identity/spec is obsolete; it is not a reload fallback.

After a config is accepted and runtime readiness succeeds, deleting the previous root and fragment backups is post-commit housekeeping. PV attempts both deletions independently. A cleanup failure leaves the promoted config and successful runtime in place, emits one structured warning containing every failed backup path, and does not trigger rollback, a compensating load, or a process stop.

When a matching PV-owned process exists, PV verifies ownership immediately before the admin request. Before sending `POST /load`, PV atomically marks that exact runtime metadata as replacement-required; a definitive response clears the marker. A rejected or unavailable `POST /load` relies on Caddy's atomic load contract to keep the previous in-memory config active. PV restores the previous generated root and fragments on disk, keeps the owned process and PID running, records the typed failure, and never signals or restarts that process as a fallback.

When the client cannot determine whether a `POST /load` is still executing or was accepted, PV does not issue a competing compensating load because the final request ordering cannot be established. PV keeps the promoted desired root and fragments on disk, preserves the owned process, leaves its replacement-required marker set, and records the typed unknown-outcome failure without claiming which config is active. A later reconciliation never sends another load to that process: it stops the verified marked process, waits for its process group and pending handlers to exit, then starts a replacement from the latest desired config. The replacement establishes a new process epoch before reconciliation can report success, so an older unresolved load cannot overtake newer desired state.

After a successful load, PV runs the existing runtime readiness checks. If a real post-load readiness failure requires rollback, PV restores the previous files, re-verifies ownership of the same process, loads the restored previous root through the same admin endpoint, and verifies the previous readiness. PV returns the original failure after a successful rollback. If restored-config load or readiness fails, PV returns a compound failure containing both errors and does not claim that the prior runtime was restored. If ownership changes or the process exits, PV restores disk state but does not contact an unverified process. A PF `drifted` or `unknown` result handled by the existing `PreserveRuntime` policy is recorded as degraded routing state and does not by itself roll back a Caddy-accepted config.

When PHP runtime worker config changes, PV uses this admin API contract for the matching worker; it does not fall back to a signal or restart. A worker for a new PHP runtime identity is started as a new process, and an obsolete worker is replaced through normal lifecycle reconciliation.

Project-serving worker logs are captured per PHP runtime identity, with Project hostname included in access logs where feasible. PV v1 does not create per-Project log files. FrankenPHP worker Caddyfile logging directives should be used where practical.

Project-serving worker logs are split by PHP runtime identity, such as `~/.pv/logs/workers/php-8.4.log` or `~/.pv/logs/workers/php-8.4+redis.log`, because one worker serves all Projects assigned to that runtime.

Gateway access logs are enabled by default, stored locally under `~/.pv/logs/`, and rotated. Gateway logs are split into access and error logs, such as `~/.pv/logs/gateway/access.log` and `~/.pv/logs/gateway/error.log`, using standalone Caddy logging. Caddy's inherited stdout and stderr are appended to `~/.pv/logs/gateway/supervisor.log` so supervisor diagnostics do not mix with Caddy's error stream. Structured/JSON logs should be used when the respective Caddy or FrankenPHP runtime supports them cleanly.

When routing or Gateway config changes, PV loads the Gateway config through the Caddy admin API contract above. A matching owned Gateway is kept running on load rejection or API unavailability; only an absent or obsolete process may be replaced.

PV owns one local CA and passes that CA to the standalone Caddy Gateway configuration. Caddy generates and manages Project certificates from that CA as needed for hostnames in PV's desired routing table: primary Project hostnames plus additional `hostnames:` from valid Project config. The Gateway selects certificates by SNI.

While a Project is resource-only, PV retains any existing stable Project TLS files but does not refresh them or include the Project in Gateway TLS demand. Re-enabling serving resumes normal TLS reconciliation after hostname and document-root validation succeeds.

The Gateway does not automatically route `*.project.test` to a Project. Subdomain routing must be explicitly requested in Project config, which allows `acme.test` and `api.acme.test` to belong to different Projects.

For unknown `.test` hostnames, the Gateway should return a simple self-contained HTML response explaining that no PV Project is linked for the hostname and suggesting `pv link` when technically feasible.

PV supports Project-level PHP extension opt-ins without named profiles, local compilation, or arbitrary user-provided shared modules. PHP and FrankenPHP Managed Resource artifacts are distributed as prebuilt macOS binaries with a common default extension set plus a curated catalog of bundled optional shared modules. Optional modules are disabled by default and loaded only through PV-generated runtime ini overlays when a Project asks for them.

PV builds standalone PHP and FrankenPHP as single-binary/static-style artifacts with fixed default compiled-in extensions and bundled optional shared modules. These artifacts must not depend on Homebrew or local package-manager libraries. PV does not support arbitrary dynamic PHP extension loading, `phpize`, or PECL-installed extensions.

Standalone PHP artifacts include the `php` executable and runtime files needed by that build. They do not include `phpize` or `php-config` in v1 because user-built extensions are not supported.

The default loaded PHP extension set is Laravel-first and shared across supported PHP tracks: `bcmath`, `ctype`, `curl`, `dom`, `exif`, `fileinfo`, `filter`, `ftp`, `gd`, `hash`, `iconv`, `intl`, `json`, `libxml`, `mbstring`, `openssl`, `pcntl`, `pcre`, `pdo`, `pdo_mysql`, `pdo_pgsql`, `pdo_sqlite`, `phar`, `posix`, `session`, `simplexml`, `sockets`, `sodium`, `sqlite3`, `tokenizer`, `xml`, `xmlreader`, `xmlwriter`, `zip`, and `zlib`. GD includes FreeType, JPEG, AVIF, and WebP support.

The initial bundled optional extension catalog is `redis`, `sqlsrv`, `pdo_sqlsrv`, `xdebug`, `apcu`, `pcov`, `imagick`, `mongodb`, `yaml`, and `rar`. Future optional extensions should be added only when users ask for them and PV can build, smoke-test, license, and support them across the intended PHP track and platform matrix.

For a given PHP runtime identity, standalone PHP and FrankenPHP must expose the same loaded PHP extension set so CLI and browser execution do not drift.

For a given PHP track, standalone PHP and FrankenPHP must use the exact same PHP patch version. For example, if the `8.4` track resolves to PHP `8.4.8`, both the standalone PHP artifact and the FrankenPHP artifact for that track use PHP `8.4.8`.

PV ships one PHP artifact pair per PHP track. Optional extension combinations do not create separate downloaded artifact flavors in the first implementation; they create runtime-specific ini overlays and FrankenPHP workers from the same installed track artifact.

PV builds its own FrankenPHP artifacts for the PHP tracks it supports because upstream FrankenPHP releases do not provide the exact PV-required build matrix. The initial PV-managed FrankenPHP/PHP tracks are `8.3`, `8.4`, and `8.5`, with `8.5` as the manifest default track.

PV does not support custom PHP ini settings in Project config.

For each installed PHP track, PV seeds track-level PHP defaults under `~/.pv/resources/php/<track>/etc/php.ini` and `~/.pv/resources/php/<track>/etc/conf.d/`. The defaults are mutable track data, not artifact release payload data, so artifact updates and old-release pruning do not remove user edits. PV runs standalone PHP, Composer-through-PHP, and Project-serving FrankenPHP workers with process-level `PHPRC` and `PHP_INI_SCAN_DIR` pointing at the track defaults. PV does not pass these ini discovery paths through Caddyfile `env` and does not expand the default profile into Caddyfile `php_ini` directives.

For Project-level extension opt-ins, PV generates runtime-specific `conf.d` overlays under PV-owned config storage and appends those overlays to `PHP_INI_SCAN_DIR` for the affected standalone PHP, Composer-through-PHP, and Project-serving FrankenPHP worker processes. Generated extension ini files are PV-owned and replaced during reconciliation. Unsupported extension names in Project config are ignored at runtime and surfaced as non-blocking diagnostics rather than Project config errors.

- If there are 5 Projects and all of them use the same PHP version, PV provisions 1 Project-serving FrankenPHP process.
- If 2 Projects use PHP 8.3, 2 use PHP 8.4, and 1 uses PHP 8.5, PV provisions 3 Project-serving FrankenPHP processes. The Gateway proxies each Project hostname to the worker for that Project's PHP runtime.
- If 2 Projects use PHP 8.4 with no optional extensions and 1 Project uses PHP 8.4 with `redis`, PV provisions 2 Project-serving FrankenPHP processes for PHP 8.4.

User commands describe what should exist. The daemon reconciles the machine toward that desired state and records observed status when reality does not match.

PHP runtime resolution: Project config `php` field → global default. Project config may use either scalar form, such as `php: 8.4`, or object form, such as `php: { version: 8.4, extensions: [redis] }`. If the object form omits `version`, PV resolves the PHP track through the global/default flow and applies the requested extension list to that resolved track.

PV does not infer PHP versions from `composer.json`. Composer constraints can be complex and are not always present, so Projects that need a specific PHP version should declare it in Project config.

If Project config asks for a PHP version that is not installed, daemon reconciliation installs it automatically for Project serving.

`pv php:use <track>` sets the current linked Project's PHP track. It resolves and validates the current Project config, installs the standalone PHP and matching FrankenPHP artifacts for the resolved track, updates the preferred Project config file (`pv.yml` when present, otherwise `pv.yaml`, otherwise a new `pv.yml`), records the resolved concrete track in `pv.db`, and requests Project reconciliation.

`pv php:use <track> --global` sets the global default PHP track. It resolves the requested track, installs the standalone PHP and matching FrankenPHP artifacts, then records the resolved concrete track in `pv.db`. The global default is used outside linked Projects and by linked Projects without `php` in Project config.

Both Project and global `php:use` install the missing standalone PHP and FrankenPHP artifacts before recording the selection. If installation fails, the selection is not changed. `pv php:install [track]` installs the standalone PHP and FrankenPHP pair without changing any Project or global selection.

The `php` shim does not auto-download missing tracks. If the resolved Project or global track is not installed, it exits non-zero with a repair command such as `pv php:install <track>`.

## Desired State and Daemon Availability

Commands that change desired state write that state even when the daemon is not running.

After writing desired state, the command requests reconciliation. If the daemon is not running, the command warns that reconciliation is pending and exits successfully. The command exits non-zero only when recording desired state fails or the command input is invalid.

Desired-state commands wait for reconciliation only when their contract implies readiness, such as `pv setup` and explicit install/update commands. Fast intent commands such as `pv link` and `pv unlink` write desired state, request reconciliation, and return after the daemon accepts the job.

Non-waiting commands such as `pv link` exit zero once desired state is recorded and the daemon has accepted reconciliation, even if that reconciliation later fails. Waiting commands such as setup, install, update, and restart exit non-zero when their daemon job fails.

Install and update commands submit daemon jobs, stream progress over the socket, and exit when the job completes or fails.

`pv update` updates the PV application, its independently versioned privileged helper, and all installed Managed Resource tracks. Resource-specific update commands, such as `pv mysql:update`, update only installed tracks of that Managed Resource.

`pv update` updates the application and helper before Managed Resources so the latest daemon owns resource update logic and manifest interpretation. It downloads only changed app/helper components, verifies each SHA-256 checksum and byte count, updates the registered helper before activating an app that requires it, installs changed release files under `~/.pv/bin/releases/<version>/`, atomically updates `~/.pv/bin/pv` when the app changed, coordinates daemon restart only for an app update, releases the self-update/daemon-mutation coordination lock, and then runs a daemon-owned Managed Resource update job. If verification, helper lifecycle, binary replacement, daemon restart, daemon health, or startup migration health fails, `pv update` stops and reports the failure instead of continuing to Managed Resource updates.

If the PV application is already current, `pv update` still continues to the daemon-owned Managed Resource update phase in the same foreground process after releasing the coordination lock. If a newer PV application release is activated successfully, `pv update` re-execs the active `~/.pv/bin/pv` through an internal hidden continuation path before submitting the Managed Resource update job. The continuation path skips the app-update phase, does not reprint the `PV update` header or app-phase lines, validates that it is running from the installed active release layout, and submits only the daemon-owned Managed Resource update phase.

`pv update` does not prompt for confirmation before applying available PV application or Managed Resource updates. Running the command is the explicit user intent to update. Safety comes from checksum verification, atomic release installation, rollback, and non-destructive data handling.

`pv update --check` refreshes the PV app update manifest and Managed Resource artifact manifest, reports PV application and privileged-helper availability, reports updates for installed Managed Resource tracks, and exits without applying changes. It checks the app and helper even when no Managed Resources are installed.

`pv update --check` exits zero when the update check succeeds, even if updates are available. It exits non-zero only when the check itself fails.

`pv update --check` requires the PV daemon to be running. If the daemon is not available, it fails with a clear message suggesting `pv daemon:restart` or `pv setup`.

`pv update --check` is read-only and does not take the coordination lock. If a foreground self-update or daemon mutation owns the lock, it fails clearly instead of waiting or mutating state.

`pv update --check --json` is supported in v1 and reports machine-readable PV application update availability plus installed Managed Resource track update availability. Non-check update progress does not need JSON output in v1.

`pv update --check` reports both PV application update availability and Managed Resource update availability when possible. If Managed Resource update metadata requires a newer PV application version, the check reports the available PV application update and clearly marks Managed Resource update availability as blocked until PV is updated.

The plain `pv update` output is quiet and phase-based. Successful examples are:

```text
PV update
Privileged helper: current 1.0.0 (protocol 1)
PV application: updated 0.1.0 -> 0.2.0
Daemon restarted and healthy
Managed Resources: updated 3 artifact(s)
Managed Resources reconciled: <daemon summary>
```

```text
PV update
PV application: current 0.2.0
Privileged helper: current 1.0.0 (protocol 1)
Managed Resources: current
```

```text
PV update
PV application: current 0.2.0
Privileged helper: current 1.0.0 (protocol 1)
Managed Resources: none installed
```

Pre-activation failures print `PV update` to stdout when the command has started, one specific `error: ...` line to stderr, and no rollback message. `--json` remains valid only with `--check`; bare `pv update --json` remains invalid.

The plain `pv update --check` output is:

```text
PV application: current 0.1.0
Privileged helper: current 1.0.0 (protocol 1)
Managed Resources:
  none installed
```

When an application update is available, the first line is `PV application: update available <current-version> -> <latest-version> (<platform>)`. When the app update check cannot select an asset for the current platform, the first line is `PV application: unavailable <current-version> (<reason>)`. App manifest parse, compatibility, fetch, and cache errors are check failures, not availability statuses, so the command exits non-zero and reports the error on stderr.

The helper line follows the app line as `Privileged helper: current <version> (protocol <protocol>)`, `Privileged helper: update required <current-version> -> <latest-version> (protocol <protocol>)`, or `Privileged helper: unavailable (<reason>)`. An older app manifest cannot repair a missing or protocol-mismatched helper and reports that no applicable helper repair exists.

Installed Managed Resource tracks are listed one per line as `  <resource> <track>: <status> <details>`. Status values are `current`, `update available`, `blocked`, `revoked`, and `unavailable`. `current` includes the installed artifact version. `update available` includes `<current-artifact-version> -> <latest-artifact-version>`. `blocked` includes `requires PV <minimum-pv-version>, current PV <current-pv-version>` and is used when artifact metadata cannot be interpreted until PV itself is updated. `revoked` is used when the currently installed artifact is explicitly revoked in the refreshed artifact manifest; it includes the installed artifact version, revocation reason, and the replacement artifact version when one is available. `unavailable` is used for per-track metadata problems such as a missing resource, missing track, no installable platform candidate, or ambiguous artifact selection; it includes the installed artifact version and the reason. If the newest candidate in the refreshed artifact manifest is revoked but a non-revoked fallback exists, output keeps the normal `current`, `update available`, or `revoked` status and appends `; newest <revoked-artifact-version> revoked: <reason>`.

`pv update --check --json` writes this command-specific object to stdout on successful checks:

```json
{
  "app": {
    "status": "current",
    "current_version": "0.1.0",
    "latest_version": "0.1.0",
    "platform": "darwin-arm64",
    "asset": {
      "url": "https://downloads.prvious.test/pv/0.1.0/pv-darwin-arm64",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 12345678
    },
    "helper": {
      "status": "current",
      "current_version": "1.0.0",
      "latest_version": "1.0.0",
      "current_protocol_version": 1,
      "latest_protocol_version": 1,
      "url": "https://downloads.prvious.test/pv/0.1.0/pv-helper-1.0.0-darwin-arm64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": 2345678,
      "reason": null
    },
    "reason": null
  },
  "managed_resources": [
    {
      "status": "update_available",
      "resource": "mysql",
      "track": "8.4",
      "current_artifact_version": "8.4.0-pv1",
      "current_artifact_path": "/Users/me/.pv/resources/mysql/8.4/releases/8.4.0-pv1",
      "latest_artifact_version": "8.4.1-pv1",
      "current_revocation": null,
      "latest_revocation": null,
      "blocked_by": null,
      "reason": null
    }
  ]
}
```

The JSON `app.status` values are `current`, `update_available`, and `unavailable`. `latest_version` and `asset` are `null` only for `unavailable`. `app.helper` is present when a platform asset was selected and contains its own `status`, current/latest version, current/latest protocol, URL, checksum, size, and nullable reason. Helper status values are `current`, `update_available`, and `unavailable`; current identity fields are null when the helper cannot report them, and `reason` explains `unavailable`. The JSON Managed Resource `status` values are `current`, `update_available`, `blocked`, `revoked`, and `unavailable`. `current_artifact_version`, `current_artifact_path`, `resource`, and `track` always identify the installed track. `latest_artifact_version` is `null` only when no installable latest artifact can be selected or the check is blocked before parsing artifact metadata. `current_revocation`, `latest_revocation`, and `blocked_by` are nullable objects. Revocation objects contain `artifact_version` and `reason`. `blocked_by` contains `minimum_pv_version` and `current_pv_version`. `reason` is `null` except for `unavailable` statuses.

The daemon protocol adds a read-only `managed_resource_update_check` request:

```json
{
  "protocol_version": 3,
  "command": "managed_resource_update_check"
}
```

The daemon response is a normal response line with `status: "ok"` and an `update_check` object:

```json
{
  "type": "response",
  "protocol_version": 3,
  "status": "ok",
  "message": "Managed Resource update check completed",
  "update_check": {
    "managed_resources": [
      {
        "status": "current",
        "resource": "redis",
        "track": "8.8",
        "current_artifact_version": "8.8.0-pv1",
        "current_artifact_path": "/Users/me/.pv/resources/redis/8.8/releases/8.8.0-pv1",
        "latest_artifact_version": "8.8.0-pv1",
        "current_revocation": null,
        "latest_revocation": null,
        "blocked_by": null,
        "reason": null
      }
    ]
  }
}
```

The daemon returns all installed Managed Resource tracks, including current tracks, not only tracks with available updates. If the artifact manifest requires a newer PV version, the daemon still returns all installed tracks from local state and marks each one `blocked` with `blocked_by`. Per-track metadata errors are reported as `unavailable` entries so one bad resource or track does not hide other successful checks. Global refresh failures other than manifest incompatibility, such as network failure with no usable metadata, return a normal daemon error response and make the command fail.

PV does not auto-check for updates in the background. Update-related network checks happen only when users run `pv update`, `pv update --check`, setup/install commands that need manifests, or explicit install/update commands for Managed Resources.

Resource-specific update commands do not support `--check` in v1. Update preview is available only through top-level `pv update --check`.

PV application self-update metadata comes from a PV app update manifest that is separate from the Managed Resource artifact manifest. The app update manifest includes PV application version metadata, platform-specific download URLs, SHA-256 checksums, and compatibility fields needed by the self-updater.

The v1 PV app update manifest is a single stable-channel release document. It does not contain multiple channels, preview metadata, nightly metadata, or Managed Resource artifact data. The minimal v1 JSON shape is:

```json
{
  "schema_version": 2,
  "channel": "stable",
  "version": "0.2.0",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "assets": [
    {
      "platform": "darwin-arm64",
      "url": "https://downloads.prvious.test/pv/0.2.0/pv-darwin-arm64",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.prvious.test/pv/0.2.0/pv-helper-1.0.0-darwin-arm64",
        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "size": 2345678
      }
    },
    {
      "platform": "darwin-amd64",
      "url": "https://downloads.prvious.test/pv/0.2.0/pv-darwin-amd64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.prvious.test/pv/0.2.0/pv-helper-1.0.0-darwin-amd64",
        "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "size": 2345678
      }
    }
  ]
}
```

`schema_version` must be `2`; other schema versions fail as unsupported rather than being partially interpreted. `channel` must be exactly `stable`; runtime/user-facing channel selection is not part of v1. `version`, `minimum_pv_version`, and helper `version` use PV's simple version identity, `major.minor.patch` with no leading zero components. `minimum_pv_version` is the minimum currently installed PV application version that may apply this release; if the running PV binary is older than that value, self-update manifest parsing fails clearly and tells the user that a newer PV parser is required.

`published_at` must be an RFC 3339 timestamp for the PV application release. Each entry in `assets` describes a native PV application binary and separate native helper binary for one exact supported target platform. V1 app update assets support `darwin-arm64` and `darwin-amd64`; `any` is not valid. Helper `protocol_version` is a positive integer versioned independently from the app and helper release versions. Each `url` must be HTTPS, include a host, and include a non-empty final path segment that is not `.` or `..` and does not contain backslashes. Each `sha256` must be a 64-character hexadecimal digest, normalized case-insensitively by the parser. Each `size` must be a positive byte count.

The manifest must contain at least one asset and must not contain duplicate assets for the same platform. Selecting the current platform returns the matching asset for the current target platform. If the current target platform is missing, selection fails clearly instead of falling back to another platform.

The PV app update manifest is published at a stable PV-owned URL used by the Rust self-updater. Initially, that stable URL may be backed by GitHub Releases, such as a versioned `pv-app-manifest.json` release asset plus a stable latest manifest URL. The human-facing installer URL is separate and serves a generated installer script based on the same PV app release metadata.

PV v1 relies on HTTPS/GitHub trust for the PV app update manifest itself. The app update manifest format should allow signatures to be added later without breaking compatibility.

For `pv update`, the CLI fetches the PV app update manifest and performs PV binary self-update before handing Managed Resource update work to the daemon. The daemon owns Managed Resource manifest refresh, install, update, and runtime reconciliation through a mutating `RunJob` request with `kind = "update"` and `scope = "system"`.

The mutating PV application phase runs in the foreground `pv update` process. The foreground process activates the new `~/.pv/bin/pv` symlink when needed, restarts or kickstarts the daemon, waits for daemon health, reports success or rollback, and releases the coordination lock before Managed Resource work begins. App-current continuation submits the daemon update job from the same process. App-updated continuation re-execs the active `~/.pv/bin/pv` into a hidden internal continuation path so the newly active CLI/protocol submits the daemon update job.

The PV application update phase runs in this order:

1. Print `PV update`.
2. Acquire the self-update/daemon-mutation coordination lock at `~/.pv/run/update.lock`.
3. Validate the installed active release symlink and running version.
4. Validate or normalize the PV-owned LaunchAgent to `~/.pv/bin/pv daemon:run`.
5. Fetch and parse the PV app update manifest.
6. Compare the app version plus installed helper version/protocol with the selected platform asset. If both are current, report current, release the coordination lock, and continue to the Managed Resource phase in the same process.
7. Download and verify only the changed components. For a combined app/helper update, store the checksummed helper and its user-owned version, protocol, and checksum metadata beside the target app release, then install or replace the root helper before app activation. For a helper-only update, install and lifecycle-probe the root helper from the command-scoped download first, then promote the helper plus metadata into the current release; promotion failure restores both the previous registered helper and release files. `pv setup` uses the active release metadata for later helper repair. A helper lifecycle failure stops before app activation.
8. Install and activate the newer app release when its version changed.
9. Reload the validated PV-owned LaunchAgent with tolerant bootout/bootstrap, then kickstart the daemon without submitting `reconcile system`.
10. Wait for daemon health.
11. Release the coordination lock and re-exec the active `~/.pv/bin/pv` into the internal Managed Resource continuation.

The coordination lock covers the network fetch, binary swap, and daemon transition. `pv update` does not enqueue `reconcile system` or `update system` while the coordination lock is held. The Managed Resource update job is submitted only after the foreground app phase releases the lock or after the re-execed continuation starts.

If the PV application and helper are already current, top-level `pv update` reports both current and continues to the daemon-owned Managed Resource update phase without download, reinstall, reactivation, or daemon restart. A helper-only update downloads and explicitly authenticates only the helper lifecycle operation, does not reactivate the app, and does not restart the daemon. Helper-only publication reuses and verifies the immutable current app assets, publishes new helper-version-scoped release records, and rejects protocol changes because those require a matching app release. An app-only update does not replace the registered helper and does not prompt for administrator authentication.

As a one-time publication transition, release tooling may read only the version from a current schema-1 stable app manifest and requires the schema-2 candidate app version to be strictly newer. It does not carry legacy assets forward or add schema-1 support to runtime update parsing.

Top-level `pv update` updates all installed Managed Resource tracks, not only tracks currently needed by linked Projects.

The daemon `update system` job refreshes the Managed Resource artifact manifest once for the whole top-level Managed Resource phase, including when no Managed Resource tracks are installed. All installed-track update decisions in that phase use the same refreshed manifest snapshot. The job considers installed PHP/FrankenPHP tracks as paired update groups, then Composer track `2` when installed, then backing service tracks in deterministic canonical order. It updates only installed tracks within their existing tracks, does not install manifest defaults just because they exist, does not install tracks that are only demanded by Project config, and does not rewrite Project config.

After one or more Managed Resource artifacts change, the daemon update job runs system reconciliation before reporting success so running updated resources restart or continue under existing runtime ownership rules. If no tracks changed, the job may skip heavier runtime restart work and report `current` or `none installed`. If reconciliation fails after artifacts were updated, the update job fails and reports the reconciliation/runtime failure. PR 22E does not add a global all-resource rollback; each track or PHP/FrankenPHP pair keeps its existing atomic update behavior, the previous artifact revision remains retained where the installer supports it, and successful earlier resource updates remain in place.

For `pv update --check`, the CLI fetches the PV app update manifest and computes PV application update availability, then asks the daemon to refresh the Managed Resource artifact manifest and report installed Managed Resource track update availability.

User-facing `pv update` only performs PV application self-update when PV is running from the installed active release layout. Before mutation, `~/.pv/bin/pv` must be a symlink to `releases/<version>/pv`, that symlink target must exist and be a file, and the symlink version must match the running PV application version. If any precondition fails, `pv update` fails before download or activation and does not repair the layout by copying `current_exe()`.

PV application update is version-driven. If the manifest app version equals or is lower than the running version, it does not download, reinstall, or reactivate that app version. An equal app version may carry a same-protocol helper-only update. For an older app manifest, PV accepts only a strictly newer helper on the already-running protocol; it never downgrades a helper or uses the older manifest to repair a missing/protocol-mismatched helper. Downgrades are out of v1 scope.

Installed and self-updating PV uses the stable active symlink as the LaunchAgent program path: `~/.pv/bin/pv daemon:run`. During update preflight, a PV-owned stale LaunchAgent plist is normalized to that stable path. After every actual app release activation, PV reloads the validated PV-owned launchd job with tolerant bootout/bootstrap before kickstart so launchd resolves the active symlink again and uses the current `ProgramArguments`. This reload is required whether the plist was already current or was normalized during preflight. If post-activation health fails, rollback restores the previous active symlink and repeats the same bootout/bootstrap/kickstart sequence before checking daemon health. A missing LaunchAgent fails before activation with guidance to run `pv setup` or `pv daemon:enable`. A non-PV-owned LaunchAgent fails before activation and is left unchanged. If LaunchAgent normalization succeeds but the manifest says the PV application is current, `pv update` exits zero without restarting or reloading the daemon.

PV app and helper downloads use freshly created command-scoped temporary files under `~/.pv/downloads/`; an existing path is never reused or truncated. While streaming each response, PV computes SHA-256 and counts bytes. After the stream completes, PV verifies byte count and digest against the selected manifest component. Verification failure deletes the temporary file, fails before activation, and leaves the active release unchanged. Successful installation stores the binaries in the selected release directory, then removes the temporary downloads; v1 does not introduce a persistent app-component download cache.

PV self-update keeps the previous PV application binary and helper candidate for rollback. If the updated binary cannot restart the daemon and report healthy, `pv update` restores the previous app and helper identities, restarts the daemon again, and reports that the app update was rolled back. If no helper was registered before the update, helper rollback removes the newly registered helper. A daemon protocol mismatch during the post-activation health check means the newly activated daemon is running with a newer protocol and is not by itself a rollback reason; subsequent user commands run through the newly active `~/.pv/bin/pv` binary.

PV application rollback applies only to PV app update failures or post-update daemon health failure. If the PV app self-update succeeds but later Managed Resource updates fail, PV keeps the newer app binary installed and reports the Managed Resource update failure.

PV applies database migrations after swapping to the new PV binary and restarting with that binary. The new binary owns its embedded migrations; rollback remains safe because migrations are required to be backward-compatible with the immediately previous PV version.

If the new PV binary's database migration fails during self-update, `pv update` rolls back to the previous PV binary, restarts the daemon with the previous binary, and reports the migration failure. Transactional migrations should leave `pv.db` unchanged on failure.

The daemon writes a minimal startup failure marker at `~/.pv/run/daemon-startup-error.json` when startup fails before it can serve health. A marker has `kind` set to `migration_failed` for `StateError::MigrationFailed` and `StateError::MigrationNameMismatch`, or `startup_failed` for other daemon startup errors, plus a user-facing `message`. The daemon removes any stale marker on successful startup before it serves health. `pv update` clears stale marker content immediately before kickstarting the updated daemon and reads this marker only after post-update daemon health wait fails; missing or malformed marker content falls back to a generic daemon health failure.

`pv update` does not create an extra `pv.db` backup before every PV application self-update. The embedded migration system creates a timestamped backup only when migrations are about to run.

PV application self-update restarts the daemon/control plane but does not stop currently running Gateway, Project-serving workers, or backing Managed Resource processes before swapping the PV binary. The updated daemon adopts existing PV-owned child processes where ownership verification passes and then reconciles or updates resources as needed when a later command requests that work.

App rollback is attempted after activation was attempted. Helper rollback is also attempted when helper replacement itself fails, before any incompatible app can be activated. If daemon startup, daemon health, or migration health fails after activation and rollback succeeds, stdout reports `PV application: update failed; rolled back to <previous-version>` and stderr reports the original failure. A generic daemon health failure is reported as `daemon did not become healthy after update`; migration failure is reported as `database migration failed after update: <message>`.

If restoring the previous active symlink fails, stdout reports that rollback failed and stderr includes both the original failure and the symlink restore failure. If the previous symlink is restored but daemon restart or health after rollback fails, stdout reports that the previous app release was restored, stderr includes the original failure, says daemon restart after rollback failed, and suggests `pv daemon:restart` or `pv setup`. After the previous symlink is restored, PV attempts to remove the failed new release directory even if daemon restart after rollback fails; cleanup failure is a warning, not the primary result.

PV coordinates foreground self-update with queued daemon mutations through an OS-level advisory filesystem lock whose legacy path remains `~/.pv/run/update.lock`. The foreground `pv update` process holds it during the binary swap and daemon transition. The daemon mutation queue holds the same lock while reconciliation or update work is queued or running. This is not a universal serializer for every foreground PV mutation.

Contention errors describe the active self-update/daemon-mutation coordination lock and its backing path. The persistent `update.lock` file is not itself evidence of contention: it may remain after the owning process exits, and an unlocked stale file does not block work. The active OS lock is released automatically when its owning file handle closes. Newly generated CLI, daemon, and job errors use this neutral coordination terminology. Previously persisted job summaries and errors remain historical data and are returned verbatim; PV does not migrate or retroactively normalize their wording.

While the coordination lock is held, conflicting mutating commands fail clearly instead of waiting. The daemon rejects new mutating requests; it does not queue them behind a foreground self-update. Simple local read-only commands that do not require daemon protocol compatibility, such as `pv env`, may still run. Read-only commands that need daemon state, such as `pv status`, fail clearly during the transition.

After a successful PV application self-update, PV keeps the current app release plus one previous app release under `~/.pv/bin/releases/`. Older app releases are pruned. Pruning never removes the active release or the previous rollback release. If pruning fails after a successful app update and healthy daemon restart, `pv update` exits zero and reports the cleanup failure as a warning on stderr.

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

`pv uninstall` is safe by default. It stops and unregisters the LaunchAgent, removes `/etc/resolver/test`, removes PV's `pf` redirect rules, removes PV local CA trust, deregisters and removes the root helper, removes the installer-managed `PV ENV` shell profile block when present, stops PV-managed processes, and removes PV app binaries, shims, runtime metadata, sockets, generated configs, and installed Managed Resource binaries. Helper removal cleans the exclusively PV-owned system support directory, including interrupted work files. macOS may request administrator authentication when helper artifacts exist; an installation created with `--no-setup` does not prompt merely to remove an absent helper. Before editing a shell profile during uninstall, PV creates a backup.

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
    caddy/
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

During reconciliation and `pv restart`, the daemon regenerates and validates Gateway/worker configs before loading them through the Caddy admin API or starting a lifecycle-replacement process. If config generation or validation fails, PV keeps currently working processes running and reports the failure.

Gateway config validation uses the installed Caddy track `2` binary, while worker config validation uses the matching installed FrankenPHP binary. Each validates its generated Caddyfile before an admin load or lifecycle replacement. If validation fails, PV keeps the previous active config and process and surfaces the validation error in observed state and logs.

Generated Gateway/worker config writes are atomic. PV writes new config to temporary files, validates them, then atomically renames them into place so runtime processes never read partial config files.

PV keeps the previous active generated Gateway/worker config until the new config validates, loads successfully through `POST /load`, and passes runtime readiness. An API rejection restores the previous generated files while Caddy keeps the previous in-memory config active. A later readiness failure reloads the restored previous root through the same admin endpoint; if that rollback fails, PV reports both failures and does not claim that the previous runtime was restored.

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

The `php` shim is Project-aware, similar to version managers such as `fnm` or `nvm`. When run inside a linked Project, it uses that Project's resolved PHP runtime identity. Outside a linked Project, it uses the global default PHP track without Project-level optional extensions.

Composer is split by responsibility: the Composer PHAR and version metadata live under `~/.pv/resources/composer/`, the Composer shim lives under `~/.pv/bin/`, and `~/.pv/composer/` is the user-facing `COMPOSER_HOME` for global packages and cache.

The Composer shim invokes the Composer PHAR through PV's `php` shim so Composer inherits Project-aware PHP selection. Inside a linked Project, Composer uses that Project's PHP runtime identity; outside, it uses the global default PHP track without Project-level optional extensions.

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

The Managed Resource artifact manifest endpoint is a property of the built PV binary in v1. Production/default builds use PV's stable/default artifact manifest endpoint. Maintainer staging builds may override that compiled default by setting `PV_DEFAULT_ARTIFACT_MANIFEST_URL` at build time, for example:

```sh
PV_DEFAULT_ARTIFACT_MANIFEST_URL=https://artifacts-staging.pv.prvious.dev/manifest.json cargo build --release
```

PV v1 does not expose runtime/user-facing artifact manifest selection through CLI flags such as `--channel` or `--manifest-url`, config files, shell environment variables, LaunchAgent environment variables, shell profile edits, installer channel parameters, or database state. Runtime/user-facing manifest selection could redirect PV to an unintended artifact manifest and could cause the CLI and daemon to disagree about which manifest owns Managed Resource artifacts. Tests may still inject manifest URLs through test-only seams.

The PV artifact release pipeline may either wrap suitable upstream binaries or build missing binaries from source, but it always produces a normalized PV artifact archive before publishing. For example, if Redis does not publish the macOS binary shape PV needs, the release pipeline builds Redis ahead of time and publishes the resulting PV-owned Redis artifact. The release pipeline is expected to run in hosted automation such as GitHub Actions, not on user machines.

Artifact recipes prefer wrapping official upstream binaries when those binaries pass PV's archive validation and smoke-test requirements. Recipes build from source when upstream binaries are unavailable, do not match PV's required build matrix, cannot be packaged to run from PV's installed resource layout, or fail PV's smoke tests.

Managed Resource artifact build recipes, scripts, patches, and expected archive layouts live in the PV repository, such as under `release/artifacts/`, so artifact production changes are reviewed with PV adapter and manifest compatibility changes. Deployment secrets and storage credentials stay in CI/provider configuration, not in the repository.

Artifact recipes may apply small versioned build or packaging patches when required for macOS portability, static-style PHP/FrankenPHP builds, PV-installed resource layout compatibility, or reproducible packaging. PV avoids long-lived behavior-changing forks of upstream Managed Resources. Any patch that changes runtime behavior rather than build/packaging behavior requires an explicit design decision before publication.

PV v1 keeps Managed Resource artifact build/release automation in the same repository and CI system as the PV application. A separate artifact-build repository is deferred until coordination or security needs justify the split.

Managed Resource artifact build and publication workflows are separate from PV application binary build and release workflows. Normal PV application CI/release does not rebuild Managed Resource artifacts. Artifact publication is an explicit release workflow with resource, track, upstream version, PV build revision, and target platform inputs.

Shared artifact release metadata validation and manifest generation are implemented in Rust as internal repository tooling, such as an `xtask` or `pv-release` crate. Resource-specific build recipes remain shell scripts because they mostly orchestrate upstream tools such as `configure`, `make`, `cmake`, `spc`, `go build`, `cargo build`, `codesign`, `otool`, and `tar`.

PV repackages even usable upstream binaries into a consistent artifact layout instead of exposing raw upstream archive layouts to the client. This keeps install, validation, rollback, and adapter behavior stable when upstream packaging changes.

Each Managed Resource artifact is distributed as a single `.tar.gz` archive per resource, track, upstream version, PV build revision, and platform. PV downloads one archive, verifies it against the remote artifact manifest, unpacks it into a temporary directory, validates the expected adapter-specific files, and then atomically installs it.

Each Managed Resource artifact archive expands into exactly one top-level directory named from the artifact identity, such as `redis-7.2.5-pv1-darwin-arm64/`. The archive must not place files directly at the extraction root.

Each Managed Resource artifact archive includes upstream license and notice files where required by the redistributed resource and bundled dependencies. PV v1 does not provide a dedicated licenses command.

License and notice validation happens in the artifact release pipeline, not in PV's client-side resource adapters. Runtime adapters validate files required to install and run the resource, while publication checks enforce licensing metadata before an artifact appears in the public manifest.

Managed Resource artifacts must run from PV's installed resource layout before publishing. Any shebang, rpath, install-name, or embedded path fixes happen in the release pipeline before final signing and checksum generation. User machines do not patch Managed Resource binaries during install.

PV does not rely on whole-archive binary or string scanning as a v1 publication gate. Artifact recipes prove portability through archive layout validation, adapter-required file checks, and target-platform smoke tests. Resource-specific static checks may be added later if a resource's packaging risk justifies them.

For v1, PV ad-hoc signs Managed Resource Mach-O binaries in the release pipeline after any binary path fixes. Paid Developer ID signing and notarization for Managed Resource artifacts are deferred unless macOS Gatekeeper or quarantine behavior requires them for a reliable v1 install experience. Checksums are computed only after final signing and packaging.

The remote Managed Resource artifact manifest is the only manifest in v1. PV-owned artifact archives do not contain per-archive manifest files in v1; validation comes from the remote manifest plus the compiled-in resource adapter rules.

Published Managed Resource artifact archive URLs are immutable. Artifact object keys include enough identity to distinguish the resource, track, upstream version, PV build revision, platform, and content. If a published artifact is bad, PV publishes a new build revision and updates the manifest to point at the new artifact instead of replacing the existing object in place.

Managed Resource artifact identity includes both the upstream resource version and a PV build revision. For example, a Redis artifact may represent upstream Redis `7.2.5` with PV build revision `pv1`; if PV changes packaging, patches, build flags, or validation for the same upstream version, it publishes `pv2` rather than mutating `pv1`.

The artifact manifest stores upstream version and PV build revision as separate fields, such as `upstream_version` and `pv_build_revision`, plus a derived display/install identity such as `artifact_version: "7.2.5-pv1"`. Each artifact also records `published_at`, the timestamp when that artifact became installable through the public manifest. PV uses the separate version fields and `published_at` for update logic and diagnostics while showing the combined artifact version where concise output is useful.

The artifact manifest may include artifact provenance metadata such as upstream source URL, upstream checksum, applied patch identifiers, PV repository commit SHA, recipe path/version, build run ID, and build timestamp. Provenance metadata is for diagnostics, audit, and release operations; it is not a client-side build instruction set.

The artifact manifest does not define Managed Resource lifecycle behavior or resource-specific archive layout requirements. Install, start, init, readiness, allocation, reconciliation behavior, and required file/path validation live in PV's resource adapters because each Managed Resource has different lifecycle rules. For example, the Redis adapter knows it needs `bin/redis-server`, while the Postgres adapter knows it needs `bin/postgres`, `bin/initdb`, and supporting files such as `share/postgres.bki`.

PV resource adapters are compiled into the Rust binary. PV will not support plugin resource adapters; all control-plane and adapter behavior lives in the single `pv` binary. Managed Resources remain external binaries/artifacts managed by PV.

If the artifact manifest schema is unsupported, or the manifest requires a newer PV version than the installed PV application, commands that need artifact metadata fail clearly and tell the user to run `pv update`.

Manifest incompatibility does not stop already-installed local runtime from working. Existing linked Projects, Gateway, DNS, installed Managed Resources, and desired state continue using local `pv.db` and installed artifacts. Only commands that need artifact metadata fail.

The artifact manifest defines resource-specific update tracks. Project config versions select a track, not necessarily a full upstream semantic version. `pv update` and resource-specific update commands update installed Managed Resources only within their existing tracks.

Examples: MySQL `8.0` tracks update within `8.0.x`, MySQL `8.4` tracks update within `8.4.x`, PostgreSQL `17` tracks update within `17.x`, and PostgreSQL `18` tracks update within `18.x`.

The initial v1 Managed Resource artifact track set is Caddy `2`; PHP/FrankenPHP `8.3`, `8.4`, and `8.5`; MySQL `8.0`, `8.4`, and `9.7`; Postgres `17` and `18`; Redis `8.8`; Composer `2`; Mailpit `1`; and RustFS `1`. Manifest defaults are Caddy `2`, PHP/FrankenPHP `8.5`, MySQL `8.4`, Postgres `18`, Redis `8.8`, Composer `2`, Mailpit `1`, and RustFS `1`. MySQL `8.0` is compatibility-only, not a default.

Install commands and Project config versions resolve to the latest artifact in the requested track. "Latest" means the non-revoked artifact with the newest `published_at` timestamp after platform selection. If two candidate artifacts for the same resource, track, and platform have the same `published_at`, the manifest is ambiguous and invalid. For example, `pv mysql:install 8.0` installs the latest available MySQL artifact in the `8.0` track.

`latest` is accepted as a version alias that resolves to the manifest's default track for that Managed Resource. PV stores the resolved track, not `latest`, so existing Projects do not float when manifest defaults change later.

If a Project config Managed Resource block omits `version`, PV treats the omitted value as the `latest` selector and resolves it to the artifact manifest's default concrete track before writing desired state. Existing Projects keep their internally stored concrete track when manifest defaults change later.

PV does not rewrite Project config to replace `latest` with the resolved track. The resolved track is stored internally in `pv.db` and shown in status/list output.

PV v1 supports track-based versions only. It does not support exact artifact pinning in Project config.

Projects attach to Managed Resource tracks. When PV updates an installed track to a newer artifact, Projects using that track automatically use the updated artifact.

When updating a running Managed Resource track, PV restarts it immediately as part of the explicit update command. If restart fails, PV preserves the previous artifact when possible and reports the failure.

PV caches the last successfully fetched artifact manifest under `~/.pv/downloads/manifest.json`. When offline, PV can use cached metadata and already-downloaded or installed resources, but cannot install versions missing from the cache/downloads. Offline or stale-manifest failures should be reported clearly.

`pv update` refreshes the artifact manifest every time. The top-level Managed Resource phase refreshes it once per run and uses that same snapshot for every installed-track decision, including the no-installed-tracks case. Setup and install commands try to fetch the latest manifest and fall back to the cached manifest when offline. PV v1 does not need a manifest cache TTL.

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

Managed Resource artifact recipes retain an explicit macOS deployment target of macOS 13.0 even though PV supports macOS 14 and newer. Keeping otherwise compatible artifacts runnable on macOS 13 does not make that operating system supported. Recipes must not silently inherit a newer GitHub runner deployment target, and raising an artifact deployment target requires a separate compatibility decision.

The artifact manifest may use `platform: "any"` only for truly portable artifacts that do not contain platform-specific binaries. Composer is the expected v1 `platform: "any"` artifact because PV packages `composer.phar` inside a PV-owned archive. Native Managed Resource artifacts use explicit platform values such as `darwin-arm64` or `darwin-amd64`.

When resolving artifacts, PV prefers an exact platform match over `platform: "any"`. PV uses `platform: "any"` only when no exact platform-specific artifact exists for the selected resource, track, and artifact version.

The artifact release pipeline should build and validate `darwin-arm64` and `darwin-amd64` artifacts on native macOS runners for each architecture when available. Cross-compilation is acceptable only for resources where target-architecture smoke tests prove the artifact works. For database/runtime artifacts such as Postgres, MySQL, and FrankenPHP, target-architecture validation is required before publication.

For macOS v1 artifacts, recipes rely on GitHub-hosted macOS runners plus recipe-managed build dependency setup rather than containerized builds. Recipes should pin build tool versions where practical. Homebrew may be used as a CI build-time dependency source, but published artifacts must not retain unmanaged Homebrew runtime dependencies or absolute Homebrew paths.

Maintainer-local macOS artifact builds also run natively on macOS. Docker is not a supported path for producing or validating macOS Managed Resource artifacts because it cannot exercise native Mach-O linking, signing, rpaths, or runtime behavior.

Strict byte-for-byte reproducible Managed Resource builds are not a v1 requirement. Artifact recipes should still record provenance and pin source/dependency/tool inputs where practical, but v1 does not block publication on deterministic rebuild verification.

The artifact release pipeline must pass adapter-specific smoke tests before publishing a Managed Resource artifact. Redis starts `redis-server`, checks `redis-cli ping` returns `PONG`, and stops cleanly. Postgres runs `initdb`, starts `postgres`, runs `psql SELECT 1`, and stops cleanly. MySQL initializes a temporary data directory, starts the server, connects as admin, runs `SELECT 1`, and stops cleanly. Mailpit starts the server, checks the HTTP UI and SMTP port bind, and stops cleanly. RustFS starts the server, checks S3 API readiness, creates or lists a test bucket, and stops cleanly. FrankenPHP/PHP runs `php -v`, verifies the expected PHP version and required fixed extension set, serves a tiny PHP site through FrankenPHP over loopback, and stops cleanly. PHP extension validation verifies that every PV-required v1 extension is compiled into both standalone PHP and FrankenPHP; extra compiled extensions do not fail publication.

Artifact object upload and public artifact availability are separate steps. The release pipeline may upload immutable candidate artifact archives after they pass their own build checks, but PV clients only see artifacts referenced by the published artifact manifest. The public manifest references only artifacts that passed required smoke tests. Partial manifest publication is allowed only for intentionally supported platforms/resources; public v1 should not mark a resource track generally available until both `darwin-arm64` and `darwin-amd64` artifacts pass.

While StaticPHP v3 Intel FrankenPHP builds remain deferred, Artifact Publication may be configured for an Apple Silicon-only initial preview. That preview gate must be explicit in release workflow inputs and must not be treated as completion of the intended public v1 native platform matrix.

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

Project config accepts a root-level `serve` boolean. It defaults to `true`. With `serve: false`, the Project remains linked and PV still reconciles its declared Managed Resources, Resource allocations, and environment mappings, but PV does not create a Gateway route, TLS demand, or PHP worker for the Project. PV does not start framework or application development servers on the Project's behalf.

Serving-only config remains valid but dormant while `serve: false`. This includes `document_root`, `hostnames:`, the primary Project hostname, and env entries that use `${project_url}`, `${tls_cert}`, `${tls_key}`, or `${tls_ca}`. PV preserves those values in user-owned config, ignores them for runtime planning, and omits env entries that depend on serving-only placeholders. If serving is enabled again, PV validates and applies those values normally and restores the omitted env entries.

Basic YAML types, unknown keys, env key rules, and placeholder spelling are validated in both serving modes. A malformed dormant value is still a config error when it can be validated without serving context. Hostname collision and document-root existence checks are deferred until serving is enabled. As with other invalid Project config, a failed transition keeps the last valid desired state active.

Empty string values for meaningful config fields, such as `php` or Managed Resource `version`, are invalid.

Version/track fields may be YAML strings or numbers. PV normalizes them to strings during validation.

Project config can request Managed Resource tracks and define environment variable mappings for a Project. The mappings may use PV-provided placeholder values such as resource username, password, database, bucket, prefix, endpoint, and assigned port.

Project config can declare additional Project hostnames with `hostnames:`. These hostnames are routed to the same Project and receive Gateway TLS certificates for their own hostnames. `hostnames:` is additive and does not include or redefine the primary Project hostname, which comes from `pv link --hostname` or the directory-derived default. Additional hostnames must be full `.test` hostnames; PV v1 rejects non-`.test` hostnames and wildcard hostnames.

All hostnames in PV's desired routing table are unique across primary and additional hostnames. If an additional hostname conflicts with another Project's primary or additional hostname, the Project config is invalid. If `pv link --hostname` tries to use a hostname that is already primary or additional for another Project, it fails with a clear collision error. PV keeps serving the last valid desired state and surfaces conflicts in `pv list` and `pv status`.

When a served Project changes to `serve: false`, it retains its primary and additional hostname reservations so another Project cannot take them during a temporary serving pause. A Project first linked with `serve: false` does not need or reserve a real `.test` hostname.

Project config `hostnames:` cannot include the Project's own primary hostname.

Project config `hostnames:` cannot contain duplicates after normalization.

Project config can override the served document root with `document_root:`. The value must be relative to the Project root; `.` is allowed and means the Project root. PV rejects absolute paths, document roots that escape the Project directory, or paths that do not exist as directories. PV validates document roots using canonicalized paths and rejects symlink-resolved paths that escape the canonical Project root.

Project config validation rejects unknown top-level keys and unknown nested keys with clear errors. Typos in resource, env, or allocation sections fail validation and keep the last valid desired state active.

Project config accepts YAML anchors, aliases, and merge keys as YAML syntax. PV resolves them before validation. Helper keys are not a PV feature; unknown keys that remain after YAML merge and alias resolution fail validation.

If Project config asks for a Managed Resource track that is not installed, daemon reconciliation installs it automatically.

Declaring a Managed Resource in Project config means the Project needs that resource. Reconciliation installs and starts the selected track even when no env mappings or Resource allocations are declared.

An explicit `php:` selection remains available to the Project-aware `php` and Composer shims for a resource-only Project and installs the selected PHP/FrankenPHP lifecycle pair when required. It does not create a PHP worker while `serve: false`. Without explicit `php:`, a resource-only Project creates no Project-specific PHP demand and Project-aware CLI commands may use the global PHP runtime fallback.

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

SQL database names use the Project's immutable slug with underscores: `<project_slug>_<allocation_name>`. Hyphens in both the Project slug and allocation name are converted to underscores. For example, Project slug `acme` and allocation `app-db` creates database `acme_app_db`.

SQL database names are generated when the Resource allocation is first created and then stored in `pv.db`. If the Project's hostname, path, slug derivation input, or serving mode changes later, PV keeps using the existing stored database name instead of renaming the database or creating a new database.

Generated local development secrets are stored plainly in the user-owned SQLite database for v1. PV relies on filesystem permissions rather than macOS Keychain encryption at rest.

Generated credentials are stable. PV creates them once for the relevant Managed Resource instance/track or Resource allocation and does not rotate them during reconciliation or update. Credentials change only when the owning resource/allocation data is explicitly pruned or PV-owned state is removed.

PV v1 does not support credential rotation commands.

Redis Resource allocations create generated key prefixes only in v1. PV does not manage Redis logical DB indexes or ACL users in v1. Redis prefix values use `<project-slug>-<allocation>-`. For example, Project slug `acme` and allocation `cache` renders `acme-cache-`. Redis prefixes are generated when the Resource allocation is first created and then stored in `pv.db`. Later Project changes do not switch the stored key namespace.

For Redis prefixes, allocation names are normalized the same way as RustFS bucket allocation segments: underscores are converted to hyphens.

Mailpit does not support Resource allocations in v1. It is a shared capture service that may expose resource-level env values such as SMTP host, SMTP port, and dashboard URL.

RustFS uses one randomly generated PV-managed root/access credential per Managed Resource instance/track so PV can manage and access the local RustFS instance. RustFS Resource allocations create per-Project buckets and render the shared instance access credentials plus bucket name. PV v1 does not create dedicated per-allocation RustFS access keys.

RustFS Resource allocation bucket names use the Project slug and allocation name: `<project-slug>-<allocation_name>`. For example, Project slug `acme` and allocation `uploads` creates bucket `acme-uploads`.

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

PV uses readable Project-slug-based generated names for user-visible Resource allocation objects. SQL database names use `<project_slug>_<allocation_name>`, RustFS bucket names use `<project-slug>-<allocation_name>`, and Redis prefixes use `<project-slug>-<allocation>-`. These names are generated at first allocation creation and stored in `pv.db`; reconciliation always reuses the persisted `generated_name` and never renames an existing database, bucket, or prefix.

PV applies a hard 63-character maximum to generated Resource allocation object names in v1. If the generated SQL database name, Redis prefix, or RustFS bucket name would exceed 63 characters, PV fails Project config/reconciliation with a clear error. PV does not truncate, hash, or silently rewrite generated Resource allocation names in v1.

If a Resource allocation is removed from Project config and later re-added with the same name for the same Project and Managed Resource, PV reuses the same stored generated Resource allocation object name and reconnects to the existing underlying object when it still exists.

If an underlying object for a desired Resource allocation is manually deleted outside PV, reconciliation recreates it and records the drift repair.

For existing Resource allocation objects with unexpected permissions or configuration, PV repairs only what it owns and understands. Ambiguous drift is reported instead of aggressively rewritten. V1 repair focuses on existence and basic access.

Resource allocation creation is best effort across multiple allocations and Managed Resources. PV creates what it can, records failures, and does not render `.env` until the full Project reconciliation succeeds.

If Resource allocation reconciliation fails but Project serving can still be configured, PV keeps serving the Project where possible and marks the Project degraded or failed for resources. It does not update `.env` with incomplete values.

Project config supports three environment mapping scopes:

- Root-level `env:` for Project-level values such as `APP_URL`.
- Managed Resource-level `env:` for shared service values such as host, port, or dashboard URL.
- Allocation-level `env:` for Resource allocation values such as database credentials or bucket names.

Env mapping precedence is deepest wins: root-level `env:` is the base, Managed Resource-level `env:` overrides root-level keys for that resource, and allocation-level `env:` overrides both root-level and Managed Resource-level keys for that allocation.

If two same-depth sibling env mappings render the same final env key and neither mapping overrides the other by precedence, Project env rendering fails with a duplicate rendered env key error. For example, two allocation siblings under the same Managed Resource cannot both render `DATABASE_URL`; the Project config must use distinct keys such as `APP_DATABASE_URL` and `ANALYTICS_DATABASE_URL`.

Project config env values support PV's simple placeholder syntax: `${name}`. PV replaces placeholders with values from the current Project, Managed Resource, or Resource allocation context.

`$$` escapes a literal dollar sign in env values. For example, `$${name}` renders `${name}`, and `$$${name}` renders `$` followed by the resolved value of `${name}`.

Placeholder names must use lowercase snake_case, such as `${project_url}`, `${access_key}`, `${secret_key}`, and `${smtp_port}`.

`${project_url}` renders the URL for the primary Project hostname, such as `https://acme.test`. It does not vary by additional hostnames.

`${tls_key}` renders the stable PV-owned path to the TLS private key for the Project's primary hostname. `${tls_cert}` renders the stable PV-owned path to the TLS certificate chain for the Project's primary hostname. `${tls_ca}` renders the path to PV's local CA certificate. PV must never expose the local CA private key through Project env placeholders.

While `serve: false`, `${project_url}`, `${tls_key}`, `${tls_cert}`, and `${tls_ca}` remain recognized placeholders, but PV omits any complete env entry containing one of them instead of rendering a partial or fake serving value. Other entries at the same mapping scope continue to render.

TLS placeholders are scoped to the primary Project hostname only. They do not render files for additional `hostnames:`, do not imply wildcard certificate support, and do not imply wildcard Project routing. Additional hostnames remain explicit Gateway routes with standalone Caddy-managed TLS certificates.

PV owns stable Project TLS files under Project-specific storage in `~/.pv/certificates/` and refreshes them during reconciliation when the Project's primary hostname or local CA changes. Placeholder values must not point at Caddy's internal certificate storage; that layout is an implementation detail of the managed Gateway.

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

When a Project opts in with environment mappings, the daemon reconciles the requested Managed Resources and updates only a PV-owned delimited block inside the configured env file. PV never rewrites user-owned lines outside that block. If the configured env file does not exist, PV creates it automatically with the PV-owned block.

PV renders `.env` only after required Managed Resource ports and Resource allocations are known. Env rendering is all-or-nothing for the full Project config. If a required allocation or resource reconciliation fails, PV keeps the last valid managed block and records the failure instead of rendering incomplete values.

Project config accepts a root-level `env_file` path and defaults it to `.env`. The path is relative to the canonical Project root. Absolute paths, lexical `..` escapes, and symlinks that resolve outside the canonical Project root are rejected. The target's parent directory must already exist; PV does not create parent directories.

When creating a missing configured env file, PV creates a user-owned file containing only the PV-owned block with `0600` permissions. It does not copy `.env.example`. When updating an existing env file, PV preserves the existing file permissions.

If `env_file` changes, PV writes or updates only the newly configured target. PV leaves the prior target and its last PV-managed block untouched and does not track historical env targets for cleanup. If config later switches back to a previous target, PV updates the existing block there normally.

PV uses these exact `.env` delimiters:

```env
# >>> PV MANAGED
APP_URL=https://acme.test
# <<< PV MANAGED
```

If one complete PV-managed block already exists, PV replaces only the content between the delimiters. The block is fully regenerated on each reconciliation; user edits inside the PV-managed block are overwritten.

If multiple complete PV-managed blocks exist, PV removes all complete PV-managed blocks, preserves user-owned content around and between them, and writes one fresh PV-managed block.

Malformed PV-managed block markers fail safely. A start marker without an end marker, an end marker without a start marker, or nested markers cause env rendering to fail and leave the existing `.env` file unchanged.

PV appends the managed block at the end of `.env` and preserves surrounding formatting, including final newline, where practical.

During `.env` rendering, PV warns if generated env keys already exist outside the PV-managed block. It still writes the managed block, does not remove user-owned keys, and records the duplicate-key warning in observed state/logs.

Duplicate env key warnings appear as compact Project warnings in `pv list`, with details in `pv status` and logs. `pv project:env` also warns when duplicates exist, while still printing generated values.

PV writes `.env` values unquoted when safe and quotes/escapes values when necessary, such as values containing spaces, `#`, quotes, or newlines.

If a Project has no Project config, or its Project config has no environment mappings, PV does not touch the configured env file. If a previously generated PV-managed block exists, PV leaves the last generated values in place and stops updating the block. A served Project still uses the default PHP version, or the `php` version requested in Project config when present.

PV does not watch `.env` files. It only writes the PV-managed block during reconciliation when Project config or Managed Resource state requires an update.

PV v1 includes `pv init [path]` as a guided Project config initializer for existing directories. By default, it inspects local Project files, suggests conservative PHP, document root, env, and Managed Resource config, allows structured edits, previews the generated YAML, and writes only after confirmation. `pv init --yes` writes the detected defaults without prompting, while `pv init --print` prints the generated YAML without writing. Existing Project config values are preserved unless changed through the guided flow.

`pv init` directly writes only Project config. It does not link the Project, request reconciliation, call the daemon, install or start Managed Resources, write `.env`, edit `vite.config.js`, or run framework or package commands. Writing config for an already-linked Project can still trigger the daemon's normal file-watcher reconciliation. Vite detection generates the exact `VITE_DEV_SERVER_CERT` and `VITE_DEV_SERVER_KEY` env mappings, but the Project's Vite config must read them. PV v1 does not migrate Herd config and does not generate `serve: false`.

PV does not create sample Project config files during setup and does not create Project config during `pv link`.

## Project Linking

`pv link [path]` registers a Project as desired state and immediately requests daemon reconciliation. PV reads and validates Project config before choosing whether the new Project is served or resource-only.

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

The command succeeds when PV has recorded the desired Project and submitted the reconciliation request. If the daemon is running and reconciliation succeeds, a served Project should be reachable at its `.test` URL by the end of the command. A resource-only Project should have its declared Managed Resources, allocations, and env reconciled without becoming reachable through the Gateway.

`pv link` can run before `pv setup`. It records desired state and warns that setup is incomplete, so routing will not work until `pv setup` completes.

By default, PV derives the Project hostname from the Project directory basename, normalized to a DNS-safe slug. For example, `/Users/me/Code/Acme Store` becomes `acme-store.test`.

Project hostnames are normalized to lowercase and validated as DNS-safe `.test` hostnames. PV accepts and trims one trailing DNS dot, such as `acme.test.` to `acme.test`. Hostname uniqueness checks are case-insensitive because PV stores normalized lowercase hostnames.

`pv.test` is reserved for PV diagnostics or future internal UI and cannot be assigned to a Project.

PV identifies a Project by its canonical absolute path. Project slugs, generated names, and Project hostnames are unique attributes, but they are not Project identity. Running `pv link` more than once for the same canonical path updates the existing Project record.

PV stores both the original linked path string and the canonical absolute path. The canonical path is used for identity, routing, and equality; the original path is display/debug metadata.

PV also assigns each Project a random stable Project ID at first link and stores it in `pv.db`. The Project ID is PV's stable internal reference for the Project and does not change when the Project hostname changes. Project IDs should be short random URL-safe IDs, roughly 8-12 characters, to avoid local collisions while keeping diagnostics readable.

PV also assigns every Project a globally unique, immutable Project slug at first link and stores it in `pv.db`. The base slug is derived from the canonical directory basename using the same lowercase DNS-safe normalization as a default hostname label. If that slug is already stored by another Project, PV appends the first available numeric suffix: `appointment`, `appointment-1`, `appointment-2`, and so on. Slug collision checks use PV state, not underlying Managed Resource data. Relinking the same canonical path preserves its assigned slug.

The Project slug is the stable readable namespace for all newly generated Resource allocation names in both serving modes. It does not change when the Project path, hostname, or serving mode changes.

Persisted Project state records serving mode and Project slug additively. Where the current database hostname invariants require a non-null unique value for a newly linked resource-only Project, PV stores a unique internal `.invalid` hostname. This compatibility value is storage-only and must never appear in user-facing text or JSON, selectors, Gateway or TLS planning, env rendering, Resource allocation names, status, or logs.

If a Project directory moves and is linked again at the new path, PV v1 treats it as a new Project. Path-move semantics may be added later if needed.

If a linked Project path no longer exists, PV marks the Project failed or missing in observed state but keeps desired state. PV does not auto-unlink missing Projects.

Missing Projects continue to own their Project hostnames until the user unlinks or repairs them. If technically feasible, the Gateway returns a specific missing-Project response for linked hostnames whose Project path is missing.

If the derived Project hostname is already assigned to another Project, `pv link` fails with a clear collision error. The user can pass an explicit Project hostname with `--hostname <hostname>` to resolve the collision.

When a newly linked resource-only Project is later enabled for serving without an established real hostname, PV derives its initial hostname from its immutable Project slug. Normal hostname collision and document-root validation applies, and a collision fails clearly instead of silently choosing a different hostname.

`--hostname` accepts either a bare label or a full `.test` hostname. A bare label always normalizes to `<label>.test`; for example, `--hostname acme` and `--hostname acme.test` both normalize to `acme.test`. Multi-label hostnames must be provided in full, such as `api.acme.test`. PV v1 rejects non-`.test` hostnames.

Primary Project hostnames may contain multiple labels as long as they end in `.test`, such as `api.acme.test`.

If `pv link` is run for an already linked Project with a different `--hostname`, PV updates that Project's hostname after checking for collisions, then requests reconciliation.

Changing a Project's primary hostname triggers full Project reconciliation, including additional hostname validation, Gateway routing updates, certificate configuration updates, and PV-managed `.env` updates when the Project has opted into env rendering.

If `pv link` is run for an already linked Project with the same hostname, it is idempotent: PV refreshes desired state, requests reconciliation, reports that the Project was already linked, and exits successfully.

If `pv link` is run for an already linked Project without `--hostname`, PV preserves the existing Project hostname, including any previously configured custom hostname.

## Project Unlinking

`pv unlink` with no argument unlinks the Project resolved from the current directory, using the nearest linked Project ancestor rule.

`pv unlink <selector>` unlinks the Project resolved by an exact Project slug or hostname. Hostname selectors accept the same forms as `pv link --hostname`, so `acme` and `acme.test` can normalize to `acme.test`. Additional hostnames declared in `hostnames:` may also resolve the owning Project. Output identifies the owning primary Project hostname for a served Project and the Project slug for a resource-only Project.

If a bare selector could resolve both an exact Project slug and a normalized hostname belonging to different Projects, PV fails clearly and suggests using the full `.test` hostname to select the served Project.

`pv unlink <additional-hostname>` does not require confirmation in v1 because unlink is non-destructive, but output must make the resolved primary Project explicit.

`pv unlink` exits non-zero if the target cannot be resolved to a linked Project.

`pv unlink` removes the Project from desired state and requests reconciliation. It never deletes the Project directory. Resource allocations, databases, Redis data, RustFS data, and other managed service data remain unless a separate destructive command is introduced. PV stops reconciling and watching them for that Project.

If PV previously generated a Project `.env` block, `pv unlink` leaves the block in place and stops updating it.

## Opening Projects

`pv open [hostname]` opens a served Project hostname in the user's browser from desired state. It does not require observed state to confirm that the Project is currently reachable.

With a hostname argument, `pv open <hostname>` opens that Project directly. With no argument, it opens the current Project or falls back to the picker.

When opening a Project without a hostname argument, PV opens the primary Project hostname even if the Project has additional hostnames.

The hostname argument accepts the same normalized forms as `pv unlink`, so `acme` and `acme.test` both resolve to `acme.test`.

If the hostname argument matches an additional hostname from a Project's `hostnames:`, `pv open` opens that exact hostname.

If no current Project can be resolved and the terminal is non-interactive, `pv open` exits non-zero unless a hostname argument was provided.

Current-directory Project resolution walks up from the current directory to linked Project roots, stopping at the filesystem root. If linked Projects are nested, PV chooses the nearest linked Project ancestor. This applies to commands that resolve a Project from the current directory.

An explicit resource-only Project target returns a clear non-zero error and never launches a browser. Resource-only Projects are excluded from the interactive picker.

`pv open` is Project-focused and does not open Managed Resource dashboards.

`pv mailpit:open` / `pv mail:open` opens the Mailpit dashboard only when Mailpit is already running. It does not start Mailpit or change desired state.

`pv rustfs:open` / `pv s3:open` opens the RustFS console only when RustFS is already running. It does not start RustFS or change desired state.

If `pv open` is run outside a linked Project, it shows a picker of linked Projects and opens the selected Project in the user's browser.

The picker displays each Project's primary hostname first, followed by its canonical absolute path. For example: `acme.test  /Users/me/Code/acme`. Additional hostnames are not separate picker entries in v1.

The picker sorts Projects by primary Project hostname.

## Listing Projects

`pv list` lists desired Projects and enriches them with the Project config and env rendering status currently stored in `pv.db`. At minimum, each row includes separate Project and Mode fields, canonical absolute path, resolved PHP version, declared Managed Resource demand, env rendering status, and currently known serving status. Mode is `served` or `resource-only`; serving observation may remain `unknown` until reconciliation.

Status values use words such as `ok`, `pending`, `failed`, `degraded`, or `unknown` by default. TTY output may add color or icons, but words remain present.

If the daemon has not reconciled a Project yet, the Project still appears with pending or unknown observed status.

In the Project field, `pv list` normally shows the primary hostname for a served Project and the immutable slug for a resource-only Project. It may show a compact indicator for additional hostnames, such as a count. Full additional hostname detail belongs in broader status/detail output.

`pv list --json` includes `mode`, `slug`, nullable `hostname`, and `env_file` for every Project. The internal `.invalid` compatibility hostname is represented as `null`, never as a string.

`pv list` does not show Resource allocation details by default. It stays focused on Project-level serving status.

`pv list` sorts Projects by their displayed Project value by default.

## System Status

`pv status` reports whole-system PV status. It is not scoped to the current Project.

The status output should include daemon state, Gateway state, DNS resolver state, `pf` redirect state, CA trust, installed Managed Resources, and any failed or pending Projects. It should not duplicate the full `pv list` Project table.

`pv status` derives aggregate health from existing current state in v1: PV-owned files, LaunchAgent registration, socket health, SQLite state, runtime observed state, Project env observed state, recent jobs, installed Managed Resource tracks, port assignments, DNS, `pf`, CA, and read-only platform inspectors. PV does not need separate persisted aggregate system-health or Project-health subjects for the first status implementation.

### Current Failures and Job History

Aggregate health uses current unresolved failures, not every retained failed job. Daemon jobs remain immutable historical records for `pv jobs`; a later success never deletes, rewrites, or changes the status of an earlier failure. PV instead treats each failure as an episode for a diagnostic subject. A diagnostic subject identifies both the affected component and the condition that was verified, such as Gateway runtime readiness, one Project's env rendering, or one Managed Resource track's runtime health. Outcomes for unrelated subjects do not supersede each other.

A failed job outcome remains current until PV records newer successful verification for the same subject. Successful verification may come from a completed job whose recorded reconciliation coverage contains that subject or from a newer healthy runtime observation that directly verifies the same condition. A generic healthy process observation does not clear a config, env-rendering, allocation, or privileged-integration failure that it did not verify. Job outcomes use a persisted monotonic sequence assigned when each success or failure is committed, so completion order rather than job start time is the total order for overlapping jobs. A healthy observed-state record supersedes a job failure only when its persisted observation timestamp is strictly later than the failure completion timestamp; equal timestamps do not supersede the failure. Unrelated newer work has no effect.

Observed-state failures use the existing latest-record model independently of job failure episodes. A later observed-state write for the same runtime or Project-env subject replaces the previous observation in database write order. Successful job coverage never directly supersedes an observed-state failure; successful reconciliation of that condition must also persist a matching healthy observation, or the observed failure remains current. Cross-source ordering is therefore intentionally asymmetric: healthy observation may resolve an older job failure under the strict timestamp rule above, while job success resolves only job diagnostic outcomes and relies on the reconciliation path's observed-state write to resolve observed failure.

Job supersession uses job kind plus recorded work coverage, not the requested scope string alone. Scope remains useful request and history metadata, but it is not proof that a component was reconciled. Coverage is recorded per subject only when that subject's work completed successfully and verified its condition; a successful parent job does not cover a Project or resource whose per-subject work failed. A completed `reconcile system` records the system orchestration subject plus the global Gateway, linked Projects, and demanded Managed Resources that succeeded in that run. The system orchestration subject does not implicitly cover every component subject. A successful Project reconciliation covers the selected Project and the global Gateway because the Project path invokes Gateway reconciliation. Gateway-runtime reconciliation covers the Gateway. Backing Managed Resource reconciliation covers the resource track and only the Project state successfully refreshed by that operation. Only completed successful subject coverage clears failures; failed or incomplete per-subject work does not clear an earlier failure unless a newer healthy observed-state record independently verifies the subject.

An `update system` job that finds no installed updates or no changed artifacts verifies only the update assessment. It does not cover reconciliation and cannot clear a reconciliation or runtime failure. When an update changes artifacts and subsequently performs reconciliation successfully, its recorded coverage includes the subjects actually reconciled in addition to the update assessment. This distinction prevents a no-op update from hiding an unverified system failure.

Failure deduplication applies only within one continuous unresolved episode. A repeated background error for the same subject and equivalent error may be coalesced while no newer success exists. Once newer successful coverage or a matching healthy observation resolves the episode, the same error recurring later starts a new current failure and is recorded again. Searching retained history for any matching error string is never sufficient deduplication.

`pv status` and `pv doctor` select repair advice from the current failure's typed subject and condition. They prefer the narrowest command that can repair and then verify that subject, such as `pv daemon:restart`, `pv dns:install`, `pv ports:install`, `pv ca:trust`, or an applicable Managed Resource command. They use broader `pv restart` or `pv setup` guidance only when no focused command covers the failure. Historical failures remain visible through `pv jobs` but do not make aggregate status fail after their subjects have newer successful verification.

`pv status` shows the log directory and a summary of the most recent daemon or reconciliation errors without dumping full logs by default.

`pv status` may show Managed Resource health and ports, but it must not print credentials or secrets.

`pv status` distinguishes daemon states from PV-owned plist state, read-only `launchd` state, and socket health. If PV owns a LaunchAgent plist, status may use a read-only `launchctl print` inspection to distinguish loaded, running, and down states, but the lifecycle model remains `pv daemon:enable`, `pv daemon:disable`, and `pv daemon:restart`.

`pv logs` shows daemon logs by default. Daemon logs include the structured PV daemon log, such as `~/.pv/logs/daemon.log`, plus LaunchAgent stdout/stderr logs so startup failures are visible before structured daemon logging initializes. Flags may include `--follow`, `--gateway`, `--worker <php-track>`, and `--all` for broader log streams.

`pv logs` shows the last 100 lines by default. Users can change the number of lines with `-n <lines>`.

`pv logs -n` rejects negative values and caps the maximum at 5000 lines to avoid accidentally dumping huge logs into the terminal.

When showing the last N lines, `pv logs` includes recent rotated log files if the active log file has fewer than N lines.

`pv logs --follow` shows the last N lines first, then streams new lines. `pv logs --follow -n 0` streams only new lines.

`pv logs --follow` uses rotated files only for the initial last-N output. After startup, it follows active log files only.

When `pv logs --follow` streams multiple files, PV prefixes each line with the source, such as `daemon`, `launchd:stdout`, or `launchd:stderr`.

`pv logs` may colorize source prefixes when output is an interactive TTY. Color is disabled automatically when output is piped, `NO_COLOR` is set, or the global `--no-color` flag is used. Log output may also apply minimal severity color for obvious level words: `error` and `fatal` as red, `warn` and `warning` as yellow, and `debug` and `trace` as dim text. The words remain present in the output.

`pv logs --all --follow` includes every PV-owned log stream, including daemon, LaunchAgent, Gateway, Project-serving workers, and Managed Resource logs, with source prefixes.

`pv logs --gateway` shows Gateway access, error, and supervisor logs by default when split Gateway logs exist, in that order. When following multiple streams, PV prefixes lines with sources such as `gateway:access`, `gateway:error`, and `gateway:supervisor`. A combined v1 Gateway log remains supported and is labeled `gateway` instead of requiring a runtime log-layout redesign.

`pv logs --worker <php-runtime>` accepts explicit PHP runtime identities, such as `8.4` or `8.4+redis`, and `latest`. `latest` resolves to the manifest default PHP track without Project-level optional extensions. If the resolved runtime has no log file, PV prints a clear message that no logs exist for that PHP runtime.

`pv logs` supports Managed Resource log filtering with flags such as `--resource mysql --track 8.0`, matching the resource/track log layout.

If `pv logs --resource <name>` is used without `--track`, PV infers the track only when one track is installed for that Managed Resource. If multiple tracks are installed, PV requires `--track` and lists available tracks.

`pv logs --resource <name> --track latest` resolves `latest` to the manifest default track for that Managed Resource.

`pv logs --resource` accepts the same aliases as resource command namespaces, such as `pg`, `mail`, and `s3`, and normalizes them internally to canonical names: `postgres`, `mailpit`, and `rustfs`.

`pv doctor` is a deeper read-only diagnostic than `pv status`. It checks expected files, permissions, ports, resolver behavior, `pf` rules, LaunchAgent registration, manifest cache, and common conflicts, then suggests repair commands. Failed checks should prefer one focused repair command where a command exists, such as `pv daemon:enable`, `pv daemon:restart`, `pv dns:install`, `pv ports:install`, or `pv ca:trust`; otherwise PV may suggest `pv setup` as the broad repair.

`pv jobs` is a read-only diagnostic command that lists recent daemon jobs, including setup, install, update, restart, and reconciliation jobs. It shows status, scope, start/end time, and failure summary. Live progress remains attached to the command that started the job.

Read/status commands support `--json` output in v1, including `pv status`, `pv doctor`, `pv ports:status`, `pv list`, `pv project:env`, `pv jobs`, `pv update --check`, and Managed Resource list commands. `pv update --check --json` is valid, while bare `pv update --json` is invalid because update progress is not a JSON stream in v1. Mutating progress-stream commands do not need JSON output in v1 unless it is cheap to provide. JSON output should use minimal command-specific objects and arrays that map directly to the command output; v1 does not require envelope metadata such as `schema_version` or CLI version fields unless a later automation contract needs them.

`pv update --check --json` belongs to the self-update and update-orchestration slice even though it is part of the v1 JSON surface. Diagnostics work should not change update-check behavior.

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
| pv project:env [selector] [--json] | Print generated Project environment values without editing the configured env file                          |
| pv list [--json]         | List linked Projects with PHP, declared Managed Resource demand, env status, and currently known serving status    |
| pv logs [--follow]       | Show PV daemon/reconciliation logs                                                                                |
| pv status [--json]       | Show whole-system PV status                                                                                       |
| pv setup [--yes] [--non-interactive] [--no-path] | Configure macOS resolver, `pf` redirects, CA trust, daemon registration, and default Managed Resources |
| pv uninstall [--prune] [--force] | Uninstall PV, preserving data by default                                                                  |
| pv unlink [selector]     | Unlink a Project by current directory, Project slug, or Project hostname                                           |
| pv update [--check] [--json] | Update the PV application and installed Managed Resources to their latest versions, or report available updates with `--check`; `--json` requires `--check` |
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
| pv doctor [--json] | Run read-only diagnostics for setup, DNS, ports, CA, daemon, Gateway, manifest cache, conflicts |
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
| pv ports:status [--json] | Show verified PV `pf` redirect state for loopback `80`/`443` |
| pv ports:install   | Install or repair PV's `pf` redirect rules            |
| pv ports:uninstall | Remove PV's `pf` redirect rules                       |

## PHP + Frankenphp

| command                                           | what it does                                                                  |
| ------------------------------------------------- | ----------------------------------------------------------------------------- |
| pv php:use <version> [--global]                   | Set the Project PHP track, or the global default with `--global`/`-g`. Installs matching PHP and FrankenPHP artifacts before recording the selection. |
| pv php:install [version]                          | Install a PHP track (e.g., pv php:install 8.4). Uses the manifest default track if omitted. |
| pv php:update                                     | Update all installed PHP tracks and matching FrankenPHP artifacts              |
| pv php:uninstall <version> [--prune] [--force]    | Uninstall a PHP/FrankenPHP track pair. `--force` bypasses active selection guards. |
| pv php:list [--json]                              | List installed PHP tracks                                                     |

## Composer

| command                                      | what it does                                    |
| -------------------------------------------- | ----------------------------------------------- |
| pv composer:install                          | Install Composer track `2` and ensure the resolved global/default PHP and FrankenPHP pair is installed |
| pv composer:uninstall [--prune] [--force]    | Remove Composer track `2`. Preserve Composer home/cache unless `--prune` is provided; `--force` bypasses removal guards. |
| pv composer:update                           | Update Composer track `2` to the latest non-revoked artifact |

## Postgres (Alias: pg)

| command                                 | what it does                                                                      |
| --------------------------------------- | --------------------------------------------------------------------------------- |
| pv {postgres or pg}:install [version]   | Install a Postgres track. Uses the manifest default track if omitted. |
| pv {postgres or pg}:uninstall <version> [--prune] [--force] | Uninstalls a Postgres track. `--force` bypasses active-use guards and prune confirmation. |
| pv {postgres or pg}:update              | Update all installed Postgres tracks                                              |
| pv {postgres or pg}:list [--json]       | List installed Postgres tracks                                                    |

## Mysql

| command                      | what it does                                                                   |
| ---------------------------- | ------------------------------------------------------------------------------ |
| pv mysql:install [version]   | Install a MySQL track. Uses the manifest default track if omitted. |
| pv mysql:uninstall <version> [--prune] [--force] | Uninstalls a MySQL track. `--force` bypasses active-use guards and prune confirmation. |
| pv mysql:update              | Update all installed MySQL tracks                                               |
| pv mysql:list [--json]       | List installed MySQL tracks                                                     |

## Mailpit (Alias: mail)

| command                                  | what it does                                                                     |
| ---------------------------------------- | -------------------------------------------------------------------------------- |
| pv {mailpit or mail}:install [version]   | Install a Mailpit track. Uses the manifest default track if omitted. |
| pv {mailpit or mail}:uninstall <version> [--prune] [--force] | Uninstalls a Mailpit track. `--force` bypasses active-use guards and prune confirmation. |
| pv {mailpit or mail}:update              | Update all installed Mailpit tracks                                              |
| pv {mailpit or mail}:list [--json]       | List installed Mailpit tracks                                                    |
| pv {mailpit or mail}:open                | Open the running Mailpit dashboard                                               |

## Rustfs (Alias: s3)

| command                               | what it does                                                                    |
| ------------------------------------- | ------------------------------------------------------------------------------- |
| pv {rustfs or s3}:install [version]   | Install a RustFS track. Uses the manifest default track if omitted. |
| pv {rustfs or s3}:uninstall <version> [--prune] [--force] | Uninstalls a RustFS track. `--force` bypasses active-use guards and prune confirmation. |
| pv {rustfs or s3}:update              | Update all installed RustFS tracks                                               |
| pv {rustfs or s3}:list [--json]       | List installed RustFS tracks                                                     |
| pv {rustfs or s3}:open                | Open the running RustFS console                                                 |

## Redis

| command                      | what it does                                                                   |
| ---------------------------- | ------------------------------------------------------------------------------ |
| pv redis:install [version]   | Install a Redis track. Uses the manifest default track if omitted. |
| pv redis:uninstall <version> [--prune] [--force] | Uninstalls a Redis track. `--force` bypasses active-use guards and prune confirmation. |
| pv redis:update              | Update all installed Redis tracks                                               |
| pv redis:list [--json]       | List installed Redis tracks                                                     |
