package registry

import (
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestServiceInstance_LegacyKindFieldIsIgnored(t *testing.T) {
	const legacy = `{
		"port": 9000,
		"console_port": 9001,
		"kind": "binary",
		"enabled": true
	}`
	var inst ServiceInstance
	if err := json.Unmarshal([]byte(legacy), &inst); err != nil {
		t.Fatalf("legacy registry should still parse: %v", err)
	}
	if inst.Port != 9000 {
		t.Errorf("Port: got %d, want 9000", inst.Port)
	}
	if inst.ConsolePort != 9001 {
		t.Errorf("ConsolePort: got %d, want 9001", inst.ConsolePort)
	}
	if inst.Enabled == nil || !*inst.Enabled {
		t.Errorf("Enabled: got %v, want non-nil true", inst.Enabled)
	}
}

func TestLoad_IgnoresLegacyKindField(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs: %v", err)
	}
	legacy := `{
		"services": {
			"s3": {"port": 9000, "console_port": 9001, "kind": "binary", "enabled": true}
		},
		"projects": []
	}`
	if err := os.WriteFile(config.RegistryPath(), []byte(legacy), 0o644); err != nil {
		t.Fatalf("write registry: %v", err)
	}

	reg, err := Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	s3 := reg.Services["s3"]
	if s3 == nil {
		t.Fatal("s3 entry missing after Load")
	}
	if s3.Port != 9000 {
		t.Errorf("s3.Port: got %d, want 9000", s3.Port)
	}
	if s3.Enabled == nil || !*s3.Enabled {
		t.Errorf("s3.Enabled: got %v, want non-nil true", s3.Enabled)
	}
}

func TestServiceInstance_RoundTripDropsKind(t *testing.T) {
	enabled := true
	inst := ServiceInstance{Port: 9000, Enabled: &enabled}
	out, err := json.Marshal(&inst)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if strings.Contains(string(out), `"kind"`) {
		t.Errorf("re-saved JSON contains kind: %s", out)
	}
}
