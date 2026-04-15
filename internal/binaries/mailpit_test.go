package binaries

import (
	"runtime"
	"strings"
	"testing"
)

func TestMailpitURL_CurrentPlatform(t *testing.T) {
	url, err := mailpitURL("v1.29.6")
	if err != nil {
		t.Fatalf("unexpected error for %s/%s: %v", runtime.GOOS, runtime.GOARCH, err)
	}
	if !strings.HasPrefix(url, "https://github.com/axllent/mailpit/releases/download/v1.29.6/") {
		t.Errorf("URL = %q; missing expected prefix", url)
	}
	if !strings.HasSuffix(url, ".tar.gz") {
		t.Errorf("URL = %q; expected .tar.gz suffix", url)
	}
}

func TestMailpitArchiveName_AllPlatforms(t *testing.T) {
	tests := []struct {
		goos, goarch, want string
	}{
		{"darwin", "arm64", "mailpit-darwin-arm64.tar.gz"},
		{"darwin", "amd64", "mailpit-darwin-amd64.tar.gz"},
		{"linux", "amd64", "mailpit-linux-amd64.tar.gz"},
		{"linux", "arm64", "mailpit-linux-arm64.tar.gz"},
	}
	for _, tc := range tests {
		archMap, ok := mailpitPlatformNames[tc.goos]
		if !ok {
			t.Errorf("no entry for GOOS=%s", tc.goos)
			continue
		}
		platform, ok := archMap[tc.goarch]
		if !ok {
			t.Errorf("no entry for GOARCH=%s on %s", tc.goarch, tc.goos)
			continue
		}
		got := "mailpit-" + platform + ".tar.gz"
		if got != tc.want {
			t.Errorf("%s/%s: got %q, want %q", tc.goos, tc.goarch, got, tc.want)
		}
	}
}

func TestDownloadURL_MailpitCase(t *testing.T) {
	url, err := DownloadURL(Mailpit, "v1.29.6")
	if err != nil {
		t.Fatalf("DownloadURL returned error: %v", err)
	}
	if url == "" {
		t.Error("DownloadURL returned empty string")
	}
}

func TestLatestVersionURL_MailpitCase(t *testing.T) {
	got := LatestVersionURL(Mailpit)
	want := "https://api.github.com/repos/axllent/mailpit/releases/latest"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
