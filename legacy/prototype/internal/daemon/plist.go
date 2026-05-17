package daemon

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"text/template"

	"github.com/prvious/pv/internal/config"
)

const Label = "dev.prvious.pv"

// PlistConfig holds the values needed to render the launchd plist.
type PlistConfig struct {
	Label        string
	PvBinaryPath string
	LogDir       string
	HomeDir      string
	RunAtLoad    bool
	EnvVars      map[string]string
}

// DefaultPlistConfig returns a PlistConfig populated from the current environment.
// The pv binary path is resolved from the running executable so the plist works
// regardless of where pv was installed (e.g. ~/.local/bin, /usr/local/bin).
func DefaultPlistConfig() PlistConfig {
	pvDir := config.PvDir()

	pvBinary := filepath.Join(config.BinDir(), "pv")
	if exe, err := os.Executable(); err == nil {
		if resolved, err := filepath.EvalSymlinks(exe); err == nil {
			pvBinary = resolved
		}
	}

	return PlistConfig{
		Label:        Label,
		PvBinaryPath: pvBinary,
		LogDir:       config.LogsDir(),
		HomeDir:      pvDir,
		RunAtLoad:    false,
		EnvVars: map[string]string{
			"XDG_DATA_HOME":   pvDir,
			"XDG_CONFIG_HOME": pvDir,
			"PATH":            config.BinDir() + ":/usr/local/bin:/usr/bin:/bin",
		},
	}
}

const plistTmpl = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{{.Label}}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{{.PvBinaryPath}}</string>
        <string>start</string>
    </array>

    <key>RunAtLoad</key>
    <{{.RunAtLoad}}/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>{{.LogDir}}/pv.log</string>

    <key>StandardErrorPath</key>
    <string>{{.LogDir}}/pv.err.log</string>

    <key>WorkingDirectory</key>
    <string>{{.HomeDir}}</string>

    <key>EnvironmentVariables</key>
    <dict>{{range $k, $v := .EnvVars}}
        <key>{{$k}}</key>
        <string>{{$v}}</string>{{end}}
    </dict>
</dict>
</plist>
`

// RenderPlist renders the launchd plist XML from the given config.
func RenderPlist(cfg PlistConfig) ([]byte, error) {
	tmpl, err := template.New("plist").Parse(plistTmpl)
	if err != nil {
		return nil, fmt.Errorf("cannot parse plist template: %w", err)
	}

	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, cfg); err != nil {
		return nil, fmt.Errorf("cannot render plist: %w", err)
	}

	return buf.Bytes(), nil
}

// PlistPath returns the path to the plist file in ~/Library/LaunchAgents/.
func PlistPath() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, "Library", "LaunchAgents", Label+".plist")
}

// WritePlist renders the plist and writes it to ~/Library/LaunchAgents/.
func WritePlist(cfg PlistConfig) error {
	data, err := RenderPlist(cfg)
	if err != nil {
		return err
	}

	dir := filepath.Dir(PlistPath())
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("cannot create LaunchAgents directory: %w", err)
	}

	if err := os.WriteFile(PlistPath(), data, 0644); err != nil {
		return fmt.Errorf("cannot write plist: %w", err)
	}

	return nil
}

// RemovePlist deletes the plist file from ~/Library/LaunchAgents/.
func RemovePlist() error {
	if err := os.Remove(PlistPath()); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("cannot remove plist: %w", err)
	}
	return nil
}
