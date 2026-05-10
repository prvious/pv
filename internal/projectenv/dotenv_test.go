package projectenv

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestReadDotEnv(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	content := `APP_NAME=MyApp
DB_CONNECTION=mysql
DB_HOST=127.0.0.1
DB_PORT=3306
# This is a comment
REDIS_HOST=127.0.0.1
`
	os.WriteFile(envPath, []byte(content), 0644)

	env, err := ReadDotEnv(envPath)
	if err != nil {
		t.Fatalf("ReadDotEnv() error = %v", err)
	}

	if env["APP_NAME"] != "MyApp" {
		t.Errorf("APP_NAME = %q", env["APP_NAME"])
	}
	if env["DB_CONNECTION"] != "mysql" {
		t.Errorf("DB_CONNECTION = %q", env["DB_CONNECTION"])
	}
	if env["REDIS_HOST"] != "127.0.0.1" {
		t.Errorf("REDIS_HOST = %q", env["REDIS_HOST"])
	}
	// Comment should not appear.
	if _, ok := env["# This is a comment"]; ok {
		t.Error("comments should not appear in parsed env")
	}
}

func TestReadDotEnv_Missing(t *testing.T) {
	_, err := ReadDotEnv("/nonexistent/.env")
	if err == nil {
		t.Error("expected error for missing file")
	}
}

func TestMergeDotEnv_ReplaceExisting(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	backupPath := filepath.Join(dir, ".env.pv-backup")

	original := "APP_NAME=MyApp\nDB_HOST=localhost\nDB_PORT=3306\n"
	os.WriteFile(envPath, []byte(original), 0644)

	newVars := map[string]string{
		"DB_HOST": "127.0.0.1",
		"DB_PORT": "33032",
	}

	err := MergeDotEnv(envPath, backupPath, newVars)
	if err != nil {
		t.Fatalf("MergeDotEnv() error = %v", err)
	}

	// Check backup was created.
	backup, err := os.ReadFile(backupPath)
	if err != nil {
		t.Fatalf("backup not created: %v", err)
	}
	if string(backup) != original {
		t.Errorf("backup = %q, want %q", string(backup), original)
	}

	// Check new content.
	result, _ := os.ReadFile(envPath)
	if !strings.Contains(string(result), "DB_HOST=127.0.0.1") {
		t.Error("DB_HOST not updated")
	}
	if !strings.Contains(string(result), "DB_PORT=33032") {
		t.Error("DB_PORT not updated")
	}
	if !strings.Contains(string(result), "APP_NAME=MyApp") {
		t.Error("APP_NAME should be preserved")
	}
}

func TestMergeDotEnv_AppendNew(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")

	original := "APP_NAME=MyApp\n"
	os.WriteFile(envPath, []byte(original), 0644)

	newVars := map[string]string{
		"DB_HOST": "127.0.0.1",
	}

	err := MergeDotEnv(envPath, "", newVars)
	if err != nil {
		t.Fatalf("MergeDotEnv() error = %v", err)
	}

	result, _ := os.ReadFile(envPath)
	if !strings.Contains(string(result), "DB_HOST=127.0.0.1") {
		t.Error("DB_HOST not appended")
	}
}

func TestMergeDotEnv_NewFile(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")

	newVars := map[string]string{
		"DB_HOST": "127.0.0.1",
		"DB_PORT": "33032",
	}

	err := MergeDotEnv(envPath, "", newVars)
	if err != nil {
		t.Fatalf("MergeDotEnv() error = %v", err)
	}

	result, _ := os.ReadFile(envPath)
	if !strings.Contains(string(result), "DB_HOST=127.0.0.1") {
		t.Error("DB_HOST not written")
	}
}
