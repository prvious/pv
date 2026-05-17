//go:build e2e_tier1

package tier1

import (
	"testing"

	"github.com/prvious/pv/test/e2e/tier"
)

func TestTier1RequiresGitHubHostedCI(t *testing.T) {
	definition, err := tier.DefinitionFor(tier.Level1)
	if err != nil {
		t.Fatalf("load tier 1 definition: %v", err)
	}
	if err := definition.Validate(tier.FromOS()); err != nil {
		t.Fatal(err)
	}
	t.Skip("Tier 1 real local-process scenarios are wired by CI after the guard passes")
}
