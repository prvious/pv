# Replace Valet Cert Path with Vite Env Vars

**Date:** 2026-04-02
**Status:** Approved

## Problem

pv stores TLS certs in `~/.config/valet/Certificates/` and maintains a `config.json` to mimic Valet, solely for `laravel-vite-plugin` auto-detection. This has issues:

1. **`--name` flag mismatch**: The Vite plugin resolves hostname from `basename(cwd)`, not the link name. When `--name` differs from the directory name, Vite can't find the cert.
2. **Unnecessary Valet coupling**: pv pretends to be Valet by maintaining `~/.config/valet/` — a directory owned by another tool.
3. **Conflict risk**: If real Valet/Herd is installed, pv's writes to `~/.config/valet/` can interfere.

The Vite plugin has a second detection path: `VITE_DEV_SERVER_KEY` and `VITE_DEV_SERVER_CERT` env vars in `.env`. This path uses explicit file paths and is immune to the basename mismatch.

## Design

### Approach

1. Move cert storage from `~/.config/valet/Certificates/` to `~/.pv/data/certs/`
2. Write `VITE_DEV_SERVER_KEY` and `VITE_DEV_SERVER_CERT` to the project's `.env` during link
3. Remove all Valet config directory code

### Changes

#### 1. Cert storage (`internal/certs/valet.go` → rewrite)

Replace Valet-specific functions with pv-native cert storage:

- `CertsDir() string` — returns `~/.pv/data/certs/`
- `CertPath(hostname) string` — returns `~/.pv/data/certs/{hostname}.crt`
- `KeyPath(hostname) string` — returns `~/.pv/data/certs/{hostname}.key`
- `GenerateSiteTLS(hostname)` — writes cert/key to `CertsDir()` using `GenerateSiteCert` (same low-level cert generation)
- `RemoveSiteTLS(hostname)` — removes cert/key from `CertsDir()`
- `RemoveLinkedCerts(hostnames)` — removes cert/key pairs for given hostnames from `CertsDir()`

**Delete:** `EnsureValetConfig`, `ValetConfigDir`, `ValetCertsDir`, `RemoveConfig`. All `config.json` management is gone.

#### 2. New automation step (`internal/laravel/steps.go`)

`SetViteTLSStep` writes `VITE_DEV_SERVER_KEY` and `VITE_DEV_SERVER_CERT` to `.env`:

- `ShouldRun`: `isLaravel(ctx.ProjectType) && HasEnvFile(ctx.ProjectPath)`
- `Run`: builds cert/key paths from `certs.CertPath(hostname)` and `certs.KeyPath(hostname)` where hostname is `ctx.ProjectName + "." + ctx.TLD`, then writes via `services.MergeDotEnv`
- `Gate`: `"set_vite_tls"`
- `Label`: `"Set Vite TLS"`

#### 3. Settings (`internal/config/settings.go`)

Add `SetViteTLS AutoMode` field to the `Automation` struct, defaulting to `AutoOn`.

#### 4. Pipeline order (`cmd/link.go`)

Add `&laravel.SetViteTLSStep{}` after `&laravel.SetAppURLStep{}` in the pipeline.

#### 5. Update callers

- `cmd/setup.go` — remove `certs.EnsureValetConfig(tld)` call
- `cmd/uninstall.go` — remove `certs.RemoveConfig()` call; keep `certs.RemoveLinkedCerts(hostnames)` (it now removes from `~/.pv/data/certs/`)
- `cmd/unlink.go` — `certs.RemoveSiteTLS` already called, internal path changes only
- `cmd/link.go` — relink path already calls `certs.RemoveSiteTLS`, no change needed
- `internal/automation/steps/generate_tls_cert.go` — remove `certs.EnsureValetConfig` call, just call `certs.GenerateSiteTLS(hostname)`

#### 6. No changes required

- `internal/certs/certs.go` — `GenerateSiteCert` (low-level cert generation) is unchanged
- DNS, Caddy templates, registry — unaffected
- `internal/server/` — unaffected

### Hostname resolution

Certs always use the link name (`ctx.ProjectName`), which respects the `--name` flag. The env vars point to `~/.pv/data/certs/{projectName}.{tld}.crt`. This eliminates the basename mismatch entirely.

### Testing

- Update `internal/certs/valet_test.go` for new cert paths
- Add tests for `SetViteTLSStep` (ShouldRun + Run)
- Add automation gate test for `set_vite_tls`
- Existing `GenerateSiteCert` tests in `certs_test.go` are unaffected
