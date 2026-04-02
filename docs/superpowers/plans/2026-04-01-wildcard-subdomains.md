# Wildcard Subdomain Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every linked project automatically responds to `*.name.tld` in addition to `name.tld`, matching Valet/Herd behavior.

**Architecture:** Add wildcard to all 8 Caddy site address templates and add a wildcard SAN to the Valet cert generator. DNS and registry are already wildcard-capable.

**Tech Stack:** Go, Caddy templates, x509 certificates

**Spec:** `docs/superpowers/specs/2026-04-01-wildcard-subdomains-design.md`

---

### Task 1: Update Caddy site template tests to expect wildcards

**Files:**
- Modify: `internal/caddy/caddy_test.go`

- [ ] **Step 1: Update `TestGenerateSiteConfig_LaravelOctane` assertion**

Change line 58 from:
```go
	if !strings.Contains(content, "octane-app.test {") {
		t.Error("expected domain octane-app.test")
	}
```
to:
```go
	if !strings.Contains(content, "octane-app.test, *.octane-app.test {") {
		t.Error("expected domain octane-app.test with wildcard")
	}
```

- [ ] **Step 2: Update `TestGenerateSiteConfig_Laravel` assertion**

Change line 121 from:
```go
	if !strings.Contains(content, "lara-app.test {") {
		t.Error("expected domain lara-app.test")
	}
```
to:
```go
	if !strings.Contains(content, "lara-app.test, *.lara-app.test {") {
		t.Error("expected domain lara-app.test with wildcard")
	}
```

- [ ] **Step 3: Update `TestGenerateSiteConfig_DomainName` assertion**

