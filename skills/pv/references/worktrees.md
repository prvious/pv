# Git worktrees under PV

PV watches the Git worktrees of every Project linked in a repository's main checkout. That includes worktrees created by agent apps, wherever they live on disk. For each one, PV links the matching directory as its own Project. You don't run a PV command for this.

## What a worktree gets

- **Its own Project:** a slug and hostname of `<source-slug>-<worktree-directory>`, such as `acme-happy-otter.test`. The name comes from the worktree's directory name, not the branch. Some agent apps name every worktree directory after the repository, giving `acme-acme.test`, `acme-acme-1.test` and so on. Always look the hostname up.
- **Its own data:** every allocation in its `pv.yml` gets a new database, Redis key prefix or bucket. Nothing falls back to the main checkout's database.
- **A copied `.env`:** if the worktree has no env file, PV copies the main checkout's and clears its PV block, then renders the worktree's own values. It never overwrites an existing env file.
- **Its own config:** the worktree reads only its own `pv.yml`, the one committed on its branch.
- **Monorepos:** each linked member gets its own Project. If `apps/web` and `apps/api` are linked in the main checkout, a worktree at `../feat` gets Projects at `../feat/apps/web` and `../feat/apps/api`.

## Wait until it is ready

A new worktree passes through these stages:

1. **Discovery.** Usually within a second of Git creating the worktree. If its `pv.yml` declares approved `prepare` hooks, PV runs them first, which can take minutes for `composer install`.
2. **Linked.** `pv list --json` lists a Project whose `path` is inside the worktree.
3. **Data ready and served.** PV adds the route only after the worktree's databases and env are in place. So the first HTTP response from `https://<hostname>`, whatever its status code, shows both; until then the TLS handshake fails. A resource-only Project has no route, so `env: "rendered"` is the only sign, and only when `pv.yml` maps env values. `env: "none"` says nothing about readiness. If you can't make the request, as inside a sandbox, say that readiness is unconfirmed.
4. **Setup.** If `pv.yml` declares `setup` hooks, PV runs them once after the first successful reconciliation, for example migrations or `npm run build`. The page can load before setup finishes.

To follow setup, look at `pv jobs --json` for entries with `kind: "hooks"`:

- `scope` is `project:<id>` (the `id` from `pv list --json`), or `worktree:<source-id>:<worktree-id>` before the worktree is linked.
- `status` shows whether the run is still `running`, `succeeded` or `failed`. A failed run's `error` names the command, its exit status and its log, `~/.pv/logs/hooks/project-<id>.log`.
- A run skipped because the hooks aren't approved completes with a summary that names `pv hooks:trust`. Tell the developer that setup didn't run, and why.

Until a worktree has its own Project, don't run `pv php:use`, `pv unlink`, `pv open` or `pv project:env` inside it. Some PV versions resolve those to the main checkout's Project when the worktree sits inside it, as `.claude/worktrees/<name>` does.

Before handing over a worktree URL, say what you verified: the Project exists, its URL answers, and setup succeeded, was skipped, or isn't declared. Say which of these you couldn't check.

## Hooks

`pv.yml` may declare shell commands under `hooks.prepare` and `hooks.setup`. PV runs them only after the developer approves each command's exact text with `pv hooks:trust` in a terminal. Worktrees share the main checkout's approvals for the same folder. A changed or new command needs approval again. Without approval, PV links and provisions the worktree and skips both lists.

## Cleanup

- **`git worktree remove`** (or `git worktree prune` after deleting the folder) makes PV unlink the worktree's Projects and **delete their databases, Redis keys and buckets**. That includes data on older service versions.
- **`pv unlink`** inside the worktree keeps the data and stops PV from linking that worktree again until Git removes it.
- **A deleted folder** that Git still registers stays linked and shows `status: "missing"`.

Ask the developer before removing a worktree whose data they may want.
