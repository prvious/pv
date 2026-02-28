package binaries

import (
	"strings"
	"testing"
)

func TestAll_ReturnsThreeBinaries(t *testing.T) {
	all := All()
	if len(all) != 3 {
		t.Fatalf("All() returned %d binaries, want 3", len(all))
	}
}

func TestAll_FrankenPHPFirst(t *testing.T) {
	all := All()
	if all[0].Name != "frankenphp" {
		t.Errorf("All()[0].Name = %q, want %q", all[0].Name, "frankenphp")
	}
}

func TestDownloadURL_FrankenPHP(t *testing.T) {
	url, err := DownloadURL(FrankenPHP, "1.11.3")
	if err != nil {
		t.Fatalf("DownloadURL() error = %v", err)
	}
	if !strings.Contains(url, "v1.11.3") {
		t.Errorf("URL %q does not contain version segment", url)
	}
	if !strings.Contains(url, "github.com/dunglas/frankenphp") {
		t.Errorf("URL %q does not contain FrankenPHP repo", url)
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

func TestDownloadURL_PHP(t *testing.T) {
	url, err := DownloadURL(PHP, "8.5.3")
	if err != nil {
		t.Fatalf("DownloadURL() error = %v", err)
	}
	if !strings.Contains(url, "8.5.3") {
		t.Errorf("URL %q does not contain version segment", url)
	}
	if !strings.Contains(url, "dl.static-php.dev") {
		t.Errorf("URL %q does not contain dl.static-php.dev", url)
	}
	if !strings.HasSuffix(url, ".tar.gz") {
		t.Errorf("URL %q does not end with .tar.gz", url)
	}
	if !strings.Contains(url, "-cli-") {
		t.Errorf("URL %q does not contain -cli-", url)
	}
}

func TestLatestVersionURL_PHP_Empty(t *testing.T) {
	url := LatestVersionURL(PHP)
	if url != "" {
		t.Errorf("LatestVersionURL(PHP) = %q, want empty (version from FrankenPHP)", url)
	}
}

func TestChecksumURL_FrankenPHP_Empty(t *testing.T) {
	url, err := ChecksumURL(FrankenPHP, "1.11.3")
	if err != nil {
		t.Fatalf("ChecksumURL() error = %v", err)
	}
	if url != "" {
		t.Errorf("ChecksumURL(FrankenPHP) = %q, want empty (no checksums published)", url)
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

func TestLatestVersionURL_FrankenPHP(t *testing.T) {
	url := LatestVersionURL(FrankenPHP)
	if !strings.Contains(url, "dunglas/frankenphp") {
		t.Errorf("LatestVersionURL(FrankenPHP) = %q, want dunglas/frankenphp", url)
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
