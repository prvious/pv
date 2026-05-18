package fixtures

import (
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/project"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestNewLaravelFixtureIsDeterministic(t *testing.T) {
	firstSandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new first sandbox: %v", err)
	}
	secondSandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new second sandbox: %v", err)
	}

	first, err := NewLaravel(firstSandbox, WithName("Acme"))
	if err != nil {
		t.Fatalf("new first fixture: %v", err)
	}
	second, err := NewLaravel(secondSandbox, WithName("Acme"))
	if err != nil {
		t.Fatalf("new second fixture: %v", err)
	}

	firstSnapshot := snapshotFixture(t, first.Root)
	secondSnapshot := snapshotFixture(t, second.Root)
	if !reflect.DeepEqual(firstSnapshot, secondSnapshot) {
		t.Fatalf("fixtures are not deterministic:\nfirst: %#v\nsecond: %#v", firstSnapshot, secondSnapshot)
	}
}

func TestNewLaravelFixtureSupportsInitScenarios(t *testing.T) {
	sandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}

	fixture, err := NewLaravel(sandbox, WithName("Acme"))
	if err != nil {
		t.Fatalf("new fixture: %v", err)
	}

	if fixture.Root != sandbox.ProjectRoot {
		t.Fatalf("fixture root = %s, want sandbox project root %s", fixture.Root, sandbox.ProjectRoot)
	}
	if !project.DetectLaravel(fixture.Root) {
		t.Fatal("fixture was not detected as Laravel")
	}
	assertMissing(t, filepath.Join(fixture.Root, "pv.yml"))
	assertMissing(t, filepath.Join(fixture.Root, ".env"))
	assertMissing(t, filepath.Join(fixture.Root, "composer.lock"))
	assertMissing(t, filepath.Join(fixture.Root, "vendor"))
	assertFileContains(t, filepath.Join(fixture.Root, "composer.json"), "laravel/framework")
	assertFileContains(t, filepath.Join(fixture.Root, ".env.example"), "APP_ENV=local")
}

func TestLaravelFixtureSupportsLinkSetupAndResourceMutations(t *testing.T) {
	sandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}
	fixture, err := NewLaravel(sandbox, WithName("Acme"))
	if err != nil {
		t.Fatalf("new fixture: %v", err)
	}

	contract, err := fixture.WriteContract(
		WithHosts("acme.test", "admin.acme.test"),
		WithServices("postgres", "redis", "mailpit", "rustfs"),
		WithSetup("printf setup > storage/logs/setup.log"),
	)
	if err != nil {
		t.Fatalf("write fixture contract: %v", err)
	}
	if !slices.Equal(contract.Services, []string{"postgres", "redis", "mailpit", "rustfs"}) {
		t.Fatalf("services = %#v", contract.Services)
	}
	if !slices.Equal(contract.Setup, []string{"printf setup > storage/logs/setup.log"}) {
		t.Fatalf("setup = %#v", contract.Setup)
	}
	loaded, err := project.LoadContract(fixture.Root)
	if err != nil {
		t.Fatalf("load fixture contract: %v", err)
	}
	if !reflect.DeepEqual(loaded, contract) {
		t.Fatalf("loaded contract = %#v, want %#v", loaded, contract)
	}

	broken, err := fixture.WriteContract(WithBrokenSetup())
	if err != nil {
		t.Fatalf("write broken setup contract: %v", err)
	}
	if !slices.Equal(broken.Setup, []string{"false"}) {
		t.Fatalf("broken setup = %#v", broken.Setup)
	}

	if err := fixture.WriteEnv("APP_NAME=Existing\nUSER_SECRET=keep\n"); err != nil {
		t.Fatalf("write fixture env: %v", err)
	}
	assertFileContains(t, filepath.Join(fixture.Root, ".env"), "USER_SECRET=keep")
}

func TestLaravelFixtureFilesStayUnderSandboxProjectRoot(t *testing.T) {
	sandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}
	fixture, err := NewLaravel(sandbox)
	if err != nil {
		t.Fatalf("new fixture: %v", err)
	}
	if err := fixture.WriteEnv("APP_NAME=Scoped\n"); err != nil {
		t.Fatalf("write fixture env: %v", err)
	}
	if _, err := fixture.WriteContract(WithServices("mailpit")); err != nil {
		t.Fatalf("write fixture contract: %v", err)
	}

	err = filepath.WalkDir(fixture.Root, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		assertWithin(t, sandbox.ProjectRoot, path)
		return nil
	})
	if err != nil {
		t.Fatalf("walk fixture root: %v", err)
	}
}

func snapshotFixture(t *testing.T, root string) map[string]string {
	t.Helper()

	snapshot := map[string]string{}
	err := filepath.WalkDir(root, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		rel, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		snapshot[rel] = string(data)
		return nil
	})
	if err != nil {
		t.Fatalf("snapshot fixture: %v", err)
	}
	return snapshot
}

func assertMissing(t *testing.T, path string) {
	t.Helper()

	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("expected %s to be missing, got stat error %v", path, err)
	}
}

func assertFileContains(t *testing.T, path string, needle string) {
	t.Helper()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	if !strings.Contains(string(data), needle) {
		t.Fatalf("expected %s to contain %q, got:\n%s", path, needle, data)
	}
}

func assertWithin(t *testing.T, root string, path string) {
	t.Helper()

	rel, err := filepath.Rel(root, path)
	if err != nil {
		t.Fatalf("rel %s to %s: %v", path, root, err)
	}
	if rel == ".." || strings.HasPrefix(rel, ".."+string(os.PathSeparator)) {
		t.Fatalf("expected %s to be inside %s", path, root)
	}
}
