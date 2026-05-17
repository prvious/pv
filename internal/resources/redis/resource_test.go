package redis

import (
	"testing"

	"github.com/prvious/pv/internal/control"
)

func TestRedisResourceIsRunnableCache(t *testing.T) {
	desired := Desired("8.0")
	if desired.Resource != control.ResourceRedis {
		t.Fatalf("resource = %q", desired.Resource)
	}
	if Env("8.0")["REDIS_PORT"] != "6379" {
		t.Fatalf("env = %#v", Env("8.0"))
	}
}
