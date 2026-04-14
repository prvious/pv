package phpenv

import (
	"testing"
)

func TestPlatformNameFor(t *testing.T) {
	tests := []struct {
		goos, goarch string
		want         string
		wantErr      bool
	}{
		{"darwin", "arm64", "mac-arm64", false},
		{"darwin", "amd64", "mac-x86_64", false},
		{"linux", "amd64", "linux-x86_64", false},
		{"linux", "arm64", "linux-aarch64", false},
		{"windows", "amd64", "", true},
		{"darwin", "riscv64", "", true},
	}

	for _, tt := range tests {
		name := tt.goos + "/" + tt.goarch
		t.Run(name, func(t *testing.T) {
			got, err := platformNameFor(tt.goos, tt.goarch)
			if tt.wantErr {
				if err == nil {
					t.Errorf("platformNameFor(%q, %q) error = nil, want error", tt.goos, tt.goarch)
				}
				return
			}
			if err != nil {
				t.Fatalf("platformNameFor(%q, %q) error = %v", tt.goos, tt.goarch, err)
			}
			if got != tt.want {
				t.Errorf("platformNameFor(%q, %q) = %q, want %q", tt.goos, tt.goarch, got, tt.want)
			}
		})
	}
}

func TestFrankenPHPAssetName(t *testing.T) {
	platform, err := platformName()
	if err != nil {
		t.Skipf("unsupported platform: %v", err)
	}

	got, err := frankenphpAssetName("8.4")
	if err != nil {
		t.Fatalf("frankenphpAssetName() error = %v", err)
	}
	want := "frankenphp-" + platform + "-php8.4"
	if got != want {
		t.Errorf("frankenphpAssetName() = %q, want %q", got, want)
	}
}

func TestPHPCLIURL(t *testing.T) {
	platform, err := platformName()
	if err != nil {
		t.Skipf("unsupported platform: %v", err)
	}

	got, err := phpCLIURL("v1.2.3", "8.4")
	if err != nil {
		t.Fatalf("phpCLIURL() error = %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/v1.2.3/php-" + platform + "-php8.4.tar.gz"
	if got != want {
		t.Errorf("phpCLIURL() = %q, want %q", got, want)
	}
}
