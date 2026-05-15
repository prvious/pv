package mailpit

import (
	"io"
	"os"
	"reflect"
	"strings"
	"testing"

	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

// canonicalCommands is the full set of mailpit:* commands the package
// exposes. Each must have a paired hidden mail:* alias.
var canonicalCommands = []string{
	"mailpit:install",
	"mailpit:uninstall",
	"mailpit:update",
	"mailpit:start",
	"mailpit:stop",
	"mailpit:restart",
	"mailpit:status",
	"mailpit:logs",
}

func newRoot(t *testing.T) *cobra.Command {
	t.Helper()
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "mailpit", Title: "Mailpit"})
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

// TestRegister_AliasesPresentAndHidden verifies that every mailpit:*
// command has a paired mail:* alias clone that is registered and hidden
// in --help — the contract documented on aliasCommand.
func TestRegister_AliasesPresentAndHidden(t *testing.T) {
	root := newRoot(t)
	for _, canonical := range canonicalCommands {
		alias := "mail:" + canonical[len("mailpit:"):]
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
// contract: the mail:* alias must point at the same RunE as the
// canonical mailpit:* command.
func TestRegister_AliasShareImplementation(t *testing.T) {
	root := newRoot(t)
	for _, canonical := range canonicalCommands {
		alias := "mail:" + canonical[len("mailpit:"):]

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
		canonPtr := reflect.ValueOf(canonCmd.RunE).Pointer()
		aliasPtr := reflect.ValueOf(aliasCmd.RunE).Pointer()
		if canonPtr != aliasPtr {
			t.Errorf("alias %q RunE differs from canonical %q (clone broken)", alias, canonical)
		}
	}
}

func TestStopSignalsThroughNoDaemonHelper(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	output := captureStderr(t, func() {
		if err := stopCmd.RunE(stopCmd, []string{pkg.DefaultVersion()}); err != nil {
			t.Fatalf("stop RunE error = %v", err)
		}
	})

	if strings.Contains(output, "will start") {
		t.Fatalf("stderr = %q, must not promise service will start", output)
	}
	if !strings.Contains(output, "daemon not running; changes will apply on next `pv start`") {
		t.Fatalf("stderr = %q, want neutral no-daemon signal message", output)
	}
}

func captureStderr(t *testing.T, fn func()) string {
	t.Helper()

	original := os.Stderr
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatalf("pipe stderr: %v", err)
	}
	os.Stderr = w
	t.Cleanup(func() {
		os.Stderr = original
	})

	fn()

	if err := w.Close(); err != nil {
		t.Fatalf("close stderr writer: %v", err)
	}
	out, err := io.ReadAll(r)
	if err != nil {
		t.Fatalf("read stderr: %v", err)
	}
	if err := r.Close(); err != nil {
		t.Fatalf("close stderr reader: %v", err)
	}
	os.Stderr = original

	return string(out)
}
