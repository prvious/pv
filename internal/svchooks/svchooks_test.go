package svchooks

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
)

func TestApplyFallbacksToLinkedProjects_Mail(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	os.WriteFile(envPath, []byte("MAIL_MAILER=smtp\n"), 0644)

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Kind: "binary", Port: 1025},
		},
		Projects: []registry.Project{
			{Name: "test-app", Path: projectDir, Type: "laravel",
				Services: &registry.ProjectServices{Mail: true}},
		},
	}

	origConfirm := automation.ConfirmFunc
	automation.ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { automation.ConfirmFunc = origConfirm }()

	ApplyFallbacksToLinkedProjects(reg, "mail")

	env, _ := projectenv.ReadDotEnv(envPath)
	if env["MAIL_MAILER"] != "log" {
		t.Errorf("MAIL_MAILER = %q, want log", env["MAIL_MAILER"])
	}
}

func TestBindBinaryServiceToAllProjects_Mail(t *testing.T) {
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "app-no-services", Path: "/tmp/a", Type: "laravel"},
			{Name: "app-services-unset", Path: "/tmp/b", Type: "laravel",
				Services: &registry.ProjectServices{Redis: true}},
			{Name: "app-octane", Path: "/tmp/c", Type: "laravel-octane"},
			{Name: "app-other", Path: "/tmp/d", Type: "static"},
			{Name: "app-already", Path: "/tmp/e", Type: "laravel",
				Services: &registry.ProjectServices{Mail: true}},
		},
	}

	if err := BindBinaryServiceToAllProjects(reg, "mail"); err != nil {
		t.Fatalf("BindBinaryServiceToAllProjects returned error: %v", err)
	}

	for _, tc := range []struct {
		name     string
		wantMail bool
	}{
		{"app-no-services", true},
		{"app-services-unset", true},
		{"app-octane", true},
		{"app-other", false},
		{"app-already", true},
	} {
		p := reg.Find(tc.name)
		if p == nil {
			t.Fatalf("project %q not found", tc.name)
		}
		gotMail := p.Services != nil && p.Services.Mail
		if gotMail != tc.wantMail {
			t.Errorf("project %q: Mail = %v, want %v", tc.name, gotMail, tc.wantMail)
		}
	}
}

func TestBindBinaryServiceToAllProjects_S3(t *testing.T) {
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "app-laravel", Path: "/tmp/a", Type: "laravel"},
			{Name: "app-static", Path: "/tmp/b", Type: "static"},
		},
	}

	if err := BindBinaryServiceToAllProjects(reg, "s3"); err != nil {
		t.Fatalf("BindBinaryServiceToAllProjects returned error: %v", err)
	}

	laravelApp := reg.Find("app-laravel")
	if laravelApp == nil || !laravelApp.Services.S3 {
		t.Error("app-laravel: S3 should be true after BindBinaryServiceToAllProjects")
	}
	staticApp := reg.Find("app-static")
	if staticApp != nil && staticApp.Services != nil && staticApp.Services.S3 {
		t.Error("app-static: S3 should not be set for non-Laravel projects")
	}
}

func TestBindBinaryServiceToAllProjects_UnknownServiceReturnsError(t *testing.T) {
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "app", Path: "/tmp/a", Type: "laravel"},
		},
	}

	err := BindBinaryServiceToAllProjects(reg, "bogus")
	if err == nil {
		t.Fatal("expected error for unknown service name, got nil")
	}

	p := reg.Find("app")
	if p != nil && p.Services != nil {
		t.Error("unknown service: must not mutate project Services (guards against silent skips when new binary services are added)")
	}
}

// TestBindBinaryServiceToAllProjects_EnablesProjectsUsingServiceLookup locks
// the contract with registry.ProjectsUsingService — the reason this function
// exists. Regression here would silently break the #69 fix.
func TestBindBinaryServiceToAllProjects_EnablesProjectsUsingServiceLookup(t *testing.T) {
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "pre-linked", Path: "/tmp/a", Type: "laravel"},
			{Name: "pre-linked-octane", Path: "/tmp/b", Type: "laravel-octane"},
			{Name: "static-site", Path: "/tmp/c", Type: "static"},
		},
	}

	if before := reg.ProjectsUsingService("mail"); len(before) != 0 {
		t.Fatalf("precondition: ProjectsUsingService(mail) should be empty before bind, got %d", len(before))
	}

	if err := BindBinaryServiceToAllProjects(reg, "mail"); err != nil {
		t.Fatalf("BindBinaryServiceToAllProjects returned error: %v", err)
	}

	names := map[string]bool{}
	for _, n := range reg.ProjectsUsingService("mail") {
		names[n] = true
	}
	if !names["pre-linked"] || !names["pre-linked-octane"] {
		t.Errorf("ProjectsUsingService(mail) missing laravel projects after bind: got %v", names)
	}
	if names["static-site"] {
		t.Error("ProjectsUsingService(mail) should not include non-Laravel projects")
	}
}
