package project

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type State struct {
	Path     string   `json:"path"`
	PHP      string   `json:"php"`
	Hosts    []string `json:"hosts"`
	Services []string `json:"services"`
}

type Registry struct {
	Path string
}

func (r Registry) Link(ctx context.Context, projectPath string, contract Contract) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := contract.Validate(); err != nil {
		return err
	}
	state := State{
		Path:     projectPath,
		PHP:      contract.PHP,
		Hosts:    append([]string(nil), contract.Hosts...),
		Services: append([]string(nil), contract.Services...),
	}
	if err := os.MkdirAll(filepath.Dir(r.Path), 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return os.WriteFile(r.Path, data, 0o600)
}

type EnvWriter struct {
	Path string
}

func (w EnvWriter) Apply(values map[string]string) error {
	var existing string
	data, err := os.ReadFile(w.Path)
	if err == nil {
		existing = string(data)
		if err := os.WriteFile(w.Path+".bak", data, 0o600); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	clean := removeManagedBlock(existing)
	var b strings.Builder
	b.WriteString(strings.TrimRight(clean, "\n"))
	if b.Len() > 0 {
		b.WriteString("\n")
	}
	b.WriteString("# pv managed begin\n")
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		value := values[key]
		fmt.Fprintf(&b, "%s=%s\n", key, value)
	}
	b.WriteString("# pv managed end\n")
	return os.WriteFile(w.Path, []byte(b.String()), 0o600)
}

type Runner interface {
	Run(context.Context, string, string, map[string]string) error
}

func RunSetup(ctx context.Context, projectPath string, phpBin string, commands []string, runner Runner) error {
	for _, command := range commands {
		if err := ctx.Err(); err != nil {
			return err
		}
		if err := runner.Run(ctx, projectPath, command, map[string]string{"PATH": phpBin}); err != nil {
			return err
		}
	}
	return nil
}

func removeManagedBlock(data string) string {
	start := strings.Index(data, "# pv managed begin")
	end := strings.Index(data, "# pv managed end")
	if start == -1 || end == -1 || end < start {
		return data
	}
	end += len("# pv managed end")
	return data[:start] + data[end:]
}
