package tools

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestExpose_Symlink(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create a fake binary in internal/bin.
	fakeBin := filepath.Join(config.InternalBinDir(), "mago")
	if err := os.WriteFile(fakeBin, []byte("fake"), 0755); err != nil {
		t.Fatal(err)
	}

	tool := &Tool{
		Name:       "mago",
		AutoExpose: true,
		Exposure:   ExposureSymlink,
		InternalPath: func() string {
			return fakeBin
		},
	}

	if err := Expose(tool); err != nil {
		t.Fatalf("Expose() error = %v", err)
	}

	linkPath := filepath.Join(config.BinDir(), "mago")
	target, err := os.Readlink(linkPath)
	if err != nil {
		t.Fatalf("symlink not created: %v", err)
	}
	if target != fakeBin {
		t.Errorf("symlink target = %q, want %q", target, fakeBin)
	}
}

func TestExpose_Shim(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	php := Get("php")
	if err := Expose(php); err != nil {
		t.Fatalf("Expose(php) error = %v", err)
	}

	shimPath := filepath.Join(config.BinDir(), "php")
	info, err := os.Stat(shimPath)
	if err != nil {
		t.Fatalf("php shim not created: %v", err)
	}
	if info.Mode()&0111 == 0 {
		t.Error("php shim is not executable")
	}

	content, _ := os.ReadFile(shimPath)
	if !strings.Contains(string(content), "#!/bin/bash") {
		t.Error("php shim missing shebang")
	}
}

func TestUnexpose(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create a fake file in bin/.
	fakePath := filepath.Join(config.BinDir(), "mago")
	if err := os.WriteFile(fakePath, []byte("fake"), 0755); err != nil {
		t.Fatal(err)
	}

	tool := &Tool{
		Name:     "mago",
		Exposure: ExposureSymlink,
	}

	if err := Unexpose(tool); err != nil {
		t.Fatalf("Unexpose() error = %v", err)
	}

	if _, err := os.Stat(fakePath); !os.IsNotExist(err) {
		t.Error("expected file to be removed")
	}
}

func TestUnexpose_NonExistent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	tool := &Tool{Name: "nonexistent", Exposure: ExposureSymlink}
	if err := Unexpose(tool); err != nil {
		t.Fatalf("Unexpose() on missing file should not error, got: %v", err)
	}
}

func TestIsExposed(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	tool := &Tool{Name: "mago", Exposure: ExposureSymlink}

	if IsExposed(tool) {
		t.Error("IsExposed() = true before expose")
	}

	// Create the file.
	if err := os.WriteFile(filepath.Join(config.BinDir(), "mago"), []byte("x"), 0755); err != nil {
		t.Fatal(err)
	}

	if !IsExposed(tool) {
		t.Error("IsExposed() = false after creating file")
	}
}

func TestExposeAll(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create fake mago binary so symlink target exists.
	fakeMago := filepath.Join(config.InternalBinDir(), "mago")
	if err := os.WriteFile(fakeMago, []byte("fake"), 0755); err != nil {
		t.Fatal(err)
	}

	if err := ExposeAll(); err != nil {
		t.Fatalf("ExposeAll() error = %v", err)
	}

	// PHP shim should exist (AutoExpose=true, ExposureShim).
	if _, err := os.Stat(filepath.Join(config.BinDir(), "php")); err != nil {
		t.Error("php shim not created by ExposeAll")
	}

	// Composer shim should exist.
	if _, err := os.Stat(filepath.Join(config.BinDir(), "composer")); err != nil {
		t.Error("composer shim not created by ExposeAll")
	}

	// Mago symlink should exist.
	if _, err := os.Lstat(filepath.Join(config.BinDir(), "mago")); err != nil {
		t.Error("mago symlink not created by ExposeAll")
	}

	// Colima should NOT be exposed (AutoExpose=false).
	if _, err := os.Lstat(filepath.Join(config.BinDir(), "colima")); err == nil {
		t.Error("colima should not be exposed by ExposeAll")
	}
}

func TestExpose_None(t *testing.T) {
	tool := &Tool{Name: "colima", Exposure: ExposureNone}
	if err := Expose(tool); err != nil {
		t.Fatalf("Expose(ExposureNone) should be no-op, got: %v", err)
	}
}

func TestGet(t *testing.T) {
	if Get("php") == nil {
		t.Error("Get(php) = nil")
	}
	if Get("nonexistent") != nil {
		t.Error("Get(nonexistent) should be nil")
	}
}

func TestList(t *testing.T) {
	list := List()
	if len(list) != len(registry) {
		t.Errorf("List() returned %d tools, want %d", len(list), len(registry))
	}
	// Verify sorted.
	for i := 1; i < len(list); i++ {
		if list[i].Name < list[i-1].Name {
			t.Errorf("List() not sorted: %s before %s", list[i-1].Name, list[i].Name)
		}
	}
}

func TestMustGet(t *testing.T) {
	// Known tool should not panic.
	tool := MustGet("php")
	if tool == nil {
		t.Error("MustGet(php) = nil")
	}

	// Unknown tool should panic.
	defer func() {
		if r := recover(); r == nil {
			t.Error("MustGet(unknown) did not panic")
		}
	}()
	MustGet("nonexistent")
}

func TestRegistryIntegrity(t *testing.T) {
	for name, tool := range registry {
		if tool.Name != name {
			t.Errorf("tool %q: Name=%q does not match registry key", name, tool.Name)
		}
		if tool.DisplayName == "" {
			t.Errorf("tool %q: DisplayName is empty", name)
		}
		if tool.InternalPath == nil {
			t.Errorf("tool %q: InternalPath is nil", name)
		}
		if tool.Exposure == ExposureShim && tool.WriteShim == nil {
			t.Errorf("tool %q: ExposureShim requires WriteShim func", name)
		}
	}
}
