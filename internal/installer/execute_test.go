package installer

import (
	"context"
	"errors"
	"testing"
)

func TestExecuteSkipsDependentsAfterFailure(t *testing.T) {
	php := Identity{Kind: KindRuntime, Name: "php", Version: "8.4.1"}
	composer := Identity{Kind: KindTool, Name: "composer", Version: "2.9.2"}
	plan := Plan{Items: []Item{
		{ID: php},
		{ID: composer, DependsOn: []Identity{php}},
	}}
	cause := errors.New("download missing")

	results, err := Execute(context.Background(), plan, fakeInstaller{fail: map[string]error{php.String(): cause}})

	if err != nil {
		t.Fatalf("Execute returned error: %v", err)
	}
	if results[0].State != ResultFailed {
		t.Fatalf("php state = %q, want failed", results[0].State)
	}
	if results[1].State != ResultSkipped {
		t.Fatalf("composer state = %q, want skipped", results[1].State)
	}
	if !errors.Is(results[1].Err, cause) {
		t.Fatalf("composer error = %v, want %v", results[1].Err, cause)
	}
}

type fakeInstaller struct {
	fail map[string]error
}

func (i fakeInstaller) Install(_ context.Context, item Item) error {
	return i.fail[item.ID.String()]
}
