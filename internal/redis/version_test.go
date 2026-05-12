package redis

import "testing"

func TestParseRedisVersion(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{"Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=fake", "7.4.1"},
		{"Redis server v=8.6.0 sha=00000000:0 malloc=libc bits=64 build=x", "8.6.0"},
	}
	for _, tc := range tests {
		got, err := parseRedisVersion(tc.input)
		if err != nil {
			t.Errorf("parseRedisVersion(%q) error: %v", tc.input, err)
			continue
		}
		if got != tc.want {
			t.Errorf("parseRedisVersion(%q) = %q, want %q", tc.input, got, tc.want)
		}
	}
}

func TestParseRedisVersion_Empty(t *testing.T) {
	if _, err := parseRedisVersion(""); err == nil {
		t.Error("expected error for empty input")
	}
}

func TestParseRedisVersion_Garbage(t *testing.T) {
	if _, err := parseRedisVersion("not redis output at all"); err == nil {
		t.Error("expected error for garbage input")
	}
}
