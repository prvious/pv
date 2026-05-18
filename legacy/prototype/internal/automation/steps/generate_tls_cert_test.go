package steps

import (
	"testing"
)

func TestExpandHostsForCertMinting(t *testing.T) {
	tests := []struct {
		name    string
		project string
		tld     string
		aliases []string
		want    []string
	}{
		{"no aliases", "myapp", "test", nil, []string{"myapp.test"}},
		{"empty aliases", "myapp", "test", []string{}, []string{"myapp.test"}},
		{"one alias", "myapp", "test", []string{"admin.myapp.test"}, []string{"myapp.test", "admin.myapp.test"}},
		{"two aliases", "myapp", "test", []string{"a.test", "b.test"}, []string{"myapp.test", "a.test", "b.test"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := expandHostsForCertMinting(tt.project, tt.tld, tt.aliases)
			if len(got) != len(tt.want) {
				t.Fatalf("len(got) = %d, want %d (got %v, want %v)", len(got), len(tt.want), got, tt.want)
			}
			for i, h := range tt.want {
				if got[i] != h {
					t.Errorf("[%d] = %q, want %q", i, got[i], h)
				}
			}
		})
	}
}
