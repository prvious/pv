package binaries

import (
	"strings"
	"testing"
)

func TestTools_ReturnsTwoBinaries(t *testing.T) {
	tools := Tools()
	if len(tools) != 2 {
		t.Fatalf("Tools() returned %d binaries, want 2", len(tools))
	}
}

func TestTools_MagoFirst(t *testing.T) {
	tools := Tools()
	if tools[0].Name != "mago" {
		t.Errorf("Tools()[0].Name = %q, want %q", tools[0].Name, "mago")
	}
	if tools[1].Name != "composer" {
		t.Errorf("Tools()[1].Name = %q, want %q", tools[1].Name, "composer")
	}
}

func TestDownloadURL_Mago(t *testing.T) {
	url, err := DownloadURL(Mago, "1.13.2")
	if err != nil {
		t.Fatalf("DownloadURL() error = %v", err)
	}
	if !strings.Contains(url, "1.13.2") {
		t.Errorf("URL %q does not contain version segment", url)
	}
	if !strings.Contains(url, "github.com/carthage-software/mago") {
		t.Errorf("URL %q does not contain Mago repo", url)
	}
	if !strings.HasSuffix(url, ".tar.gz") {
		t.Errorf("URL %q does not end with .tar.gz", url)
	}
}

func TestDownloadURL_Composer(t *testing.T) {
	url1, err := DownloadURL(Composer, "2.7.0")
	if err != nil {
		t.Fatalf("DownloadURL() error = %v", err)
	}
	url2, err := DownloadURL(Composer, "2.8.0")
	if err != nil {
		t.Fatalf("DownloadURL() error = %v", err)
	}
	if url1 != url2 {
		t.Errorf("Composer URLs differ for different versions: %q vs %q", url1, url2)
	}
	if !strings.Contains(url1, "composer.phar") {
		t.Errorf("URL %q does not contain composer.phar", url1)
	}
}

func TestDownloadURL_Unknown(t *testing.T) {
	_, err := DownloadURL(Binary{Name: "unknown"}, "1.0")
	if err == nil {
		t.Error("DownloadURL(unknown) should return error")
	}
}

func TestChecksumURL_Mago_Empty(t *testing.T) {
	url, err := ChecksumURL(Mago, "1.13.2")
	if err != nil {
		t.Fatalf("ChecksumURL() error = %v", err)
	}
	if url != "" {
		t.Errorf("ChecksumURL(Mago) = %q, want empty", url)
	}
}

func TestChecksumURL_Composer(t *testing.T) {
	url, err := ChecksumURL(Composer, "latest")
	if err != nil {
		t.Fatalf("ChecksumURL() error = %v", err)
	}
	if !strings.Contains(url, "sha256") {
		t.Errorf("ChecksumURL(Composer) = %q, want sha256 in URL", url)
	}
}

func TestLatestVersionURL_Mago(t *testing.T) {
	url := LatestVersionURL(Mago)
	if !strings.Contains(url, "carthage-software/mago") {
		t.Errorf("LatestVersionURL(Mago) = %q, want carthage-software/mago", url)
	}
}

func TestLatestVersionURL_Composer_Empty(t *testing.T) {
	url := LatestVersionURL(Composer)
	if url != "" {
		t.Errorf("LatestVersionURL(Composer) = %q, want empty", url)
	}
}
