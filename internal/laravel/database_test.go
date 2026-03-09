package laravel

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveDatabaseName_FromEnvExample(t *testing.T) {
	dir := t.TempDir()
	envExample := filepath.Join(dir, ".env.example")
	os.WriteFile(envExample, []byte("DB_DATABASE=my_custom_db\n"), 0644)

	got := ResolveDatabaseName(dir, "my-project")
	if got != "my_custom_db" {
		t.Errorf("ResolveDatabaseName() = %q, want %q", got, "my_custom_db")
	}
}

func TestResolveDatabaseName_IgnoresGenericLaravel(t *testing.T) {
	dir := t.TempDir()
	envExample := filepath.Join(dir, ".env.example")
	os.WriteFile(envExample, []byte("DB_DATABASE=laravel\n"), 0644)

	got := ResolveDatabaseName(dir, "my-project")
	if got != "my_project" {
		t.Errorf("ResolveDatabaseName() = %q, want %q", got, "my_project")
	}
}

func TestResolveDatabaseName_NoEnvExample(t *testing.T) {
	dir := t.TempDir()

	got := ResolveDatabaseName(dir, "my-project")
	if got != "my_project" {
		t.Errorf("ResolveDatabaseName() = %q, want %q", got, "my_project")
	}
}

func TestResolveDatabaseName_NoDBDatabaseKey(t *testing.T) {
	dir := t.TempDir()
	envExample := filepath.Join(dir, ".env.example")
	os.WriteFile(envExample, []byte("APP_NAME=MyApp\nAPP_ENV=local\n"), 0644)

	got := ResolveDatabaseName(dir, "my-project")
	if got != "my_project" {
		t.Errorf("ResolveDatabaseName() = %q, want %q", got, "my_project")
	}
}

func TestResolveDatabaseName_EmptyDBDatabase(t *testing.T) {
	dir := t.TempDir()
	envExample := filepath.Join(dir, ".env.example")
	os.WriteFile(envExample, []byte("DB_DATABASE=\n"), 0644)

	got := ResolveDatabaseName(dir, "my-project")
	if got != "my_project" {
		t.Errorf("ResolveDatabaseName() = %q, want %q", got, "my_project")
	}
}

func TestResolveDatabaseName_SanitizesHyphens(t *testing.T) {
	dir := t.TempDir()

	got := ResolveDatabaseName(dir, "my-cool-app")
	if got != "my_cool_app" {
		t.Errorf("ResolveDatabaseName() = %q, want %q", got, "my_cool_app")
	}
}
