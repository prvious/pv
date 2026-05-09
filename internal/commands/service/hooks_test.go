package service

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

// TestApplyStopAllFallbacks verifies the production function that stop.go
// calls in the no-args path. Binary services are skipped (they're owned
// by rustfs:* / mailpit:* now and were never stopped by service:stop).
//
// With redis migrated to a native binary, the docker registry is empty,
// so this test asserts the binary-skip path only — the docker path has
// no surface area to exercise.
func TestApplyStopAllFallbacks(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	mailProjectDir := t.TempDir()
	os.WriteFile(filepath.Join(mailProjectDir, ".env"),
		[]byte("MAIL_MAILER=smtp\n"), 0644)

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Kind: "binary", Port: 1025},
		},
		Projects: []registry.Project{
			{Name: "mail-app", Path: mailProjectDir, Type: "laravel",
				Services: &registry.ProjectServices{Mail: true}},
		},
	}

	origConfirm := automation.ConfirmFunc
	automation.ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { automation.ConfirmFunc = origConfirm }()

	applyStopAllFallbacks(reg)

	// Binary service (mail) should NOT have fallback applied.
	mailEnv, _ := services.ReadDotEnv(filepath.Join(mailProjectDir, ".env"))
	if mailEnv["MAIL_MAILER"] != "smtp" {
		t.Errorf("mail MAIL_MAILER = %q, want smtp (should NOT have been changed)",
			mailEnv["MAIL_MAILER"])
	}
}

// Note: TestUpdateLinkedProjectsEnv_OnlyUpdatesLinkedProject was dropped
// when redis (the last docker Service) migrated to a native binary.
// updateLinkedProjectsEnv takes a services.Service argument; with no
// docker Service implementations left there's nothing to pass in. The
// production function remains for callers in add.go / start.go that
// would still apply if a docker service were ever reintroduced.

func TestUnbindService_ClearsMailBinding(t *testing.T) {
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-app", Path: "/tmp/test",
				Services: &registry.ProjectServices{Mail: true, Redis: true}},
		},
	}

	reg.UnbindService("mail")

	project := reg.Find("test-app")
	if project.Services.Mail {
		t.Error("ProjectServices.Mail should be false after UnbindService")
	}
	// Redis should be untouched.
	if !project.Services.Redis {
		t.Error("ProjectServices.Redis should still be true")
	}
}
