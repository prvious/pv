package project

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRegistryEnvWriterAndSetupAreExplicit(t *testing.T) {
	ctx := t.Context()
	root := t.TempDir()
	contract := Contract{
		Version:  ContractVersion,
		PHP:      "8.4",
		Hosts:    []string{"app.test"},
		Services: []string{"mailpit"},
		Setup:    []string{"composer install", "php artisan migrate"},
	}
	registry := Registry{Path: filepath.Join(root, "state", "project.json")}
	if err := registry.Link(ctx, root, contract); err != nil {
		t.Fatalf("Link returned error: %v", err)
	}

	envPath := filepath.Join(root, ".env")
	if err := os.WriteFile(envPath, []byte("APP_NAME=Demo\n"), 0o600); err != nil {
		t.Fatalf("WriteFile .env returned error: %v", err)
	}
	writer := EnvWriter{Path: envPath}
	if err := writer.Apply(map[string]string{"MAIL_MAILER": "smtp"}); err != nil {
		t.Fatalf("Apply returned error: %v", err)
	}
	env, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatalf("ReadFile .env returned error: %v", err)
	}
	if !strings.Contains(string(env), "APP_NAME=Demo") || !strings.Contains(string(env), "# pv managed begin") {
		t.Fatalf("env was not preserved with managed block:\n%s", env)
	}
	if _, err := os.Stat(envPath + ".bak"); err != nil {
		t.Fatalf("backup missing: %v", err)
	}

	runner := &recordingRunner{}
	if err := RunSetup(ctx, root, "/tmp/php/bin", contract.Setup, runner); err != nil {
		t.Fatalf("RunSetup returned error: %v", err)
	}
	if len(runner.commands) != 2 || runner.commands[0] != "composer install" {
		t.Fatalf("commands = %#v", runner.commands)
	}
}

type recordingRunner struct {
	commands []string
}

func (r *recordingRunner) Run(_ context.Context, _ string, command string, _ map[string]string) error {
	r.commands = append(r.commands, command)
	return nil
}
