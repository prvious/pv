package daemon

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRenderPlist_ContainsLabel(t *testing.T) {
	cfg := PlistConfig{
		Label:        "dev.prvious.pv",
		PvBinaryPath: "/Users/test/.pv/bin/pv",
		LogDir:       "/Users/test/.pv/logs",
		HomeDir:      "/Users/test/.pv",
		EnvVars:      map[string]string{},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}
	xml := string(data)

	if !strings.Contains(xml, "<string>dev.prvious.pv</string>") {
		t.Error("plist missing Label")
	}
}

func TestRenderPlist_ProgramArguments(t *testing.T) {
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/home/user/.pv/bin/pv",
		LogDir:       "/home/user/.pv/logs",
		HomeDir:      "/home/user/.pv",
		EnvVars:      map[string]string{},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}
	xml := string(data)

	if !strings.Contains(xml, "<string>/home/user/.pv/bin/pv</string>") {
		t.Error("plist missing binary path in ProgramArguments")
	}
	if !strings.Contains(xml, "<string>start</string>") {
		t.Error("plist missing 'start' in ProgramArguments")
	}
}

func TestRenderPlist_KeepAlive(t *testing.T) {
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/tmp/logs",
		HomeDir:      "/tmp",
		EnvVars:      map[string]string{},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}

	if !strings.Contains(string(data), "<key>KeepAlive</key>") {
		t.Error("plist missing KeepAlive key")
	}
}

func TestRenderPlist_RunAtLoad(t *testing.T) {
	for _, tc := range []struct {
		name     string
		runAt    bool
		expected string
	}{
		{"false by default", false, "<false/>"},
		{"true when enabled", true, "<true/>"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cfg := PlistConfig{
				Label:        Label,
				PvBinaryPath: "/usr/local/bin/pv",
				LogDir:       "/tmp/logs",
				HomeDir:      "/tmp",
				RunAtLoad:    tc.runAt,
				EnvVars:      map[string]string{},
			}

			data, err := RenderPlist(cfg)
			if err != nil {
				t.Fatalf("RenderPlist error: %v", err)
			}

			if !strings.Contains(string(data), tc.expected) {
				t.Errorf("expected RunAtLoad to contain %q", tc.expected)
			}
		})
	}
}

func TestRenderPlist_LogPaths(t *testing.T) {
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/Users/dev/.pv/logs",
		HomeDir:      "/Users/dev/.pv",
		EnvVars:      map[string]string{},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}
	xml := string(data)

	if !strings.Contains(xml, "<string>/Users/dev/.pv/logs/pv.log</string>") {
		t.Error("plist missing stdout log path")
	}
	if !strings.Contains(xml, "<string>/Users/dev/.pv/logs/pv.err.log</string>") {
		t.Error("plist missing stderr log path")
	}
}

func TestRenderPlist_WorkingDirectory(t *testing.T) {
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/tmp/logs",
		HomeDir:      "/Users/dev/.pv",
		EnvVars:      map[string]string{},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}

	if !strings.Contains(string(data), "<key>WorkingDirectory</key>") {
		t.Error("plist missing WorkingDirectory key")
	}
	if !strings.Contains(string(data), "<string>/Users/dev/.pv</string>") {
		t.Error("plist missing WorkingDirectory value")
	}
}

func TestRenderPlist_EnvironmentVariables(t *testing.T) {
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/tmp/logs",
		HomeDir:      "/tmp",
		EnvVars: map[string]string{
			"XDG_DATA_HOME":   "/Users/dev/.pv",
			"XDG_CONFIG_HOME": "/Users/dev/.pv",
			"PATH":            "/Users/dev/.pv/bin:/usr/local/bin:/usr/bin:/bin",
		},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}
	xml := string(data)

	if !strings.Contains(xml, "<key>XDG_DATA_HOME</key>") {
		t.Error("plist missing XDG_DATA_HOME env var")
	}
	if !strings.Contains(xml, "<key>PATH</key>") {
		t.Error("plist missing PATH env var")
	}
	if !strings.Contains(xml, "<key>EnvironmentVariables</key>") {
		t.Error("plist missing EnvironmentVariables section")
	}
}

