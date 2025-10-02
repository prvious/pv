package dns

import (
	"testing"

	"github.com/prvious/pv/internal/app"
)

func TestActionRegistration(t *testing.T) {
	actions := app.GetActions()
	if _, exists := actions["dns:install"]; !exists {
		t.Error("Action 'dns:install' was not registered")
	}
}
