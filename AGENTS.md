# Agent Guidelines for PV CLI

## Build/Test Commands

-   **Build**: `go build -o bin/pv .`
-   **Run**: `go run .`
-   **Test all**: `go test ./...`
-   **Test single package**: `go test ./path/to/package`
-   **Test specific function**: `go test -run TestFunctionName`
-   **Format**: `go fmt ./...`
-   **Lint**: `go vet ./...`
-   **Install deps**: `go mod tidy`

## Code Style Guidelines

-   Use `gofmt` for formatting (no custom rules)
-   Group imports: stdlib, third-party, local packages
-   Use structured logging with `github.com/charmbracelet/log`
-   Error handling: log with context using `log.Fatal("message", "key", value)`
-   Variable naming: camelCase for locals, PascalCase for exports
-   Package structure: single main package for CLI tool
-   UI components: use Charm libraries (lipgloss, huh, bubbletea)
-   Dependencies: prefer minimal, well-maintained packages
-   Comments: only for exported functions and complex logic
-   File naming: lowercase with underscores for test files (`*_test.go`)
