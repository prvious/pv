# Test Directory Structure

This directory contains tests for the PV CLI project.

## Test Organization

### Integration Tests (`test/integration/`)

Integration tests are full end-to-end tests that:
- Build and execute the actual binary
- Verify command execution and outputs
- Check filesystem changes (created files, stubs, etc.)
- Test the full action pipeline

**Running integration tests:**
```bash
go test -tags=integration ./test/integration/...
```

### Unit Tests (`internal/*/`)

Unit tests should be placed alongside the code they test (e.g., `internal/app/actions_test.go`). These tests:
- Use mocked dependencies
- Test individual functions and components
- Run quickly without external dependencies

**Running unit tests:**
```bash
go test ./...
```

## Build Tags

Integration tests use the `//go:build integration` build tag to separate them from fast unit tests. This allows:
- Fast feedback from unit tests during development
- Full validation via integration tests in CI/CD
- Flexible test execution based on context

## CI/CD

The GitHub Actions workflow (`.github/workflows/test.yml`) runs both unit and integration tests on:
- Pull requests
- Pushes to main branch
- Multiple operating systems (Ubuntu, macOS, Windows)
- Go version 1.22.x

## Writing New Tests

### Integration Test Example

```go
//go:build integration

package integration

import (
	"os/exec"
	"testing"
)

func TestDockerInstall(t *testing.T) {
	// Build the binary
	cmd := exec.Command("go", "build", "-o", "pv", ".")
	if err := cmd.Run(); err != nil {
		t.Fatal(err)
	}

	// Execute action
	cmd = exec.Command("./pv", "docker", "install")
	output, err := cmd.CombinedOutput()
	
	// Assert results
	if err != nil {
		t.Fatalf("Command failed: %v\nOutput: %s", err, output)
	}
	
	// Check filesystem or other side effects
	// ...
}
```

### Unit Test Example

```go
package app

import "testing"

func TestActionRegistry(t *testing.T) {
	RegisterAction("test:action", func() error {
		return nil
	})
	
	actions := GetActions()
	if _, exists := actions["test:action"]; !exists {
		t.Error("Action not registered")
	}
}
```
