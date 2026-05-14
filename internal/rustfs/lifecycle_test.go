package rustfs

import (
	"archive/tar"
	"compress/gzip"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
)

func TestUpdate_NotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	client := &http.Client{}
	err := Update(client, DefaultVersion())
	if err == nil {
		t.Fatal("expected error when service is not installed")
	}
	if !strings.Contains(err.Error(), "not installed") {
		t.Errorf("expected not-installed error; got %q", err)
	}
}

func TestUpdate_MalformedArchivePreservesExistingBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	binPath := filepath.Join(config.RustfsBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	oldBinary := []byte("#!/bin/sh\necho old rustfs\n")
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write existing binary: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeRustfsArchive(t, w, map[string]string{
			"VERSION": "bad-artifact\n",
		})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", srv.URL)

	err := Update(srv.Client(), DefaultVersion())
	if err == nil {
		t.Fatal("expected update to fail for archive without bin/rustfs")
	}
	got, readErr := os.ReadFile(binPath)
	if readErr != nil {
		t.Fatalf("existing binary should remain readable: %v", readErr)
	}
	if string(got) != string(oldBinary) {
		t.Fatalf("existing binary was replaced; got %q, want %q", got, oldBinary)
	}
}

func TestUpdate_DirectoryBinaryArchivePreservesExistingBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	binPath := filepath.Join(config.RustfsBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	oldBinary := []byte("#!/bin/sh\necho old rustfs\n")
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write existing binary: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeRustfsArchiveEntries(t, w, []rustfsArchiveEntry{
			{name: "bin/rustfs", mode: 0o755, typeflag: tar.TypeDir},
		})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", srv.URL)

	err := Update(srv.Client(), DefaultVersion())
	if err == nil {
		t.Fatal("expected update to fail when bin/rustfs is a directory")
	}
	got, readErr := os.ReadFile(binPath)
	if readErr != nil {
		t.Fatalf("existing binary should remain readable: %v", readErr)
	}
	if string(got) != string(oldBinary) {
		t.Fatalf("existing binary was replaced; got %q, want %q", got, oldBinary)
	}
}

func TestUpdate_SymlinkBinaryArchivePreservesExistingBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	binPath := filepath.Join(config.RustfsBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	oldBinary := []byte("#!/bin/sh\necho old rustfs\n")
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write existing binary: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeRustfsArchiveEntries(t, w, []rustfsArchiveEntry{
			{name: "bin/real-rustfs", body: "#!/bin/sh\necho fake\n", mode: 0o755, typeflag: tar.TypeReg},
			{name: "bin/rustfs", mode: 0o755, typeflag: tar.TypeSymlink, linkname: "real-rustfs"},
		})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", srv.URL)

	err := Update(srv.Client(), DefaultVersion())
	if err == nil {
		t.Fatal("expected update to fail when bin/rustfs is a symlink")
	}
	got, readErr := os.ReadFile(binPath)
	if readErr != nil {
		t.Fatalf("existing binary should remain readable: %v", readErr)
	}
	if string(got) != string(oldBinary) {
		t.Fatalf("existing binary was replaced; got %q, want %q", got, oldBinary)
	}
}

func TestSwapVersionDir_RestoresOldInstallWhenStagingRenameFails(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	versionDir := config.RustfsVersionDir(DefaultVersion())
	binPath := filepath.Join(versionDir, "bin", Binary().Name)
	oldBinary := []byte("#!/bin/sh\necho old rustfs\n")
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir old bin dir: %v", err)
	}
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write old binary: %v", err)
	}

	missingStagingDir := versionDir + ".new"
	if err := swapVersionDir(versionDir, missingStagingDir); err == nil {
		t.Fatal("expected swap to fail when staging dir is missing")
	}

	got, err := os.ReadFile(binPath)
	if err != nil {
		t.Fatalf("old binary should be restored: %v", err)
	}
	if string(got) != string(oldBinary) {
		t.Fatalf("old binary content = %q, want %q", got, oldBinary)
	}
	if _, err := os.Stat(versionDir + ".old"); !os.IsNotExist(err) {
		t.Fatalf("old backup should not remain after restore; err=%v", err)
	}
}

// TestUninstall_DeleteData verifies that --force/data-deletion actually
// wipes the data directory. This is the irreversible postgres-style
// :uninstall semantic; a regression here would silently spare user data
// the user explicitly asked to be deleted.
func TestUninstall_DeleteData(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.RustfsDataDir(DefaultVersion())
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "buckets.json")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	if err := Uninstall(DefaultVersion(), true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); !os.IsNotExist(err) {
		t.Errorf("data directory must be deleted; sentinel still exists (err=%v)", err)
	}
}

