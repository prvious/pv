package setup

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestDetectShell_Zsh(t *testing.T) {
	t.Setenv("SHELL", "/bin/zsh")
	if got := DetectShell(); got != "zsh" {
		t.Errorf("DetectShell() = %q, want %q", got, "zsh")
	}
}

func TestDetectShell_Bash(t *testing.T) {
	t.Setenv("SHELL", "/usr/bin/bash")
	if got := DetectShell(); got != "bash" {
		t.Errorf("DetectShell() = %q, want %q", got, "bash")
	}
}

func TestDetectShell_Fish(t *testing.T) {
	t.Setenv("SHELL", "/usr/local/bin/fish")
	if got := DetectShell(); got != "fish" {
		t.Errorf("DetectShell() = %q, want %q", got, "fish")
	}
}

func TestDetectShell_Empty(t *testing.T) {
	t.Setenv("SHELL", "")
	if got := DetectShell(); got != "sh" {
		t.Errorf("DetectShell() = %q, want %q", got, "sh")
	}
}

func TestShellConfigFile_Zsh(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ShellConfigFile("zsh")
	want := filepath.Join(home, ".zshrc")
	if got != want {
		t.Errorf("ShellConfigFile(zsh) = %q, want %q", got, want)
	}
}

func TestShellConfigFile_Bash(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ShellConfigFile("bash")
	want := filepath.Join(home, ".bashrc")
	if got != want {
		t.Errorf("ShellConfigFile(bash) = %q, want %q", got, want)
	}
}

func TestShellConfigFile_Fish(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ShellConfigFile("fish")
	if !strings.Contains(got, filepath.Join(".config", "fish", "config.fish")) {
		t.Errorf("ShellConfigFile(fish) = %q, want .config/fish/config.fish", got)
	}
}

func TestShellConfigFile_Default(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ShellConfigFile("sh")
	want := filepath.Join(home, ".profile")
	if got != want {
		t.Errorf("ShellConfigFile(sh) = %q, want %q", got, want)
	}
}

func TestPathExportLine_Zsh(t *testing.T) {
	got := PathExportLine("zsh")
	if !strings.Contains(got, "export PATH") {
		t.Errorf("PathExportLine(zsh) = %q, want export PATH", got)
	}
	if !strings.Contains(got, ".pv/bin") {
		t.Errorf("PathExportLine(zsh) = %q, want .pv/bin", got)
	}
}

func TestPathExportLine_Fish(t *testing.T) {
	got := PathExportLine("fish")
	if !strings.Contains(got, "set -gx PATH") {
		t.Errorf("PathExportLine(fish) = %q, want set -gx PATH", got)
	}
	if !strings.Contains(got, ".pv/bin") {
		t.Errorf("PathExportLine(fish) = %q, want .pv/bin", got)
	}
}
