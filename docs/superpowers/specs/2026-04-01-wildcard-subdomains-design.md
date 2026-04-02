# Wildcard Subdomain Support

**Date:** 2026-04-01
**Status:** Approved

## Problem

`pv link` creates a single top-level domain (e.g., `project.test`). Applications that use subdomain-based routing (multi-tenant apps, locale prefixes like `us.project.test` / `ca.project.test`) cannot be developed locally without manual workarounds.

Laravel Valet and Herd both support wildcard subdomains automatically — every linked site responds to `*.site.test` with no extra configuration. pv should match this behavior.

## Design

### Approach

Automatic wildcard subdomain support for all linked sites. No flags, no per-project config. Every linked project responds to both `name.tld` and `*.name.tld`.

### Scope

Single-level wildcard only. `*.project.test` covers `us.project.test` and `ca.project.test` but not `api.us.project.test`. This is an X.509/DNS spec limitation shared by Valet and Herd.

### Changes

#### 1. Caddy site templates (`internal/caddy/caddy.go`)

Every site address changes from `{{.Name}}.{{.TLD}}` to `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}}`.

Affected templates (8 total):
- **Main process**: `laravelOctaneTmpl`, `laravelTmpl`, `phpTmpl`, `staticTmpl`
- **Proxy**: `proxyTmpl`
- **Secondary version**: `versionLaravelOctaneTmpl`, `versionLaravelTmpl`, `versionPhpTmpl`

Caddy's `tls internal` directive automatically generates valid certs covering all listed site addresses, so wildcard TLS for browser-facing HTTPS is handled by Caddy with no additional cert work.

No changes to `siteData` struct, `writeConfig`, template helpers, or generation logic.

#### 2. Valet TLS cert SAN (`internal/certs/certs.go`)

In `GenerateSiteCert`, change:
```go
DNSNames: []string{hostname}
```
to:
```go
DNSNames: []string{hostname, "*." + hostname}
```

This ensures the Valet/Vite-compatible cert (used by `laravel-vite-plugin` for the dev server) also covers wildcard subdomains. The cert filename (`{hostname}.crt`/`.key`) and the `GenerateSiteTLS` function signature remain unchanged.

#### 3. No changes required

- **DNS server** (`internal/server/dns.go`): Already resolves all `*.test` queries to `127.0.0.1` at any subdomain depth.
- **macOS resolver** (`internal/setup/resolver.go`): `/etc/resolver/test` already applies to all subdomains.
- **Registry** (`internal/registry/`): No new fields needed.
- **Link command** (`cmd/link.go`): No new flags.
- **Valet config** (`internal/certs/valet.go`): Cert filenames stay as `{name}.{tld}.crt` — Vite plugin looks up by `basename(cwd) + "." + tld`, not by subdomain.
- **TLS cert automation step** (`internal/automation/steps/generate_tls_cert.go`): Passes `hostname` as before; the SAN change is internal to `GenerateSiteCert`.
- **Unlink/cleanup**: Same files to remove, no new artifacts.

### Testing

- Update existing Caddy template tests to verify wildcard addresses appear in generated configs.
- Update cert generation tests to verify `*.hostname` appears in the cert's SAN list.

### Limitations

- **Single-level wildcards only**: `*.project.test` matches `sub.project.test` but not `deep.sub.project.test`. This is an X.509 and DNS standard limitation.
- **Firefox cert trust**: Firefox uses its own cert store, not the macOS Keychain. This is a pre-existing issue unrelated to this change.
