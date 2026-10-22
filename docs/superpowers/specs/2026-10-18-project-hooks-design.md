# Project Hooks Design

Status: proposed 2026-10-18. The owner decided D1–D13 between 2026-10-16 and 2026-10-18; D14 follows from D4.

## Summary

A Project can list shell commands in its `pv.yml` under `hooks.prepare` and `hooks.setup`. PV runs them when the Project is linked, so a new checkout ends up usable without anyone remembering `composer install`, `pnpm build` or the migrations:
- `pv link` runs both lists in the terminal.
- Worktree discovery runs both lists in the daemon, in the background.

PV doesn't judge, classify or limit the commands. It runs them in order, with the user's own shell environment, and reports what happened.

```yaml
hooks:
  prepare:
    - cp "$PV_SOURCE_PATH/auth.json" auth.json
  setup:
    - composer install
    - pnpm install
    - pnpm build
    - php artisan migrate --force
```

## Motivation

- **Discovered worktrees are served but not usable.** Worktree discovery (#364, #376) ran on a laptop with many worktrees. Every worktree got a hostname, a copied env file and its own Resource allocations, but:
  - The database wasn't migrated. Most of those Projects use SQLite, and the untracked `database/database.sqlite` file didn't exist.
  - Vite assets weren't built, so every page failed until someone ran `pnpm dev` by hand in that worktree.
  - Agents don't do these steps reliably. The owner usually views the worktrees in a browser and ended up running them by hand.
- **Only PV knows when a worktree is ready for these commands.** A migration needs the env block PV renders after it creates the worktree's databases. An agent tool's own setup script runs when the worktree is created, before PV has found it, so its `migrate` would hit a missing or wrong database.
- **The idea was already written down.** IDEAS.md described a hook runner for `pv link` (prepare and setup lists, `/bin/sh -c`, fail fast, `--skip-hooks`). It assumed a terminal and said the watcher must never run hooks. Discovery has no terminal, so this design adds a daemon side and settles the IDEAS.md open questions.

## Goals

- A Project declares `hooks.prepare` and `hooks.setup` in its own `pv.yml`.
- Hooks run when the Project is linked: on every `pv link`, and when discovery links a new worktree. Nothing else runs them.
- `prepare` runs before PV records the Project and before PV allocates anything or writes its env block. `setup` runs after the Project's reconciliation has rendered the env block.
- Hook commands get the user's current login-shell environment, worked out fresh for each run, never saved.
- `pv link` shows hook output live and exits non-zero when a hook fails.
- The daemon runs background hooks for up to 4 Projects at once, outside the job queue, so a long `pnpm install` never holds up other daemon work.
- Hook runs are recorded as jobs. `pv status` shows a failed run until the next successful one, and a background run's output is kept in a log file.

## Non-Goals

- Long-running Project processes. `pnpm dev`, queues, Horizon and similar stay excluded (DESIGN.md: "PV v1 does not manage Project background processes"). `pnpm build` gives a page that works, frozen at the time the hook ran.
- Holding a discovered worktree's route until `setup` finishes. The serving gate stays as it is, so a page may fail while `setup` is still running.
- Running hooks for Projects that are already linked when this ships.
- Deciding whether a command is safe or needed. There is no trust gate, no allow-list and no idempotency tracking. What the commands do is the user's business.
- Running a Project's commands in parallel. They run in order; a hook can start parallel work with shell syntax.
- Hooks on unlink, on cleanup or after the Project is reachable (`cleanup` and `ready` in IDEAS.md).
- `pv init` writing hooks.
- A `pv logs` flag for hook output. `pv status` and `pv jobs` print the log path of daemon runs.
- Command timeouts.

## Decisions

| | Decision | Outcome |
|---|---|---|
| D1 | Should PV run Project-declared commands at all? | Yes. PV still never decides to run `composer install` or migrations; it runs what the Project's config declares (owner, 2026-10-16) |
| D2 | Trust gate for commands from a branch's `pv.yml` | None. A discovered worktree runs what its own `pv.yml` declares (owner, 2026-10-16) |
| D3 | Environment for hook commands | The user's login-shell environment, captured fresh for every run and never saved (owner, 2026-10-16) |
| D4 | What triggers hooks | Linking only: every `pv link`, and discovery linking a new worktree. PV keeps no "hooks ran" record to decide whether to run them (owner, 2026-10-16) |
| D5 | Concurrency | Commands within a Project run in order. The daemon runs up to 4 Projects at once, like `MAX_PARALLEL_ARTIFACT_DOWNLOADS` (owner, 2026-10-16) |
| D6 | Keep `prepare` | Yes, alongside `setup` (owner, 2026-10-16) |
| D7 | Built-in worktree env copy | Kept. It runs first, then `prepare`, which may change or replace the copied file (owner, 2026-10-16) |
| D8 | Shell for the commands | `/bin/sh -c`. The login shell only supplies variables, so one committed `pv.yml` works for zsh, bash and fish users alike (owner, 2026-10-16) |
| D9 | What is captured | The whole environment, not only `PATH`. Tools rely on variables such as `COMPOSER_AUTH` and `PNPM_HOME` (owner, 2026-10-16) |
| D10 | `pv link` with setup hooks | Waits for the accepted reconciliation job to succeed, then runs `setup` in the terminal (owner, 2026-10-18) |
| D11 | Reporting background runs | A job record per run, plus a log file per subject, replaced at each run. Completed records provide history and failures, not idempotency tracking; queued/running rows exclude concurrent foreground hooks (owner, 2026-10-18) |
| D12 | Main checkout path for worktree hooks | `PV_SOURCE_PATH` (owner, 2026-10-18) |
| D13 | A failed `prepare` on a discovered worktree | Remembered in daemon memory; retried after the daemon restarts. Not an opt-out, because `pv link` on an opted-out worktree would make it an ordinary Project (owner, 2026-10-18) |
| D14 | `pv link` before `pv setup` | `prepare` runs; `setup` is skipped with a warning to run `pv link` again after `pv setup`. No deferred-hook marker (follows D4) |

D2 accepts credential exposure as well as command execution: branch hooks receive the whole login-shell environment and can read and send away credentials such as `COMPOSER_AUTH`. Branch code PV serves and Composer/pnpm scripts already run as the user and can read those same secrets from files.

## Owner-approved review amendments (2026-10-19)

These amendments override the original design where they differ.

- **A1. Credential exposure.** Hooks from a branch run with the user's whole environment, so they can read and send away credentials such as `COMPOSER_AUTH`. Accepted: branch code PV serves, and `composer`/`pnpm` scripts, already run as the user and can read the same secrets from files. Add this to D2's rationale and to Risks.
- **A2. Foreground runs and cleanup.** "Stop hooks before cleanup deletes allocations" covers daemon runs only. A `pv link` run belongs to the user's terminal; removing a worktree while you run `pv link` in it is like deleting a folder mid-`composer install`. No handoff between the CLI and the daemon.
- **A3. Record queued runs.** The daemon creates the `hooks` job row when it queues a run, not when the run starts. That has two effects:
  - Daemon startup marks queued runs failed along with running ones, so a lost `setup` shows up in `pv status`.
  - `pv link`'s "daemon hooks in progress for this Project" check also sees queued runs, so the spec's "A `setup` queued but not started" risk goes away.
- **A4. Remember a successful `prepare`.** Besides the running and failed sets, the daemon keeps a prepared set:
  - A successful `prepare` puts the worktree ID in it.
  - Discovery links a prepared worktree without running `prepare` again, and removes it from the set once linked.
  - A daemon restart empties the set, so `prepare` runs again. The spec already accepts that.
- **A5. Lifecycle admission.** `pv link` holds its shared runtime lifecycle admission all the time, except while hook commands run:
  - Release it right before a hook list starts and reacquire it right after.
  - If reacquiring fails (`pv daemon:disable` or `pv uninstall` took it), stop without recording state or contacting the daemon, and exit non-zero. Also abort before opening the database or IPC if `pv.db` existed at command start but disappeared during hooks, as after `uninstall --prune`.
- **A6. Waiting on a coalesced job.** `pv link` waits on the job ID in the daemon's accepted response. That is either its own new job or the existing queued job it was merged into (`reconciliation already queued or running`). It reads that job's record in `pv.db` until the job finishes, and runs setup only if that accepted job succeeds. If it fails, skip setup, show the job's error and say to run `pv link` again once it is fixed. Print the accepted job ID and keep waiting through daemon restarts. No protocol change, and no reuse of the streaming setup client.
- **A7. Two `pv link` runs at once.** Not handled. Two `pv link` runs in one folder at once is the same as running `composer install` twice by hand. A3 covers daemon-versus-terminal.
- **A8. Hook logs.**
  - One log per subject, replaced at each run: `~/.pv/logs/hooks/project-<project-id>.log`, or `~/.pv/logs/hooks/worktree-<source-project-id>-<worktree-id>.log` for a `prepare` before linking.
  - Worktree cleanup deletes that worktree's logs.
  - No pruning sweep. Drop the spec's "logs never outlive their records" promise.

## Design

### Config

`hooks` is a new top-level key in Project config:
- It is a mapping with two optional keys, `prepare` and `setup`.
- Each value is a list of non-empty strings, one shell command per item. A single string instead of a list is an error.
- Unknown keys under `hooks` fail validation, like every other unknown key.
- `hooks: {}` and empty lists are valid and mean no hooks.
- It is valid in both serving modes.
- Like other config errors, invalid hooks keep the last valid desired state active. `pv link` fails before running anything.

`hooks` is part of `ProjectConfig`, so PV keeps it when it rewrites `pv.yml`. `pv php:use` serializes the whole model through `write_project_config`, so a key outside the model would be lost.

A discovered worktree uses only its own `pv.yml`, as today. Its hooks are the ones committed on its branch.

### When hooks run

| Event | `prepare` | `setup` |
|---|---|---|
| `pv link`, first or again | Yes, in the terminal, before PV records the Project | Yes, in the terminal, after the accepted reconciliation job succeeds |
| `pv link --skip-hooks` | No terminal hooks | No terminal hooks; daemon setup still runs on a discovered Project's first-success transition |
| Discovery links a new worktree | Yes, in the daemon, after the env copy and before linking | Yes, in the daemon, after the Project's first successful reconciliation |
| Editing `pv.yml`, any other reconciliation, daemon start, `pv setup`, `pv update`, `git worktree move` | No | No |
| Projects already linked when this ships | No | No (see below for one exception) |

PV keeps no persistent completed-hook marker to decide whether to rerun hooks. A Project that is already linked does not get them automatically, and running `pv link` again reruns them. Discovery remembers successful prepare runs in memory only until linking, as described below.

Exception: a discovered Project linked before this change that has never finished its first reconciliation (`first_reconciled_at` is null) runs `setup` when it does. Such a Project was never usable, so this is the first link that counts.

### Running a hook list

One hook run is one Project's `prepare` list or `setup` list. A `pv link` invocation is a single run that covers both.

1. **Capture the environment.** Once per run, as described in the next section.
2. **Run each command** as `/bin/sh -c '<command>'`:
   - The working directory is the Project root.
   - stdin is `/dev/null`, everywhere, so a hook that works with `pv link` also works in the daemon.
   - Each command gets its own process group, so stopping a run stops everything it started.
   - When a command exits, PV stops anything it left running in its process group. A `php artisan serve &` started by a hook is stopped.
3. **Stop at the first command that exits non-zero.** The run fails, naming the command and its exit status.

On macOS `/bin/sh` is bash in POSIX mode. On Debian and Ubuntu it is dash. Hooks should use POSIX syntax. Parallel work needs a `wait` per process, because a plain `wait` exits 0 even when a job failed:

```sh
pnpm install & a=$!; composer install & b=$!; wait $a && wait $b
```

**Command environment**, in order of precedence (highest first):
1. `PV_SOURCE_PATH`, for discovered worktree Projects only: the source Project's path, which sits in the main checkout. It is unset for ordinary Projects.
2. PV's Composer overlay, the same values the `composer` shim sets: `~/.pv/bin` and `~/.pv/composer/vendor/bin` at the front of `PATH`, `COMPOSER_HOME` and `COMPOSER_CACHE_DIR`.
3. The captured login-shell environment.

There is no command timeout:
- In the terminal, Ctrl-C stops the running command's process group.
- In the daemon, a run stops when its Project is unlinked, when Git removes its worktree, or when the daemon stops.

### Capturing the login-shell environment

The daemon is a LaunchAgent with no `EnvironmentVariables`, so it has launchd's bare `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`). Under it, `pnpm` and `node` installed through Homebrew, fnm, nvm or mise aren't found. Saving a `PATH` at link time goes stale. fnm, for example, puts a different directory on `PATH` for every shell, such as `~/.local/state/fnm_multishells/<pid>_<time>/bin`. So PV asks the user's shell each time:

1. **Find the login shell** from the account record (`nix::unistd::User::from_uid`), not `$SHELL`.
2. **Start it interactive and login** (`<shell> -i -l -c '<script>'`):
   - It starts in `$HOME`, with the Project root passed in `PV_HOOK_PROJECT`.
   - The script runs `cd "$PV_HOOK_PROJECT"` first, so version managers that switch on directory change (fnm `--use-on-cd`, mise) pick that directory's versions, as they would in a terminal.
   - It then prints a start marker, `/usr/bin/env -0` and an end marker.
   - The script uses only syntax that zsh, bash, sh and fish all accept.
3. **Guard against shell startup quirks:**
   - Output outside the markers is ignored, so a welcome banner or a tool's warning is harmless.
   - stderr is kept only to explain a failure.
   - stdin is `/dev/null`, so a startup prompt such as an oh-my-zsh update question gets an end of input straight away instead of waiting.
   - A 10-second timeout kills the shell's process group.
4. **Parse the NUL-separated variables,** then drop shell bookkeeping: `PWD`, `OLDPWD`, `SHLVL`, `_` and `PV_HOOK_PROJECT`.

If the capture fails (timeout, non-zero exit, no markers), the run fails before any command starts. The error says "couldn't load your shell environment" and shows the end of the shell's stderr.

Captured values never go into logs, job records or error messages. They routinely contain secrets; the owner's shell exports an API key, for example.

Measured on the owner's laptop on 2026-10-16, starting from a launchd-like environment: 0.16 s, no output outside the markers, 37 variables, and `pnpm`, `node`, `composer` and `php` all resolved. With the bare daemon `PATH`, all four were missing.

Known limit: tools that load only after the first prompt (zinit turbo mode, lazy nvm loaders) never load under `-c`. Users fix that in their shell config, or with absolute paths in the hook.

### `pv link`

1. **Read and validate Project config,** as today. If the Project declares hooks, is discovered and `first_reconciled_at` is still null, fail before running anything: PV runs this worktree's hooks automatically after its first successful reconciliation; check `pv jobs`, or pass `--skip-hooks` to link without terminal hooks. Otherwise, if the Project declares hooks and the daemon is running hooks for this linked Project (a queued or running `hooks` job for it), fail before running anything and say to wait for that job, or pass `--skip-hooks`. Two runs of `composer install` in one directory at once can break it.
2. **Run `prepare`** in the terminal, unless `--skip-hooks` is passed or there is no `prepare` list. On failure:
   - Exit non-zero.
   - On a first link, nothing is recorded.
   - On a relink, the existing record is left as it was and no reconciliation is requested.
3. **Record desired state and request reconciliation,** as today.
4. **Run `setup`,** if the Project has a `setup` list and `--skip-hooks` wasn't passed:
   - Wait for the job ID in the daemon's accepted response by reading its record in `pv.db` until it finishes. The ID may name a new job or an existing queued/running reconciliation into which the request was coalesced. Print the accepted job ID while waiting, and keep waiting through daemon restarts. Run setup only when that accepted job finishes successfully. This uses the existing protocol and does not reuse the streaming setup client.
   - If the accepted job record disappears, exit non-zero: its outcome is unknown, setup was not run, and the user should run `pv link` again.
   - If the accepted job fails, skip `setup`, exit non-zero, show the job's error and say to run `pv link` again once it is fixed. No per-Project outcome recording is added.
   - If the daemon isn't running (including before `pv setup`), skip `setup`. Warn that it runs when `pv link` is run again after `pv setup`, and exit zero, matching how `pv link` behaves before setup today.
   - Otherwise run `setup` in the terminal. On failure, exit non-zero. The Project stays linked with its allocations and env block.
5. **Record the run as a `hooks` job** for the Project when it finishes (next section). A `prepare` failure on a first link has no Project to record against, so it appears only in the terminal.

Details:
- The guards cover linked Projects only. Before discovery links a worktree, `pv link` can run prepare there and link it as an ordinary Project. Discovery forgets that worktree from its running, prepared and failed sets and records a successful skipped prepare for its worktree scope, clearing any earlier failure. A daemon prepare finishing afterwards cannot link it or trigger setup.
- If discovery links the worktree while `pv link` runs prepare there, `pv link` reads the Project again before recording it: it keeps the discovered Project's hostname and applies the guards again. Without this, `pv link` would replace the worktree's hostname with a directory-derived one and run setup alongside the daemon's.
- `pv link` holds shared runtime lifecycle admission throughout the command, except while hook commands run. Release it immediately before each hook list and reacquire it immediately afterwards. If reacquiring fails because `pv daemon:disable` or `pv uninstall` took it, exit non-zero without recording state or contacting the daemon. If `pv.db` is missing after a hook list (`pv link` opens it before any hook runs), abort before opening the database or making IPC requests, so a completed `uninstall --prune` cannot recreate `~/.pv`. A completed disable or plain uninstall needs no extra detection. Otherwise a long `setup` would block disable or uninstall.
- Two concurrent `pv link` invocations in the same folder are not coordinated, like running `composer install` twice by hand.
- `pv link` without setup hooks stays a fast intent command that returns once the daemon accepts reconciliation.
- The `RunJob.requested_by_link` marker is removed, with no replacement suppression marker. Every discovered Project first-success transition queues daemon setup, whoever requested reconciliation. `pv link --skip-hooks` can trigger it; the first-reconciliation guard prevents terminal hooks before that transition.
- Each command is announced on its own line before its output, such as `Running setup: pnpm build`. Command output is passed through unchanged.
- Ctrl-C stops the running command, records the run as interrupted and exits non-zero.

### Discovered worktrees

For a newly found worktree (not opted out, its directory exists, its config can be read):
1. **PV copies the source's env file** if the worktree has none, as today.
2. **If the worktree's config has a `prepare` list,** PV doesn't link it yet:
   - It hands the worktree to the hook runner, outside the job queue.
   - While that `prepare` is queued or running, discovery and the 30-second health scan leave that worktree alone.
   - When `prepare` succeeds, the runner adds the worktree ID to an in-memory prepared set and requests system reconciliation. Discovery and the health scan honour that set: discovery links the prepared worktree without another `prepare` and removes it from the set once linked. The set is empty after a daemon restart.
   - When `prepare` fails, the daemon remembers the worktree ID in memory. Discovery and the health scan skip the worktree until the daemon restarts. The failure says to fix the cause and run `pv daemon:restart`. Nothing is written to `worktree_opt_outs`, so the worktree stays a discovered worktree.
   - If the daemon restarts at any point before the worktree is linked, `prepare` runs again on the next discovery.
3. **Without a `prepare` list,** discovery links the worktree straight away, as today.
4. **Run `setup`.** When a discovered Project's allocations and env first succeed (`first_reconciled_at` goes from null to set) and its applied config has a `setup` list, the existing finalization transaction also inserts the queued `hooks` job. The caller hands that job ID and the applied commands to the runner. Setup runs even if Gateway or another Project later fails the enclosing job, whoever requested reconciliation. A crash after commit leaves a queued job that startup marks failed.

If the daemon stops while `setup` is queued or running, PV doesn't resume it. Startup marks the run's job failed, as it does for every job left running, so `pv status` shows it, and `pv link` in the worktree reruns the hooks.

The serving gate is unchanged. A worktree's route appears once its first reconciliation succeeds, which may be before `setup` finishes.

### Daemon hook runner

- Hook runs never hold `jobs.lock` and never enter the reconciliation queue. The daemon runs one job at a time, and a minutes-long `pnpm install` must not delay `pv php:install` or reconciliation.
- Up to 4 runs at once, `prepare` and `setup` alike; the rest wait in arrival order.
- The runner uses tokio child processes on the daemon's current-thread runtime, with the process-group and `kill_on_drop` pattern from `run_validation_command` (`crates/daemon/src/gateway.rs`).
- **Stopping runs:**
  - Unlinking a Project, or unlinking a source and with it its discovered Projects, stops their queued and running hooks.
  - When Git removes a worktree, PV stops its daemon hooks before cleanup deletes any allocation object, so a daemon migration never runs against a database being dropped. A foreground `pv link` belongs to the user's terminal; removing its worktree mid-command is like deleting a folder during `composer install`. There is no CLI/daemon handoff.
  - The daemon stopping kills every hook process group.

### Reporting

**Jobs:**
- Every hook run is recorded as a job of kind `hooks`. Scopes:
  - `project:<id>` for a linked Project. This covers `pv link` runs and daemon `setup` runs.
  - `worktree:<source-project-id>:<worktree-id>` for a daemon `prepare` run on a worktree that isn't linked yet. That pair is the discovered Project's identity (DESIGN.md, Git Worktrees).
- A successful run records success coverage for its scope's subject. A failed run records a failure naming the phase, the command and its exit status. A daemon run also names its log path, for example ``setup: `pnpm build` exited with status 1 (log: ~/.pv/logs/hooks/project-<project-id>.log)``. A `pv link` run has no log, because its output went to the terminal.
- Today's `JobDiagnosticSubject::from_job_identity` already maps any non-`reconcile` kind to a subject per kind and scope, so the existing current-failure rules apply unchanged: a failure stays current until a newer success for the same subject.
- `pv status` lists current hook failures, with the log path for daemon runs.
- `pv status` ignores a hook failure whose Project, or for a `worktree:` scope whose source Project, is no longer linked. A `worktree:` failure also stops counting once that worktree is linked as a discovered Project: a pending `prepare` only matters while the worktree is blocked.
- `pv jobs` lists hook runs like any other job, with the log path for daemon runs.
- The daemon creates the `hooks` job row when it queues a run, in the running state. Startup fails queued runs as well as active runs, so a lost queued `setup` is visible in `pv status`, and `pv link` can see queued daemon hooks. `pv link` records its run when it finishes. A row left running by `pv link` would be failed by a daemon restart while the command was still going.

**Logs:**
- Each daemon run writes its commands' interleaved stdout and stderr to one log per subject, replaced at each run: `~/.pv/logs/hooks/project-<project-id>.log` for a linked Project, or `~/.pv/logs/hooks/worktree-<source-project-id>-<worktree-id>.log` for a pre-link `prepare`. The directory is `0700` and each file `0600`, because command output can contain secrets.
- Worktree cleanup deletes that worktree's logs. There is no pruning sweep and no promise that logs and retained job records have the same lifetime. Historical job paths refer to the subject's latest log.
- `pv link` runs write no log; their output went to the terminal.

## DESIGN.md Changes

Applied in PR 1, alongside the config change.

- **New section "Project Hooks",** after Git Worktrees. It holds the rules from the Design section above: config, triggers, run contract, environment capture, `pv link`, discovered worktrees, the daemon runner and reporting.
- **Project Configuration and Environment:** add `hooks` to the accepted top-level keys, with a pointer to the new section.
- **Desired State and Daemon Availability** (the `pv link` and `pv unlink` fast-intent paragraphs): `pv link` waits for reconciliation when the Project has `setup` hooks.
- **Project Linking:**
  - Replace "PV v1 does not run Project package manager commands, such as `composer install`, automatically" and the Laravel-commands sentence with: PV doesn't choose to run package manager or framework commands; it runs only the commands a Project declares in `hooks`.
  - Amend "The command succeeds when…" and the before-setup paragraph to match D10 and D14.
  - Document `--skip-hooks`.
- **Project Unlinking:** unlinking stops the Project's running hooks.
- **Git Worktrees:**
  - Replace "PV does not install dependencies or run migrations, seeders, or other application commands for a worktree" with a pointer to Project Hooks.
  - Cleanup stops a worktree's hooks before deleting allocation objects.
- **Current Failures and Job History, and `pv jobs`:** hook runs are `hooks` jobs, and `pv status` shows their failures with log paths.
- **Filesystem Layout:** `~/.pv/logs/hooks/`.
- **Commands:** add `--skip-hooks` to `pv link`.

IDEAS.md loses its "Project Hooks / Command Runner" section, which this spec replaces.

## Implementation Sequence

A linear `gh stack` of four PRs.

1. **Config and DESIGN.md.**
   - Add `hooks` to `ProjectConfig`, the hand-written parser (`crates/config/src/parser.rs`) and the writer round-trip.
   - Add new `ConfigError` variants for unknown `hooks` keys and wrong value shapes.
   - Apply the DESIGN.md changes.
   - Tests: parser cases as insta snapshots next to the existing config tests, and `pv php:use` keeping `hooks` when it rewrites `pv.yml`.
2. **Hook runner.**
   - Shared by the CLI and the daemon, in a crate both depend on. The Composer overlay (`composer_env_overlay` in `crates/cli/src/commands/composer.rs`) moves there too, so the daemon can use it.
   - Pieces: environment capture (it takes the shell path as an argument; only its caller reads the account record), command execution with an output sink (terminal or file), the Composer overlay and `PV_SOURCE_PATH`.
   - Tests use a fake login-shell script to cover: banner noise, startup prompts, a hang hitting the timeout, multi-line values, dropped bookkeeping variables, fail-fast order and process-group stop.
3. **`pv link`.**
   - `prepare` before recording, waiting for reconciliation, `setup`, `--skip-hooks`, the lifecycle admission release, the not-running warning, the `hooks` job record, the first-reconciliation guard, and the post-hook prune check.
   - Integration tests in `it/` with insta snapshots of terminal output.
4. **Daemon.**
   - The runner pool, the discovery `prepare` flow with its in-memory running, failed and prepared sets (honoured by the health scan), queuing `setup` on the first-reconciliation transition, job rows created at queue time and per-subject log files removed by worktree cleanup, stopping runs on unlink, worktree removal and shutdown, and the `pv status` / `pv jobs` output.
   - Integration tests for: 6 worktrees appearing at once (at most 4 runs at a time), a `prepare` failure retried after restart, a run cut off by a restart showing as failed, cleanup waiting for a running hook, and `pv link` in a discovered worktree not causing a second `setup`.

## Verification

For every PR:
- `cargo nextest run --workspace` passes.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo fmt --all` are clean.

For the stack, on the owner's laptop with many worktrees:
- Add `hooks` (`composer install`, `pnpm install`, `pnpm build`, `php artisan migrate --force`) to a source Project's `pv.yml` on a branch, and create several worktrees from it.
- Each worktree's page loads without manual commands, and no more than 4 hook runs happen at once.
- `pv status` and `pv jobs` show the runs, and a deliberately broken command shows as a current failure with a log path.

## Risks And Mitigations

- **Load spikes.** 20 new worktrees mean up to 40 hook runs. The 4-run cap bounds the load, and Projects already linked never run hooks.
- **Agents running the same commands.** An agent that runs `pnpm install` in the same worktree at the same time can collide with PV. Once PV owns setup, remove those steps from agent setup scripts.
- **Shell startup problems.** Markers, stdin from `/dev/null` and the 10 s timeout cover noise, prompts and hangs. Lazily loaded tools stay a documented limit.
- **Secrets.** Hooks receive the whole environment and can read and send away credentials such as `COMPOSER_AUTH`. Accepted: branch code PV serves and Composer/pnpm scripts already run as the user and can read those same secrets from files (D2). PV never persists captured values. Command output can still contain secrets, so hook logs are owner-only.
- **Job history churn.** Hook runs share the 100-job history, so a burst of worktrees pushes older jobs out sooner.
- **Removed worktrees.** When Git removes a worktree, PV records a successful skipped prepare for its worktree scope, clearing any earlier prepare failure.
- **Database drops during migrations.** Cleanup stops a worktree's daemon hooks before deleting its allocation objects. Foreground runs belong to the terminal; removing a worktree during `pv link` has no CLI/daemon handoff.
- **Concurrent foreground links.** Two `pv link` runs in the same folder are not coordinated, like two manually started Composer installs. Queued daemon runs are covered by their job rows.

## Acceptance Criteria

- `hooks.prepare` and `hooks.setup` validate as described, and `pv php:use` keeps them.
- `pv link` runs `prepare` before recording the Project and `setup` only after its accepted reconciliation job succeeds, live in the terminal. If that job fails, it skips setup, reports the job error and tells the user to run `pv link` again once it is fixed. It exits non-zero on a hook failure, and `--skip-hooks` skips both terminal lists.
- `pv link` refuses terminal hooks for a discovered Project whose `first_reconciled_at` is null; `--skip-hooks` still links and permits automatic daemon setup.
- First-success finalization and the queued daemon setup job commit atomically; restart reports an interrupted setup even if the daemon died before handing it to the runner.
- `pv link` without setup hooks still returns without waiting for reconciliation.
- Discovery runs a new worktree's `prepare` before linking it and its `setup` after its first successful reconciliation. A failed `prepare` is skipped until the daemon restarts.
- No hook runs on config edits, reconciliation, daemon start, `pv setup`, `pv update` or worktree moves, and none for Projects linked before this ships.
- Hook commands run under `/bin/sh -c` with the freshly captured login-shell environment plus PV's overlay, and `pnpm` resolves from the daemon.
- The daemon runs at most 4 hook runs at once, never under `jobs.lock`.
- Every daemon hook run, and every `pv link` hook run for a recorded Project, appears in `pv jobs`. A failed daemon run shows in `pv status` with its log path until a newer success.
- `pv link` refuses to run hooks while the daemon has queued or running hooks for the same Project.
- Unlink, worktree removal and daemon shutdown stop running hooks. Cleanup never deletes an allocation object while a daemon hook for that Project is still running. Foreground runs remain owned by their terminal.

## Rejected Alternatives

- **Saving `PATH` at `pv link` or `pv setup`.** It goes stale, and fnm's per-shell directories break it immediately (D3).
- **Running commands in the user's shell (`zsh -c`).** A committed `pv.yml` would behave differently for fish and bash teammates, and aliases and functions would leak in (D8).
- **Capturing only `PATH`.** It misses credentials and tool settings exported in shell config, such as `COMPOSER_AUTH` (D9).
- **A "hooks ran" record that decides whether to run.** Linking is the trigger (D4). Job records are kept for history and failures only.
- **Opting out a worktree whose `prepare` failed.** `pv link` would then make it an ordinary Project, without worktree naming or cleanup (D13).
- **Replacing the built-in env copy with `prepare`.** Every Project would need the same command, plus a way to find the main checkout (D7).
- **Running hooks inside daemon jobs.** One job runs at a time under `jobs.lock`, so a long install would block every other mutation.
- **Holding the route until `setup` finishes.** It needs persisted "setup finished" state, and an unknown-hostname page isn't better than the app's own error while `setup` runs.
- **A trust gate.** Serving a worktree already runs its code, and `composer install` and `pnpm install` run repository scripts anyway (D2).

## Out Of Scope, Noticed

- IDEAS.md's "Worktree Environment Commands" parking-lot entry predates worktree discovery and is stale.
- There is no `pv logs` way to follow a running background hook; `tail -f` on the logged path works.
