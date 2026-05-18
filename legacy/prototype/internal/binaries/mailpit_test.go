package binaries

import (
	"runtime"
	"testing"
)

func TestMailpitURL(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("mailpit binaries only published for darwin/arm64 in v1")
	}
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", "")
	url, err := MailpitURL("1")
	if err != nil {
		t.Fatalf("MailpitURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/mailpit-mac-arm64-1.tar.gz"
	if url != want {
		t.Fatalf("MailpitURL = %q, want %q", url, want)
	}
}

func TestMailpitURL_Override(t *testing.T) {
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", "http://example.test/mailpit.tar.gz")
	url, err := MailpitURL("1")
	if err != nil {
		t.Fatalf("MailpitURL override: %v", err)
	}
	if url != "http://example.test/mailpit.tar.gz" {
		t.Fatalf("MailpitURL override = %q", url)
	}
}

func TestDownloadURL_MailpitCase(t *testing.T) {
	if runtime.GOOS != "darwin" || runtime.GOARCH != "arm64" {
		t.Skip("mailpit binaries only published for darwin/arm64 in v1")
	}
	url, err := DownloadURL(Mailpit, "1")
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