func TestState_SetWantedWantedVersionsRemove(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted(DefaultVersion(), WantedRunning); err != nil {
		t.Fatalf("SetWanted running: %v", err)
	}
	versions, err := WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions: %v", err)
	}
	if len(versions) != 0 {
		t.Fatalf("WantedVersions should ignore not-installed rustfs, got %v", versions)
	}

	binPath := filepath.Join(config.RustfsBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	if err := os.WriteFile(binPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatalf("write fake binary: %v", err)
	}

	versions, err = WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions installed: %v", err)
	}
	if len(versions) != 1 || versions[0] != DefaultVersion() {
		t.Fatalf("WantedVersions = %v, want [%s]", versions, DefaultVersion())
	}

	if err := RemoveVersion(DefaultVersion()); err != nil {
		t.Fatalf("RemoveVersion: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Versions[DefaultVersion()]; ok {
		t.Fatalf("state still contains %s after RemoveVersion: %#v", DefaultVersion(), st.Versions)
	}
}

func TestValidateVersion_RejectsLatest(t *testing.T) {
	if err := ValidateVersion("latest"); err == nil {
		t.Fatal("expected latest rustfs version to fail")
	}
}

func TestUninstall_KeepsDataDirByDefault(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.RustfsDataDir(DefaultVersion())
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "buckets.json")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	if err := Uninstall(DefaultVersion(), false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); err != nil {
		t.Errorf("data directory must be preserved without --force; sentinel missing: %v", err)
	}
}

func TestEnvVars_RejectsInvalidVersion(t *testing.T) {
	_, err := EnvVars("bad-version", "myproject")
	if err == nil {
		t.Fatal("EnvVars: expected error for invalid version")
	}
}

func TestBuildSupervisorProcess_RejectsInvalidVersion(t *testing.T) {
	_, err := BuildSupervisorProcess("bad-version")
	if err == nil {
		t.Fatal("BuildSupervisorProcess: expected error for invalid version")
	}
}

func TestApplyFallbacksToLinkedProjects_RewritesEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	if err := os.WriteFile(envPath, []byte("FILESYSTEM_DISK=s3\n"), 0o644); err != nil {
		t.Fatalf("write .env: %v", err)
	}

	reg := &registry.Registry{
		Projects: []registry.Project{
			{
				Name:     "myapp",
				Path:     projectDir,
				Type:     "laravel",
				Services: &registry.ProjectServices{S3: DefaultVersion()},
			},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save registry: %v", err)
	}

	settings := config.DefaultSettings()
	settings.Automation.ServiceFallback = config.AutoOn
	if err := settings.Save(); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	ApplyFallbacksToLinkedProjects(reg)

	env, err := projectenv.ReadDotEnv(envPath)
	if err != nil {
		t.Fatalf("read .env after fallback: %v", err)
	}
	if got := env["FILESYSTEM_DISK"]; got != "local" {
		t.Errorf("FILESYSTEM_DISK = %q, want %q", got, "local")
	}
}

func writeRustfsArchive(t *testing.T, w http.ResponseWriter, files map[string]string) {
	t.Helper()

	entries := make([]rustfsArchiveEntry, 0, len(files))
	for name, body := range files {
		entries = append(entries, rustfsArchiveEntry{name: name, body: body, mode: 0o644, typeflag: tar.TypeReg})
	}
	writeRustfsArchiveEntries(t, w, entries)
}

type rustfsArchiveEntry struct {
	name     string
	body     string
	linkname string
	mode     int64
	typeflag byte
}

func writeRustfsArchiveEntries(t *testing.T, w http.ResponseWriter, entries []rustfsArchiveEntry) {
	t.Helper()

	gz := gzip.NewWriter(w)
	tw := tar.NewWriter(gz)

	for _, entry := range entries {
		data := []byte(entry.body)
		hdr := &tar.Header{Name: entry.name, Linkname: entry.linkname, Mode: entry.mode, Typeflag: entry.typeflag}
		if entry.typeflag == tar.TypeReg {
			hdr.Size = int64(len(data))
		}
		if err := tw.WriteHeader(hdr); err != nil {
			t.Fatalf("write tar header: %v", err)
		}
		if len(data) > 0 {
			if _, err := tw.Write(data); err != nil {
				t.Fatalf("write tar body: %v", err)
			}
		}
	}
	if err := tw.Close(); err != nil {
		t.Fatalf("close tar writer: %v", err)
	}
	if err := gz.Close(); err != nil {
		t.Fatalf("close gzip writer: %v", err)
	}
}
