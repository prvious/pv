package installer

import "testing"

func TestPlanOrdersDependenciesDeterministically(t *testing.T) {
	php := Identity{Kind: KindRuntime, Name: "php", Version: "8.4.1"}
	composer := Identity{Kind: KindTool, Name: "composer", Version: "2.9.2"}
	mailpit := Identity{Kind: KindService, Name: "mailpit", Version: "1.0.0"}
	plan := Plan{Items: []Item{
		{ID: composer, DependsOn: []Identity{php}},
		{ID: mailpit},
		{ID: php},
	}}

	ordered, err := plan.Order()
	if err != nil {
		t.Fatalf("Order returned error: %v", err)
	}
	got := []Identity{ordered[0].ID, ordered[1].ID, ordered[2].ID}
	want := []Identity{php, mailpit, composer}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("ordered[%d] = %s, want %s", i, got[i], want[i])
		}
	}
	if err := plan.Validate(); err != nil {
		t.Fatalf("Validate returned error: %v", err)
	}
}

func TestPlanRejectsInvalidDependencies(t *testing.T) {
	php := Identity{Kind: KindRuntime, Name: "php", Version: "8.4.1"}
	composer := Identity{Kind: KindTool, Name: "composer", Version: "2.9.2"}

	tests := map[string]Plan{
		"duplicate identity": {
			Items: []Item{{ID: php}, {ID: php}},
		},
		"missing dependency": {
			Items: []Item{{ID: composer, DependsOn: []Identity{php}}},
		},
		"cycle": {
			Items: []Item{
				{ID: php, DependsOn: []Identity{composer}},
				{ID: composer, DependsOn: []Identity{php}},
			},
		},
	}
	for name, plan := range tests {
		t.Run(name, func(t *testing.T) {
			if err := plan.Validate(); err == nil {
				t.Fatal("Validate returned nil error")
			}
		})
	}
}
