package certs

import (
	"os"
	"path/filepath"
	"testing"
)

func TestCertsDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	dir := CertsDir()
	expected := filepath.Join(home, ".pv", "data", "certs")
	if dir != expected {
		t.Errorf("CertsDir() = %q, want %q", dir, expected)
	}
}

func TestCertPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	p := CertPath("myapp.test")
	expected := filepath.Join(home, ".pv", "data", "certs", "myapp.test.crt")
	if p != expected {
		t.Errorf("CertPath() = %q, want %q", p, expected)
	}
}

func TestKeyPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	p := KeyPath("myapp.test")
	expected := filepath.Join(home, ".pv", "data", "certs", "myapp.test.key")
	if p != expected {
		t.Errorf("KeyPath() = %q, want %q", p, expected)
	}
}

func TestGenerateSiteTLS(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	err := GenerateSiteTLS("myapp.test")
	if err == nil {
		t.Fatal("expected error when CA doesn't exist")
	}
}

func TestRemoveSiteTLS(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	certsDir := CertsDir()
	os.MkdirAll(certsDir, 0755)

	certPath := filepath.Join(certsDir, "myapp.test.crt")
	keyPath := filepath.Join(certsDir, "myapp.test.key")
	os.WriteFile(certPath, []byte("cert"), 0644)
	os.WriteFile(keyPath, []byte("key"), 0600)

	if err := RemoveSiteTLS("myapp.test"); err != nil {
		t.Fatalf("RemoveSiteTLS() error = %v", err)
	}

	if _, err := os.Stat(certPath); !os.IsNotExist(err) {
		t.Error("cert file should be removed")
	}
	if _, err := os.Stat(keyPath); !os.IsNotExist(err) {
		t.Error("key file should be removed")
	}
}

func TestRemoveSiteTLS_NonExistent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := RemoveSiteTLS("nonexistent.test"); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestRemoveLinkedCerts(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	certsDir := CertsDir()
	os.MkdirAll(certsDir, 0755)

	for _, name := range []string{"app1.test", "app2.test", "other.test"} {
		os.WriteFile(filepath.Join(certsDir, name+".crt"), []byte("cert"), 0644)
		os.WriteFile(filepath.Join(certsDir, name+".key"), []byte("key"), 0600)
	}

	if err := RemoveLinkedCerts([]string{"app1.test", "app2.test"}); err != nil {
		t.Fatalf("RemoveLinkedCerts() error = %v", err)
	}

	for _, name := range []string{"app1.test", "app2.test"} {
		for _, ext := range []string{".crt", ".key"} {
			if _, err := os.Stat(filepath.Join(certsDir, name+ext)); !os.IsNotExist(err) {
				t.Errorf("%s%s should be removed", name, ext)
			}
		}
	}

	for _, ext := range []string{".crt", ".key"} {
		if _, err := os.Stat(filepath.Join(certsDir, "other.test"+ext)); err != nil {
			t.Errorf("other.test%s should NOT be removed", ext)
		}
	}
}
