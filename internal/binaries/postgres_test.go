package binaries

import (
	"runtime"
	"testing"
)

func TestPostgresURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("postgres binaries only published for darwin/arm64 in v1")
	}
	got, err := PostgresURL("17")
	if err != nil {
		t.Fatalf("PostgresURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/postgres-mac-arm64-17.tar.gz"
	if got != want {
		t.Errorf("PostgresURL(17) = %q, want %q", got, want)
	}
}

func TestPostgresURL_UnsupportedPlatform(t *testing.T) {
	if runtime.GOOS == "darwin" && runtime.GOARCH == "arm64" {
		t.Skip("on supported platform; this test only runs elsewhere")
	}
	if _, err := PostgresURL("17"); err == nil {
		t.Error("PostgresURL should error on unsupported platform")
	}
}
