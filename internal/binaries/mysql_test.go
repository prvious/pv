package binaries

import (
	"runtime"
	"testing"
)

func TestMysqlURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("mysql binaries only published for darwin/arm64 in v1")
	}
	tests := []struct {
		version string
		want    string
	}{
		{"8.0", "https://github.com/prvious/pv/releases/download/artifacts/mysql-mac-arm64-8.0.tar.gz"},
		{"8.4", "https://github.com/prvious/pv/releases/download/artifacts/mysql-mac-arm64-8.4.tar.gz"},
		{"9.7", "https://github.com/prvious/pv/releases/download/artifacts/mysql-mac-arm64-9.7.tar.gz"},
	}
	for _, tt := range tests {
		got, err := MysqlURL(tt.version)
		if err != nil {
			t.Errorf("MysqlURL(%q): %v", tt.version, err)
			continue
		}
		if got != tt.want {
			t.Errorf("MysqlURL(%q) = %q, want %q", tt.version, got, tt.want)
		}
	}
}

func TestMysqlURL_UnsupportedPlatform(t *testing.T) {
	if runtime.GOOS == "darwin" && runtime.GOARCH == "arm64" {
		t.Skip("on supported platform; this test only runs elsewhere")
	}
	if _, err := MysqlURL("8.4"); err == nil {
		t.Error("MysqlURL should error on unsupported platform")
	}
}

func TestMysqlURL_InvalidVersion(t *testing.T) {
	if _, err := MysqlURL(""); err == nil {
		t.Error("MysqlURL empty should error")
	}
	if _, err := MysqlURL("7.4"); err == nil {
		t.Error("MysqlURL with unsupported version should error")
	}
	if _, err := MysqlURL("latest"); err == nil {
		t.Error("MysqlURL with non-numeric version should error")
	}
}

func TestMysqlURL_OverrideEnv(t *testing.T) {
	t.Setenv("PV_MYSQL_URL_OVERRIDE", "http://127.0.0.1:9999/mysql-test.tar.gz")
	got, err := MysqlURL("8.4")
	if err != nil {
		t.Fatalf("MysqlURL: %v", err)
	}
	want := "http://127.0.0.1:9999/mysql-test.tar.gz"
	if got != want {
		t.Errorf("MysqlURL with override = %q, want %q", got, want)
	}
}

func TestIsValidMysqlVersion(t *testing.T) {
	for _, v := range []string{"8.0", "8.4", "9.7"} {
		if !IsValidMysqlVersion(v) {
			t.Errorf("IsValidMysqlVersion(%q) = false, want true", v)
		}
	}
	for _, v := range []string{"", "7.4", "latest", "8.4.1", "9"} {
		if IsValidMysqlVersion(v) {
			t.Errorf("IsValidMysqlVersion(%q) = true, want false", v)
		}
	}
}