Change line 218 from:
```go
	if !strings.Contains(content, "my-app.test {") {
		t.Errorf("expected 'my-app.test {' in output, got:\n%s", content)
	}
```
to:
```go
	if !strings.Contains(content, "my-app.test, *.my-app.test {") {
		t.Errorf("expected 'my-app.test, *.my-app.test {' in output, got:\n%s", content)
	}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `go test ./internal/caddy/ -run "TestGenerateSiteConfig_LaravelOctane|TestGenerateSiteConfig_Laravel|TestGenerateSiteConfig_DomainName" -v`
Expected: 3 FAIL — templates still produce `name.test {` without wildcard.

### Task 2: Add wildcard to main Caddy site templates

**Files:**
- Modify: `internal/caddy/caddy.go`

- [ ] **Step 1: Update `laravelOctaneTmpl`**

Change line 26 from:
```go
const laravelOctaneTmpl = `{{.Name}}.{{.TLD}} {
```
to:
```go
const laravelOctaneTmpl = `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 2: Update `laravelTmpl`**

Change line 43 from:
```go
const laravelTmpl = `{{.Name}}.{{.TLD}} {
```
to:
```go
const laravelTmpl = `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 3: Update `phpTmpl`**

Change line 54 from:
```go
const phpTmpl = `{{.Name}}.{{.TLD}} {
```
to:
```go
const phpTmpl = `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 4: Update `staticTmpl`**

Change line 65 from:
```go
const staticTmpl = `{{.Name}}.{{.TLD}} {
```
to:
```go
const staticTmpl = `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 5: Update `proxyTmpl`**

Change line 74 from:
```go
const proxyTmpl = `{{.Name}}.{{.TLD}} {
```
to:
```go
const proxyTmpl = `{{.Name}}.{{.TLD}}, *.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `go test ./internal/caddy/ -run "TestGenerateSiteConfig_LaravelOctane|TestGenerateSiteConfig_Laravel|TestGenerateSiteConfig_DomainName" -v`
Expected: 3 PASS

- [ ] **Step 7: Run full caddy test suite**

Run: `go test ./internal/caddy/ -v`
Expected: All PASS. The service console tests (`TestGenerateServiceSiteConfigs`) should still pass since those templates are not changed (they use `serviceConsoleTmpl` which is `subdomain.pv.tld`, not a project wildcard).

- [ ] **Step 8: Commit**

```bash
git add internal/caddy/caddy.go internal/caddy/caddy_test.go
git commit -m "Add wildcard subdomain to main Caddy site templates

Every linked project now responds to *.name.tld in addition to
name.tld. Covers laravel, laravel-octane, php, static, and proxy
templates."
```

### Task 3: Add wildcard to secondary (version) Caddy templates and test

**Files:**
- Modify: `internal/caddy/caddy.go`
- Modify: `internal/caddy/caddy_test.go`

- [ ] **Step 1: Add test for wildcard in version-specific config**

Add this test after `TestOctaneTemplate_WatchesAutoload_VersionSpecific`:

```go
func TestVersionSiteConfig_HasWildcard(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "ver-app", Path: projDir, Type: "laravel", PHP: "8.3"}

	if err := GenerateSiteConfig(p, "8.4"); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readVersionSiteConfig(t, "8.3", "ver-app")

	if !strings.Contains(content, "ver-app.test, *.ver-app.test {") {
		t.Errorf("expected wildcard in version site config, got:\n%s", content)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test ./internal/caddy/ -run TestVersionSiteConfig_HasWildcard -v`
Expected: FAIL — version templates still produce `http://name.tld {` without wildcard.

- [ ] **Step 3: Update `versionLaravelOctaneTmpl`**

Change line 82 from:
```go
const versionLaravelOctaneTmpl = `http://{{.Name}}.{{.TLD}} {
```
to:
```go
const versionLaravelOctaneTmpl = `http://{{.Name}}.{{.TLD}}, http://*.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 4: Update `versionLaravelTmpl`**

Change line 98 from:
```go
const versionLaravelTmpl = `http://{{.Name}}.{{.TLD}} {
```
to:
```go
const versionLaravelTmpl = `http://{{.Name}}.{{.TLD}}, http://*.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 5: Update `versionPhpTmpl`**

Change line 108 from:
```go
const versionPhpTmpl = `http://{{.Name}}.{{.TLD}} {
```
to:
```go
const versionPhpTmpl = `http://{{.Name}}.{{.TLD}}, http://*.{{.Name}}.{{.TLD}} {
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `go test ./internal/caddy/ -v`
Expected: All PASS (including the new `TestVersionSiteConfig_HasWildcard`).

- [ ] **Step 7: Commit**

```bash
git add internal/caddy/caddy.go internal/caddy/caddy_test.go
git commit -m "Add wildcard subdomain to secondary version Caddy templates

Version-specific FrankenPHP instances now also match *.name.tld
requests proxied from the main process."
```

### Task 4: Update cert generation to include wildcard SAN and test

**Files:**
- Modify: `internal/certs/certs.go`
- Modify: `internal/certs/certs_test.go`

- [ ] **Step 1: Update `TestGenerateSiteCert` to expect wildcard SAN**

In `internal/certs/certs_test.go`, change line 87 from:
```go
	if len(cert.DNSNames) != 1 || cert.DNSNames[0] != "myapp.test" {
		t.Errorf("DNSNames = %v, want [myapp.test]", cert.DNSNames)
	}
```
to:
```go
	if len(cert.DNSNames) != 2 || cert.DNSNames[0] != "myapp.test" || cert.DNSNames[1] != "*.myapp.test" {
		t.Errorf("DNSNames = %v, want [myapp.test *.myapp.test]", cert.DNSNames)
	}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test ./internal/certs/ -run TestGenerateSiteCert -v`
Expected: FAIL — `DNSNames = [myapp.test], want [myapp.test *.myapp.test]`

- [ ] **Step 3: Add wildcard SAN to `GenerateSiteCert`**

In `internal/certs/certs.go`, change line 37 from:
```go
		DNSNames:     []string{hostname},
```
to:
```go
		DNSNames:     []string{hostname, "*." + hostname},
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test ./internal/certs/ -v`
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/certs/certs.go internal/certs/certs_test.go
git commit -m "Add wildcard SAN to Valet TLS certificates

Certs now include *.hostname in DNSNames so the Vite dev server
cert covers subdomain requests."
```

### Task 5: Final verification

- [ ] **Step 1: Run full test suite**

Run: `go test ./...`
Expected: All PASS.

- [ ] **Step 2: Run lint checks**

Run: `gofmt -l . && go vet ./...`
Expected: No output (all clean).

- [ ] **Step 3: Verify build**

Run: `go build -o pv .`
Expected: Builds successfully.
