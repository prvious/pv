package rustfs

import (
	"reflect"
	"testing"

	"github.com/spf13/cobra"
)

// canonicalCommands is the full set of rustfs:* commands the package
// exposes. Each must have a paired hidden s3:* alias.
var canonicalCommands = []string{
	"rustfs:install",
	"rustfs:uninstall",
	"rustfs:update",
	"rustfs:start",
	"rustfs:stop",
	"rustfs:restart",
	"rustfs:status",
	"rustfs:logs",
}

func newRoot(t *testing.T) *cobra.Command {
	t.Helper()
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "rustfs", Title: "RustFS"})
	Register(root)
	return root
}

func TestRegister_AllCanonicalCommandsPresent(t *testing.T) {
	root := newRoot(t)
	for _, name := range canonicalCommands {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("canonical command %q not registered", name)
		}
	}
}

// TestRegister_AliasesPresentAndHidden verifies that every rustfs:* command
// has a paired s3:* alias clone that is registered and hidden in --help —
// the contract documented on aliasCommand. Regression here would either
// drop the historical s3:* spelling or pollute help output with duplicates.
func TestRegister_AliasesPresentAndHidden(t *testing.T) {
	root := newRoot(t)
	for _, canonical := range canonicalCommands {
		alias := "s3:" + canonical[len("rustfs:"):]
		aliasCmd, _, err := root.Find([]string{alias})
		if err != nil || aliasCmd.Name() != alias {
			t.Errorf("alias %q not registered", alias)
			continue
		}
		if !aliasCmd.Hidden {
			t.Errorf("alias %q must be Hidden=true to keep --help clean", alias)
		}
	}
}

// TestRegister_AliasShareImplementation locks the single-source-of-truth
// contract: the s3:* alias must point at the same RunE as the canonical
// rustfs:* command. A bug that copies and divergently mutates the clone
// would break this and silently drift the two spellings apart.
func TestRegister_AliasShareImplementation(t *testing.T) {
	root := newRoot(t)
	for _, canonical := range canonicalCommands {
		alias := "s3:" + canonical[len("rustfs:"):]

		canonCmd, _, err := root.Find([]string{canonical})
		if err != nil {
			t.Fatalf("canonical %q lookup failed: %v", canonical, err)
		}
		aliasCmd, _, err := root.Find([]string{alias})
		if err != nil {
			t.Fatalf("alias %q lookup failed: %v", alias, err)
		}
		if canonCmd.RunE == nil {
			t.Errorf("canonical %q has nil RunE", canonical)
			continue
		}
		// Compare function pointers via reflect — direct == on funcs is
		// disallowed by the language.
		canonPtr := reflect.ValueOf(canonCmd.RunE).Pointer()
		aliasPtr := reflect.ValueOf(aliasCmd.RunE).Pointer()
		if canonPtr != aliasPtr {
			t.Errorf("alias %q RunE differs from canonical %q (clone broken)", alias, canonical)
		}
	}
}
