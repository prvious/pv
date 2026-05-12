package binaries

import (
	"runtime"
	"testing"
)

func TestRedisURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("redis binaries only published for darwin/arm64 in v1")
	}
	got, err := RedisURL()
	if err != nil {
		t.Fatalf("RedisURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/redis-mac-arm64-8.6.tar.gz"
	if got != want {
		t.Errorf("RedisURL = %q, want %q", got, want)
	}
}

func TestRedisURL_UnsupportedPlatform(t *testing.T) {
	if runtime.GOOS == "darwin" && runtime.GOARCH == "arm64" {
		t.Skip("on supported platform; this test only runs elsewhere")
	}
	if _, err := RedisURL(); err == nil {
		t.Error("RedisURL should error on unsupported platform")
	}
}

func TestRedisURL_OverrideEnv(t *testing.T) {
	t.Setenv("PV_REDIS_URL_OVERRIDE", "http://127.0.0.1:9999/redis-test.tar.gz")
	got, err := RedisURL()
	if err != nil {
		t.Fatalf("RedisURL: %v", err)
	}
	want := "http://127.0.0.1:9999/redis-test.tar.gz"
	if got != want {
		t.Errorf("RedisURL with override = %q, want %q", got, want)
	}
}
