package registry

import (
	"encoding/json"
	"strings"
	"testing"
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
