package colima

import (
	"archive/tar"
	"compress/gzip"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// InstallLima downloads and extracts the Lima runtime (provides limactl).
func InstallLima(client *http.Client, progress func(written, total int64)) error {
	version, err := latestLimaVersion(client)
	if err != nil {
		return fmt.Errorf("cannot resolve Lima version: %w", err)
	}

	arch := runtime.GOARCH
	platform := runtime.GOOS

	limaArch := arch
	if platform == "linux" && arch == "arm64" {
		limaArch = "aarch64"
	} else if arch == "amd64" {
		limaArch = "x86_64"
	}

	platformName := strings.ToUpper(platform[:1]) + platform[1:]
	url := fmt.Sprintf(
		"https://github.com/lima-vm/lima/releases/download/v%s/lima-%s-%s-%s.tar.gz",
		version, version, platformName, limaArch,
	)

	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return err
	}

	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("cannot download Lima: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("cannot download Lima: HTTP %d", resp.StatusCode)
	}

	var r io.Reader = resp.Body
	if progress != nil {
		r = &progressReader{r: resp.Body, total: resp.ContentLength, progress: progress}
	}

	return extractTarGz(r, config.LimaDir())
}

// LimaInstalled checks if limactl exists.
func LimaInstalled() bool {
	_, err := os.Stat(filepath.Join(config.LimaBinDir(), "limactl"))
	return err == nil
}

// RemoveLima removes the entire Lima directory.
func RemoveLima() error {
	return os.RemoveAll(config.LimaDir())
}

func latestLimaVersion(client *http.Client) (string, error) {
	// Use a separate client to disable redirects, but inherit the caller's transport/timeout.
	noRedirect := &http.Client{
		Transport: client.Transport,
		Timeout:   client.Timeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
	resp, err := noRedirect.Head("https://github.com/lima-vm/lima/releases/latest")
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	if resp.StatusCode < 300 || resp.StatusCode >= 400 {
		return "", fmt.Errorf("cannot resolve Lima version: unexpected HTTP %d from GitHub releases", resp.StatusCode)
	}

	loc := resp.Header.Get("Location")
	if loc == "" {
		return "", fmt.Errorf("cannot resolve Lima version: HTTP %d but no Location header", resp.StatusCode)
	}
	parts := strings.Split(loc, "/")
	tag := parts[len(parts)-1]
	return strings.TrimPrefix(tag, "v"), nil
}

func extractTarGz(r io.Reader, dest string) error {
	gz, err := gzip.NewReader(r)
	if err != nil {
		return fmt.Errorf("cannot decompress: %w", err)
	}
	defer gz.Close()

	if err := os.MkdirAll(dest, 0755); err != nil {
		return err
	}

	tr := tar.NewReader(gz)

	for {
		header, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return fmt.Errorf("cannot read archive: %w", err)
		}

		name := header.Name

		// Security: skip absolute paths and parent traversals.
		name = filepath.Clean(name)
		if filepath.IsAbs(name) || strings.Contains(name, "..") {
			continue
		}

		target := filepath.Join(dest, name)
		if !strings.HasPrefix(target, filepath.Clean(dest)+string(os.PathSeparator)) {
			continue
		}

		switch header.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(target, 0755); err != nil {
				return err
			}
		case tar.TypeReg:
			if err := os.MkdirAll(filepath.Dir(target), 0755); err != nil {
				return err
			}
			f, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, os.FileMode(header.Mode))
			if err != nil {
				return err
			}
			_, copyErr := io.Copy(f, tr)
			f.Close()
			if copyErr != nil {
				return copyErr
			}
		case tar.TypeSymlink:
			if err := os.Remove(target); err != nil && !os.IsNotExist(err) {
				return fmt.Errorf("cannot remove existing file before symlink %s: %w", target, err)
			}
			if err := os.Symlink(header.Linkname, target); err != nil {
				return fmt.Errorf("cannot create symlink %s -> %s: %w", target, header.Linkname, err)
			}
		}
	}
	return nil
}
