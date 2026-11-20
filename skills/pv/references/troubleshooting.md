# Troubleshooting PV

Start with the two health reports. Both are read-only:

```sh
pv status --json   # daemon, integrations, services, Projects, current failures with repair hints
pv doctor --json   # deeper checks: files, DNS, ports, CA trust, daemon socket, jobs
```

Then narrow it down:

```sh
pv jobs --json                                   # recent daemon work, including hook runs and their errors
pv logs                                          # the daemon's log
pv logs --worker 8.4                             # the PHP 8.4 worker that serves Projects
pv logs --resource postgres --track 18           # one service track
pv logs --gateway                                # HTTPS routing
```

Suggest repair commands to the developer rather than running broad ones yourself. `pv status` and `pv doctor` name the narrowest command for each failure. Don't run `pv setup` or `pv uninstall`.

## Symptoms

| You see | Likely cause | What to do |
| --- | --- | --- |
| `php -v` shows a version other than the Project's `php` in `pv list --json` | The shell doesn't have PV's `PATH`. On macOS it is set in `~/.zprofile`, which only login shells read. | Use `~/.pv/bin/php` and `~/.pv/bin/composer` by full path. |
| `PHP track 8.4 is not installed.` | `pv.yml` asks for a version PV hasn't installed yet. The shim never downloads. | Wait for PV to apply `pv.yml` (watch `pv jobs --json`), or ask the developer to run the `pv php:install` command shown. |
| `filesystem error at "…/.pv": Operation not permitted` or `unable to open database file`, inside an agent sandbox | The sandbox blocks writes to PV's state | Run the command outside the sandbox, or allow `~/.pv` in the sandbox settings. Recent PV versions read state without writing to it. |
| `pv status` shows the daemon `down` | The background daemon stopped | Suggest `pv daemon:restart`. If the socket shows `denied`, a sandbox blocked the check and the daemon may be fine. |
| The site's TLS handshake fails | No Project is linked at that hostname | Look the hostname up with `pv list --json`. |
| Connection refused on the database port | The service track isn't running yet, or failed | `pv postgres:list --json` (or `mysql`, `redis`, …) shows its `status`. `pv status --json` lists the failure. |
| `.env` doesn't have the values from `pv.yml` | `pv.yml` is invalid, or PV hasn't applied it yet | `pv list --json` shows `config-invalid` or the `env` status. `pv status --json` names the error. |
| A worktree's migrations or assets are missing | Its setup hooks failed, were skipped as unapproved, or are still running | Find its `hooks` entry in `pv jobs --json`; see [worktrees](worktrees.md). |
| A `pv` command says the directory is not inside a linked Project | You're outside every linked Project, or in a worktree PV hasn't linked yet | Check `pv list --json`, or pass the Project's hostname or slug where the command accepts one. |
