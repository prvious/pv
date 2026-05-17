package tier

import (
	"strings"
	"testing"
)

func TestDefaultTierIsLocalSafeTier0(t *testing.T) {
	definition := Default()

	if definition.Level != Level0 {
		t.Fatalf("default tier = %s, want %s", definition.Level, Level0)
	}
	if !definition.LocalSafe {
		t.Fatal("Tier 0 must be local-safe")
	}
	if got, want := strings.Join(definition.Command, " "), "go test ./test/e2e/scenarios"; got != want {
		t.Fatalf("Tier 0 command = %q, want %q", got, want)
	}
}

func TestTier1AndTier2FailClosedOutsideCI(t *testing.T) {
	for _, level := range []Level{Level1, Level2} {
		definition, err := DefinitionFor(level)
		if err != nil {
			t.Fatalf("definition for %s: %v", level, err)
		}
		err = definition.Validate(Environment{})
		if err == nil {
			t.Fatalf("%s accepted local environment", level)
		}
		if !strings.Contains(err.Error(), "CI=true") {
			t.Fatalf("%s guard error = %v, want CI=true guidance", level, err)
		}
	}
}

func TestTier1AndTier2RequireGitHubHostedRunner(t *testing.T) {
	for _, level := range []Level{Level1, Level2} {
		definition, err := DefinitionFor(level)
		if err != nil {
			t.Fatalf("definition for %s: %v", level, err)
		}
		err = definition.Validate(Environment{
			"CI":             "true",
			"GITHUB_ACTIONS": "true",
		})
		if err == nil {
			t.Fatalf("%s accepted non-github-hosted environment", level)
		}
		if !strings.Contains(err.Error(), "RUNNER_ENVIRONMENT=github-hosted") {
			t.Fatalf("%s guard error = %v, want github-hosted guidance", level, err)
		}
		if err := definition.Validate(githubHostedEnv()); err != nil {
			t.Fatalf("%s rejected github-hosted CI env: %v", level, err)
		}
	}
}

func TestTier2HostActionPlanIsPrintable(t *testing.T) {
	definition, err := DefinitionFor(Level2)
	if err != nil {
		t.Fatalf("definition for tier 2: %v", err)
	}

	plan := definition.HostActionPlan()
	for _, want := range []string{
		"Tier 2 intended host actions:",
		"DNS",
		"TLS",
		"browser",
	} {
		if !strings.Contains(plan, want) {
			t.Fatalf("Tier 2 host action plan missing %q:\n%s", want, plan)
		}
	}
}

func githubHostedEnv() Environment {
	return Environment{
		"CI":                 "true",
		"GITHUB_ACTIONS":     "true",
		"RUNNER_ENVIRONMENT": "github-hosted",
	}
}
