package project

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestWriteAndParseLaravelContract(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "artisan"), []byte(""), 0o644); err != nil {
		t.Fatalf("WriteFile artisan returned error: %v", err)
	}
	if !DetectLaravel(dir) {
		t.Fatal("DetectLaravel returned false")
	}
	contract := DefaultLaravelContract("Example")
	if err := WriteContract(dir, contract, false); err != nil {
		t.Fatalf("WriteContract returned error: %v", err)
	}
	if err := WriteContract(dir, contract, false); err == nil {
		t.Fatal("WriteContract overwrote existing contract without force")
	}
	loaded, err := LoadContract(dir)
	if err != nil {
		t.Fatalf("LoadContract returned error: %v", err)
	}
	if loaded.Version != ContractVersion || loaded.PHP != "8.4" {
		t.Fatalf("loaded contract = %#v", loaded)
	}
	if loaded.Hosts[0] != "example.test" {
		t.Fatalf("host = %q, want example.test", loaded.Hosts[0])
	}
}

func TestDefaultLaravelContractSanitizesHostLabel(t *testing.T) {
	tests := []struct {
		name string
		want string
	}{
		{name: "My App", want: "my-app.test"},
		{name: "api_v2", want: "api-v2.test"},
		{name: "!!!", want: "app.test"},
		{name: strings.Repeat("a", 64), want: strings.Repeat("a", 63) + ".test"},
	}

	for _, tt := range tests {
		contract := DefaultLaravelContract(tt.name)
		if got := contract.Hosts[0]; got != tt.want {
			t.Fatalf("DefaultLaravelContract(%q) host = %q, want %q", tt.name, got, tt.want)
		}
	}
}

func TestParseContractRejectsUnsupportedFields(t *testing.T) {
	_, err := ParseContract("version: 1\nphp: 8.4\ninfer_from_env: true\nhosts:\n  - app.test\n")
	if err == nil {
		t.Fatal("ParseContract returned nil error")
	}
	if !strings.Contains(err.Error(), "unsupported") {
		t.Fatalf("error = %v, want unsupported field", err)
	}
}
