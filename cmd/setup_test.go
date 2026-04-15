package cmd

import (
	"testing"
)

func TestBuildServiceOptions_IncludesBothKinds(t *testing.T) {
	opts := buildServiceOptions()
	if len(opts) == 0 {
		t.Fatal("buildServiceOptions() returned empty; want at least one option")
	}

	// Pin specific names from each registry. mysql is Docker; mail and s3 are binary.
	want := []string{"mysql", "postgres", "redis", "mail", "s3"}
	for _, name := range want {
		found := false
		for _, opt := range opts {
			if opt.value == name {
				found = true
				if opt.label == "" {
					t.Errorf("option %q has empty label", name)
				}
				break
			}
		}
		if !found {
			t.Errorf("buildServiceOptions() missing %q", name)
		}
	}
}

func TestBuildServiceOptions_LabelsUseDisplayName(t *testing.T) {
	opts := buildServiceOptions()
	for _, opt := range opts {
		// DisplayName for mail should be "Mail (Mailpit)" — not "mail".
		if opt.value == "mail" && opt.label != "Mail (Mailpit)" {
			t.Errorf("mail label = %q, want %q", opt.label, "Mail (Mailpit)")
		}
		// DisplayName for s3 should be "S3 Storage (RustFS)".
		if opt.value == "s3" && opt.label != "S3 Storage (RustFS)" {
			t.Errorf("s3 label = %q, want %q", opt.label, "S3 Storage (RustFS)")
		}
	}
}
