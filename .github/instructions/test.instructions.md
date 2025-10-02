# PV CLI Test Instructions

This document provides comprehensive instructions for understanding and working with tests in the PV CLI project.

## 📁 Test Organization

Tests in this project are organized into two categories:

### Unit Tests (`internal/*/`)

**Location:** Unit tests are placed alongside the code they test (e.g., `internal/app/actions_test.go`)

**Purpose:**
- Test individual functions and components in isolation
- Use mocked dependencies
- Run quickly without external dependencies
- Provide fast feedback during development

**Example:**
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

**Running unit tests:**
```bash
go test ./...
```

### Integration Tests (`test/integration/`) - Future Enhancement

**Status:** Integration tests are planned for future implementation once concrete actions are available to test.

**Location:** All integration tests will be in the `test/integration/` directory

**Purpose:**
- Full end-to-end tests that build and execute the actual binary
- Verify command execution and outputs
- Check filesystem changes (created files, stubs, etc.)
- Test the full action pipeline
- Validate real-world usage scenarios

**Build Tag:** Integration tests will use the `//go:build integration` build tag to separate them from unit tests

**Example (for future reference):**
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

**Running integration tests (when available):**
```bash
go test -tags=integration ./test/integration/...
```

## 🏗️ Build Tags

Integration tests will use Go's build tag system to enable selective test execution:

- **Build Tag:** `//go:build integration` at the top of integration test files (when created)
- **Purpose:** Separates fast unit tests from slower integration tests
- **Benefits:**
  - Fast feedback from unit tests during development
  - Full validation via integration tests in CI/CD (once available)
  - Flexible test execution based on context

## 🚀 Running Tests

### During Development (Fast)

Run unit tests for quick feedback:
```bash
go test ./...
```

### Before Committing

Run all tests and checks:
```bash
# Unit tests
go test ./...

# Code formatting
go fmt ./...

# Static analysis
go vet ./...

# Build
go build -o bin/pv .
```

### Complete Test Suite

Run everything like CI does:
```bash
go test ./...
go fmt ./...
go vet ./...
go build -o bin/pv .
```

**Note:** Integration tests will be added in the future once concrete actions are available to test.

## 🔄 CI/CD Integration

The GitHub Actions workflow (`.github/workflows/test.yml`) automatically runs tests on:

- **Triggers:**
  - Pull requests to main branch
  - Pushes to main branch
  - Manual workflow dispatch

- **Platforms:**
  - Ubuntu (latest)
  - macOS (latest)
  - Windows (latest)

- **Go Version:** 1.22.x

- **Test Jobs:**
  1. **Test Job:** Builds binary and runs unit tests (on all 3 platforms)
  2. **Lint Job:** Validates code formatting (`gofmt`) and runs static analysis (`go vet`)

## ✍️ Writing New Tests

### Creating a Unit Test

1. Create a file named `<package>_test.go` in the same directory as the code being tested
2. Use the same package name as the code being tested
3. Write test functions starting with `Test`
4. Mock external dependencies
5. Focus on testing a single component or function

**Example file:** `internal/app/model_test.go`

```go
package app

import "testing"

func TestInitialModel(t *testing.T) {
    model := InitialModel()
    
    if len(model.options) == 0 {
        t.Error("Expected options to be populated")
    }
}
```

### Creating an Integration Test (Future)

Integration tests will be added once concrete actions are available. When creating them:

1. Create a file in `test/integration/` directory
2. Add `//go:build integration` as the first line
3. Use `package integration`
4. Build and execute the actual binary
5. Verify outputs, exit codes, and filesystem changes

**Example file (for future reference):** `test/integration/laravel_test.go`

```go
//go:build integration

package integration

import (
    "os"
    "os/exec"
    "testing"
)

func TestLaravelSetup(t *testing.T) {
    // Build the binary
    cmd := exec.Command("go", "build", "-o", "pv", ".")
    if err := cmd.Run(); err != nil {
        t.Fatal(err)
    }
    defer os.Remove("pv")

    // Execute action
    cmd = exec.Command("./pv", "laravel", "setup")
    output, err := cmd.CombinedOutput()
    
    if err != nil {
        t.Fatalf("Command failed: %v\nOutput: %s", err, output)
    }
    
    // Verify files were created
    if _, err := os.Stat("docker-compose.yml"); os.IsNotExist(err) {
        t.Error("docker-compose.yml was not created")
    }
}
```

## 📊 Test Coverage

### Viewing Test Coverage

Generate coverage report:
```bash
go test -coverprofile=coverage.out ./...
go tool cover -html=coverage.out
```

### Coverage Guidelines

- Unit tests should cover core business logic
- Integration tests should cover critical user workflows
- Aim for meaningful coverage, not just high percentages
- Focus on testing behavior, not implementation details

## 🐛 Debugging Tests

### Running a Single Test

```bash
go test -run TestName ./path/to/package
```

### Running Tests with Verbose Output

```bash
go test -v ./...
```

### Running Integration Tests with Verbose Output (when available)

```bash
go test -v -tags=integration ./test/integration/...
```

### Debugging with Delve

```bash
dlv test ./path/to/package -- -test.run TestName
```

## ✅ Best Practices

### For All Tests

- Write clear, descriptive test names that explain what is being tested
- Test one thing per test function
- Use table-driven tests for multiple similar cases
- Clean up resources (files, directories) after tests
- Make tests deterministic (avoid time-dependent tests)

### For Unit Tests

- Mock external dependencies (network, filesystem, time)
- Test edge cases and error conditions
- Keep tests fast (milliseconds, not seconds)
- Test public APIs, not internal implementation

### For Integration Tests (Future)

When integration tests are added, they should:
- Test real-world scenarios
- Verify complete workflows from start to finish
- Check actual file creation and content
- Validate exit codes and output messages
- Clean up test artifacts (binaries, generated files)

## 🚫 Common Pitfalls

- **Don't:** Commit test binaries or generated files (use `.gitignore`)
- **Don't:** Use hardcoded paths or system-specific assumptions
- **Don't:** Leave test resources on the filesystem after tests complete
- **Don't:** Test implementation details instead of behavior

## 🔧 Troubleshooting

### Tests Pass Locally but Fail in CI

- Check for platform-specific code or paths
- Verify all dependencies are available in CI environment
- Look for timing issues or race conditions
- Review CI logs for additional error details

### Integration Tests Are Slow (Future)

When integration tests are added, if they become slow:
- Ensure build tags are properly set
- Consider parallelizing tests with `t.Parallel()`
- Cache built binaries when possible
- Profile tests to identify bottlenecks

### Import Cycle Errors

- Move shared test utilities to a separate package
- Use internal test packages (`package app_test`) when needed
- Restructure code to avoid circular dependencies

## 📚 Additional Resources

- [Go Testing Documentation](https://golang.org/pkg/testing/)
- [Go Build Constraints](https://golang.org/cmd/go/#hdr-Build_constraints)
- [Table-Driven Tests in Go](https://dave.cheney.net/2019/05/07/prefer-table-driven-tests)
- [GitHub Actions Documentation](https://docs.github.com/en/actions)
