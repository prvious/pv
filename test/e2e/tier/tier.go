package tier

import (
	"fmt"
	"os"
	"strings"
)

// Level identifies an E2E safety tier.
type Level string

const (
	Level0 Level = "tier0"
	Level1 Level = "tier1"
	Level2 Level = "tier2"
)

// Definition describes how a tier is invoked and guarded.
type Definition struct {
	Level                Level
	Name                 string
	Command              []string
	LocalSafe            bool
	RequiresGitHubHosted bool
	HostActions          []string
}

// Environment is the small CI environment surface used by tier guards.
type Environment map[string]string

// Default returns the local-safe Tier 0 definition.
func Default() Definition {
	return definitions[Level0]
}

// DefinitionFor returns the definition for a named E2E tier.
func DefinitionFor(level Level) (Definition, error) {
	definition, ok := definitions[level]
	if !ok {
		return Definition{}, fmt.Errorf("unknown E2E tier %q", level)
	}
	return definition, nil
}

// FromOS reads the tier guard variables from the current process environment.
func FromOS() Environment {
	return Environment{
		"CI":                 os.Getenv("CI"),
		"GITHUB_ACTIONS":     os.Getenv("GITHUB_ACTIONS"),
		"RUNNER_ENVIRONMENT": os.Getenv("RUNNER_ENVIRONMENT"),
	}
}

// Validate fails closed when a CI-only tier is not running in GitHub-hosted CI.
func (d Definition) Validate(env Environment) error {
	if !d.RequiresGitHubHosted {
		return nil
	}
	if env["CI"] != "true" {
		return fmt.Errorf("%s is CI-only: CI=true is required", d.Level)
	}
	if env["GITHUB_ACTIONS"] != "true" {
		return fmt.Errorf("%s is CI-only: GITHUB_ACTIONS=true is required", d.Level)
	}
	if env["RUNNER_ENVIRONMENT"] != "github-hosted" {
		return fmt.Errorf("%s is CI-only: RUNNER_ENVIRONMENT=github-hosted is required", d.Level)
	}
	return nil
}

// HostActionPlan renders the host mutations Tier 2 intends to perform.
func (d Definition) HostActionPlan() string {
	if len(d.HostActions) == 0 {
		return ""
	}
	var b strings.Builder
	fmt.Fprintf(&b, "Tier 2 intended host actions:\n")
	for _, action := range d.HostActions {
		fmt.Fprintf(&b, "- %s\n", action)
	}
	return b.String()
}

var definitions = map[Level]Definition{
	Level0: {
		Level:     Level0,
		Name:      "Tier 0 hermetic E2E",
		Command:   []string{"go", "test", "./test/e2e/scenarios"},
		LocalSafe: true,
	},
	Level1: {
		Level:                Level1,
		Name:                 "Tier 1 CI local-process E2E",
		Command:              []string{"go", "test", "-tags=e2e_tier1", "./test/e2e/tier1"},
		RequiresGitHubHosted: true,
	},
	Level2: {
		Level:                Level2,
		Name:                 "Tier 2 CI privileged-host E2E",
		Command:              []string{"go", "test", "-tags=e2e_tier2", "./test/e2e/tier2"},
		RequiresGitHubHosted: true,
		HostActions: []string{
			"DNS routing for .test hostnames in a disposable GitHub-hosted runner",
			"TLS certificate generation and trust-store changes in a disposable GitHub-hosted runner",
			"browser launch against the generated HTTPS project URL in a disposable GitHub-hosted runner",
		},
	},
}
