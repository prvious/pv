package setup

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestCheckOS_OnCurrentPlatform(t *testing.T) {
	err := CheckOS()
	if runtime.GOOS == "darwin" {
		if err != nil {
			t.Errorf("CheckOS() error = %v on darwin", err)
		}
	} else {
		if err == nil {
			t.Error("CheckOS() should fail on non-darwin")
		}
	}
}

func TestPlatformLabel(t *testing.T) {
	label := PlatformLabel()
	if !strings.Contains(label, runtime.GOOS) {
		t.Errorf("PlatformLabel() = %q, want to contain %q", label, runtime.GOOS)
	}
	if !strings.Contains(label, runtime.GOARCH) {
		t.Errorf("PlatformLabel() = %q, want to contain %q", label, runtime.GOARCH)
	}
}

func TestIsAlreadyInstalled_False(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if IsAlreadyInstalled() {
		t.Error("IsAlreadyInstalled() = true, want false for fresh dir")
	}
}

func TestIsAlreadyInstalled_True(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	pvDir := filepath.Join(home, ".pv")
	if err := os.MkdirAll(pvDir, 0755); err != nil {
		t.Fatal(err)
	}

	if !IsAlreadyInstalled() {
		t.Error("IsAlreadyInstalled() = false, want true when ~/.pv exists")
	}
}
