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

func TestValidateVersionRejectsUnsupportedVersions(t *testing.T) {
	for _, version := range []string{"", "7.4", "banana", "8.6/evil"} {
		t.Run(version, func(t *testing.T) {
			if err := ValidateVersion(version); err == nil {
				t.Fatalf("ValidateVersion(%q): want error", version)
			}
		})
	}
}

func TestSetWantedRejectsUnsupportedVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted("7.4", WantedRunning); err == nil {
		t.Fatal("SetWanted unsupported version: want error")
	}

	st, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if len(st.Versions) != 0 {
		t.Fatalf("state versions = %#v, want empty", st.Versions)
	}
}
