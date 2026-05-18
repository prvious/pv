//go:build e2e_tier2

package tier2

import (
	"fmt"
	"os"
	"testing"

	"github.com/prvious/pv/test/e2e/tier"
)

func TestTier2RequiresGitHubHostedCI(t *testing.T) {
	definition, err := tier.DefinitionFor(tier.Level2)
	if err != nil {
		t.Fatalf("load tier 2 definition: %v", err)
	}
	fmt.Fprint(os.Stderr, definition.HostActionPlan())
	if err := definition.Validate(tier.FromOS()); err != nil {
		t.Fatal(err)
	}
	t.Skip("Tier 2 privileged-host scenarios are wired by CI after the guard passes")
}
