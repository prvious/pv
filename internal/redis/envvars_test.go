package redis

import "testing"

func TestEnvVars(t *testing.T) {
	got := EnvVars("verify-app")

	want := map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     "6379",
		"REDIS_PASSWORD": "null",
	}
	if len(got) != len(want) {
		t.Fatalf("EnvVars returned %d keys, want %d (%v)", len(got), len(want), got)
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("EnvVars[%q] = %q, want %q", k, got[k], v)
		}
	}
}

func TestEnvVars_ProjectNameIgnored(t *testing.T) {
	a := EnvVars("alpha")
	b := EnvVars("beta")
	for k := range a {
		if a[k] != b[k] {
			t.Errorf("EnvVars varies by projectName: %q differs (%q vs %q)", k, a[k], b[k])
		}
	}
}
