package cmd

import (
	"os"
	"path/filepath"
	"testing"
)

func TestPhpFromComposer(t *testing.T) {
	tests := []struct {
		name      string
		composer  string
		writeFile bool
		want      string
	}{
		{"no composer.json", "", false, ""},
		{"caret major.minor", `{"require":{"php":"^8.3"}}`, true, "8.3"},
		{"tilde major.minor", `{"require":{"php":"~8.4"}}`, true, "8.4"},
		{"gte major.minor", `{"require":{"php":">=8.2"}}`, true, "8.2"},
		{"exact patch pin captures leading major.minor", `{"require":{"php":"8.4.10"}}`, true, "8.4"},
		{"compound captures leading token", `{"require":{"php":"^8.3 || ^8.4"}}`, true, "8.3"},
		{"non-numeric constraint", `{"require":{"php":"@dev"}}`, true, ""},
		{"empty constraint", `{"require":{"php":""}}`, true, ""},
		{"no require key", `{}`, true, ""},
		{"no php key in require", `{"require":{"monolog/monolog":"^3.0"}}`, true, ""},
		{"malformed json", `{not valid json`, true, ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			dir := t.TempDir()
			if tt.writeFile {
				if err := os.WriteFile(filepath.Join(dir, "composer.json"), []byte(tt.composer), 0o644); err != nil {
					t.Fatal(err)
				}
			}
			got := phpFromComposer(dir)
			if got != tt.want {
				t.Errorf("phpFromComposer(%q) = %q, want %q", tt.composer, got, tt.want)
			}
		})
	}
}
