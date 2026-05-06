package binaries

import (
	"archive/tar"
	"archive/zip"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// ErrEntryNotFound is returned by ExtractTarGz when the requested entry
// is not present in the archive. Callers can use errors.Is to tolerate
// optional entries (e.g. files added in newer artifact builds).
var ErrEntryNotFound = errors.New("entry not found in archive")

// ProgressFunc is called during download with bytes written so far and total size.
// total may be -1 if Content-Length is not available.
type ProgressFunc func(written, total int64)

// Download fetches a URL to destPath atomically via temp file + rename.
func Download(client *http.Client, url, destPath string) error {
	return DownloadProgress(client, url, destPath, nil)
}

// DownloadProgress fetches a URL to destPath with optional progress reporting.
func DownloadProgress(client *http.Client, url, destPath string, progress ProgressFunc) error {
	resp, err := client.Get(url)
	if err != nil {
		return fmt.Errorf("download failed: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download failed: HTTP %d for %s", resp.StatusCode, url)
	}

	dir := filepath.Dir(destPath)
	tmp, err := os.CreateTemp(dir, ".pv-download-*")
	if err != nil {
		return fmt.Errorf("cannot create temp file: %w", err)
	}
	tmpPath := tmp.Name()

	var reader io.Reader = resp.Body
	if progress != nil {
		total := resp.ContentLength
		reader = &progressReader{reader: resp.Body, total: total, fn: progress}
	}

	if _, err := io.Copy(tmp, reader); err != nil {
		tmp.Close()
		os.Remove(tmpPath)
		return fmt.Errorf("download write failed: %w", err)
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmpPath)
		return err
	}

	if err := os.Rename(tmpPath, destPath); err != nil {
		os.Remove(tmpPath)
		return fmt.Errorf("cannot rename temp file: %w", err)
	}
	return nil
}

type progressReader struct {
	reader  io.Reader
	total   int64
	written int64
	fn      ProgressFunc
}

func (r *progressReader) Read(p []byte) (int, error) {
	n, err := r.reader.Read(p)
	r.written += int64(n)
	if r.fn != nil {
		r.fn(r.written, r.total)
	}
	return n, err
}

// VerifyChecksum checks that the SHA256 of filePath matches expectedHex.
// expectedHex may be in "hash  filename" format (as produced by sha256sum).
func VerifyChecksum(filePath, expectedHex string) error {
	// Parse "hash  filename" format
	expected := strings.Fields(expectedHex)[0]

	f, err := os.Open(filePath)
	if err != nil {
		return err
	}
	defer f.Close()

	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return err
	}

	actual := hex.EncodeToString(h.Sum(nil))
	if actual != expected {
		return fmt.Errorf("checksum mismatch: got %s, want %s", actual, expected)
	}
	return nil
}

// FetchChecksum downloads a checksum string from a URL.
func FetchChecksum(client *http.Client, checksumURL string) (string, error) {
	resp, err := client.Get(checksumURL)
	if err != nil {
		return "", fmt.Errorf("fetch checksum failed: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("fetch checksum failed: HTTP %d", resp.StatusCode)
	}

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(data)), nil
}

// ExtractTarGz extracts a single binary named binaryName from a tar.gz archive
// and writes it to destPath.
func ExtractTarGz(archivePath, destPath, binaryName string) error {
	f, err := os.Open(archivePath)
	if err != nil {
		return err
	}
	defer f.Close()

	gz, err := gzip.NewReader(f)
	if err != nil {
		return fmt.Errorf("gzip open failed: %w", err)
	}
	defer gz.Close()

	tr := tar.NewReader(gz)
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return fmt.Errorf("tar read failed: %w", err)
		}

		if filepath.Base(hdr.Name) == binaryName && hdr.Typeflag == tar.TypeReg {
			out, err := os.Create(destPath)
			if err != nil {
				return err
			}
			if _, err := io.Copy(out, tr); err != nil {
				out.Close()
				return err
			}
			return out.Close()
		}
	}

	return fmt.Errorf("%q: %w", binaryName, ErrEntryNotFound)
}

// ExtractTarGzAll extracts the entire archive into destDir, preserving
// directory structure and file modes. Refuses to extract entries that
// escape destDir (defense against path-traversal in archive entry names).
func ExtractTarGzAll(archivePath, destDir string) error {
	f, err := os.Open(archivePath)
	if err != nil {
		return err
	}
	defer f.Close()

	gz, err := gzip.NewReader(f)
	if err != nil {
		return fmt.Errorf("gzip open failed: %w", err)
	}
	defer gz.Close()

	if err := os.MkdirAll(destDir, 0o755); err != nil {
		return err
	}
	absDest, err := filepath.Abs(destDir)
	if err != nil {
		return err
	}

	tr := tar.NewReader(gz)
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return fmt.Errorf("tar read failed: %w", err)
		}

		target := filepath.Join(destDir, hdr.Name)
		absTarget, err := filepath.Abs(target)
		if err != nil {
			return err
		}
		if !strings.HasPrefix(absTarget, absDest+string(os.PathSeparator)) && absTarget != absDest {
			return fmt.Errorf("tar entry escapes dest: %s", hdr.Name)
		}

		switch hdr.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(target, os.FileMode(hdr.Mode)&0o777); err != nil {
				return err
			}
		case tar.TypeReg:
			if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
				return err
			}
			out, err := os.OpenFile(target, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, os.FileMode(hdr.Mode)&0o777)
			if err != nil {
				return err
			}
			if _, err := io.Copy(out, tr); err != nil {
				out.Close()
				return err
			}
			if err := out.Close(); err != nil {
				return err
			}
		case tar.TypeSymlink:
			os.Remove(target)
			if err := os.Symlink(hdr.Linkname, target); err != nil {
				return err
			}
		}
	}
	return nil
}

// MakeExecutable sets file permissions to 0755.
func MakeExecutable(path string) error {
	return os.Chmod(path, 0755)
}

// ExtractZip extracts a single binary from a .zip archive at archivePath,
// locating the file by basename and writing it to destPath with 0o755 mode.
// Mirrors the semantics of ExtractTarGz for .zip archives.
func ExtractZip(archivePath, destPath, binaryName string) error {
	r, err := zip.OpenReader(archivePath)
	if err != nil {
		return fmt.Errorf("open zip %s: %w", archivePath, err)
	}
	defer r.Close()

	if err := os.MkdirAll(filepath.Dir(destPath), 0o755); err != nil {
		return err
	}

	for _, f := range r.File {
		if f.FileInfo().IsDir() {
			continue
		}
		if filepath.Base(f.Name) != binaryName {
			continue
		}
		rc, err := f.Open()
		if err != nil {
			return fmt.Errorf("open %s in zip: %w", f.Name, err)
		}
		out, err := os.OpenFile(destPath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o755)
		if err != nil {
			rc.Close()
			return fmt.Errorf("create %s: %w", destPath, err)
		}
		_, copyErr := io.Copy(out, rc)
		rc.Close()
		out.Close()
		if copyErr != nil {
			return fmt.Errorf("copy %s: %w", f.Name, copyErr)
		}
		return nil
	}
	return fmt.Errorf("binary %q not found in zip %s", binaryName, archivePath)
}
