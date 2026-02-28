package setup

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestCheckBinary_WithFakeExecutable(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create a fake binary that exits 0.
	fakeBin := filepath.Join(config.BinDir(), "fakecmd")
	if err := os.WriteFile(fakeBin, []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
		t.Fatal(err)
	}

	result := checkBinary("FakeCmd", "fakecmd")
	if result.Err != nil {
		t.Errorf("checkBinary() error = %v, want nil", result.Err)
	}
	if result.Name != "FakeCmd" {
		t.Errorf("Name = %q, want %q", result.Name, "FakeCmd")
	}
}

func TestCheckBinary_MissingBinary(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	result := checkBinary("Missing", "nonexistent")
	if result.Err == nil {
		t.Error("checkBinary() error = nil, want error for missing binary")
	}
}

func TestCheckShim_WithFakeShim(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	fakeShim := filepath.Join(config.BinDir(), "fakeshim")
	if err := os.WriteFile(fakeShim, []byte("#!/bin/sh\necho 'OK'\n"), 0755); err != nil {
		t.Fatal(err)
	}

	result := checkShim("FakeShim", "fakeshim")
	if result.Err != nil {
		t.Errorf("checkShim() error = %v, want nil", result.Err)
	}
}

func TestPrintResults_AllPass(t *testing.T) {
	results := []TestResult{
		{Name: "Test A", Err: nil},
		{Name: "Test B", Err: nil},
	}
	allPassed := PrintResults(results)
	if !allPassed {
		t.Error("PrintResults() = false, want true when all pass")
	}
}

func TestPrintResults_SomeFail(t *testing.T) {
	results := []TestResult{
		{Name: "Test A", Err: nil},
		{Name: "Test B", Err: os.ErrNotExist},
	}
	allPassed := PrintResults(results)
	if allPassed {
		t.Error("PrintResults() = true, want false when some fail")
	}
}
