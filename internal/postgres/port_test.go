package postgres

import "testing"

func TestPortFor(t *testing.T) {
	tests := []struct {
		major string
		want  int
	}{
		{"17", 54017},
		{"18", 54018},
		{"99", 54099},
	}
	for _, tt := range tests {
		got, err := PortFor(tt.major)
		if err != nil {
			t.Errorf("PortFor(%q): %v", tt.major, err)
			continue
		}
		if got != tt.want {
			t.Errorf("PortFor(%q) = %d, want %d", tt.major, got, tt.want)
		}
	}
}

func TestPortFor_Invalid(t *testing.T) {
	if _, err := PortFor(""); err == nil {
		t.Error("PortFor empty should error")
	}
	if _, err := PortFor("18-alpine"); err == nil {
		t.Error("PortFor non-numeric should error")
	}
}