func TestRenderPlist_DynamicPaths(t *testing.T) {
	// Different users should produce different paths.
	cfg1 := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/Users/alice/.pv/bin/pv",
		LogDir:       "/Users/alice/.pv/logs",
		HomeDir:      "/Users/alice/.pv",
		EnvVars:      map[string]string{},
	}
	cfg2 := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/Users/bob/.pv/bin/pv",
		LogDir:       "/Users/bob/.pv/logs",
		HomeDir:      "/Users/bob/.pv",
		EnvVars:      map[string]string{},
	}

	data1, err := RenderPlist(cfg1)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}
	data2, err := RenderPlist(cfg2)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}

	if strings.Contains(string(data1), "bob") {
		t.Error("alice's plist should not contain bob's paths")
	}
	if strings.Contains(string(data2), "alice") {
		t.Error("bob's plist should not contain alice's paths")
	}
}

func TestRenderPlist_ValidXML(t *testing.T) {
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/tmp/logs",
		HomeDir:      "/tmp",
		EnvVars: map[string]string{
			"PATH": "/usr/bin",
		},
	}

	data, err := RenderPlist(cfg)
	if err != nil {
		t.Fatalf("RenderPlist error: %v", err)
	}
	xml := string(data)

	if !strings.HasPrefix(xml, "<?xml version=") {
		t.Error("plist should start with XML declaration")
	}
	if !strings.Contains(xml, "<!DOCTYPE plist") {
		t.Error("plist should contain DOCTYPE")
	}
	if !strings.Contains(xml, `<plist version="1.0">`) {
		t.Error("plist should contain plist root element")
	}
	if !strings.HasSuffix(strings.TrimSpace(xml), "</plist>") {
		t.Error("plist should end with closing plist tag")
	}
}

func TestDefaultPlistConfig(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := DefaultPlistConfig()

	if cfg.Label != Label {
		t.Errorf("Label = %q, want %q", cfg.Label, Label)
	}
	if cfg.PvBinaryPath == "" || !filepath.IsAbs(cfg.PvBinaryPath) {
		t.Errorf("PvBinaryPath = %q, want non-empty absolute path", cfg.PvBinaryPath)
	}
	if !strings.HasSuffix(cfg.LogDir, filepath.Join(".pv", "logs")) {
		t.Errorf("LogDir = %q, want suffix .pv/logs", cfg.LogDir)
	}
	if !strings.HasSuffix(cfg.HomeDir, ".pv") {
		t.Errorf("HomeDir = %q, want suffix .pv", cfg.HomeDir)
	}
	if cfg.RunAtLoad {
		t.Error("RunAtLoad should default to false")
	}
	if _, ok := cfg.EnvVars["PATH"]; !ok {
		t.Error("EnvVars should include PATH")
	}
	if _, ok := cfg.EnvVars["XDG_DATA_HOME"]; !ok {
		t.Error("EnvVars should include XDG_DATA_HOME")
	}
}

func TestWritePlist(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: filepath.Join(home, ".pv", "bin", "pv"),
		LogDir:       filepath.Join(home, ".pv", "logs"),
		HomeDir:      filepath.Join(home, ".pv"),
		EnvVars:      map[string]string{"PATH": "/usr/bin"},
	}

	if err := WritePlist(cfg); err != nil {
		t.Fatalf("WritePlist error: %v", err)
	}

	plistPath := filepath.Join(home, "Library", "LaunchAgents", Label+".plist")
	data, err := os.ReadFile(plistPath)
	if err != nil {
		t.Fatalf("cannot read written plist: %v", err)
	}

	if !strings.Contains(string(data), Label) {
		t.Error("written plist does not contain label")
	}
}

func TestRemovePlist(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Write first, then remove.
	cfg := PlistConfig{
		Label:        Label,
		PvBinaryPath: "/usr/local/bin/pv",
		LogDir:       "/tmp/logs",
		HomeDir:      "/tmp",
		EnvVars:      map[string]string{},
	}
	if err := WritePlist(cfg); err != nil {
		t.Fatalf("WritePlist error: %v", err)
	}

	if err := RemovePlist(); err != nil {
		t.Fatalf("RemovePlist error: %v", err)
	}

	plistPath := filepath.Join(home, "Library", "LaunchAgents", Label+".plist")
	if _, err := os.Stat(plistPath); !os.IsNotExist(err) {
		t.Error("plist file should not exist after RemovePlist")
	}
}

func TestRemovePlist_NoFileIsOk(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Removing when no file exists should not error.
	if err := RemovePlist(); err != nil {
		t.Fatalf("RemovePlist on missing file should not error: %v", err)
	}
}

func TestPlistPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	path := PlistPath()
	expected := filepath.Join(home, "Library", "LaunchAgents", Label+".plist")
	if path != expected {
		t.Errorf("PlistPath = %q, want %q", path, expected)
	}
}
