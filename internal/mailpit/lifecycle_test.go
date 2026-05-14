package mailpit

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

	binPath := filepath.Join(config.MailpitBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	oldBinary := []byte("#!/bin/sh\necho old mailpit\n")
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write existing binary: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeMailpitArchive(t, w, map[string]string{
			"VERSION": "bad-artifact\n",
		})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", srv.URL)

	err := Update(srv.Client(), DefaultVersion())
	if err == nil {
		t.Fatal("expected update to fail for archive without bin/mailpit")
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

	binPath := filepath.Join(config.MailpitBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	oldBinary := []byte("#!/bin/sh\necho old mailpit\n")
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write existing binary: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeMailpitArchiveEntries(t, w, []mailpitArchiveEntry{
			{name: "bin/mailpit", mode: 0o755, typeflag: tar.TypeDir},
		})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", srv.URL)

	err := Update(srv.Client(), DefaultVersion())
	if err == nil {
		t.Fatal("expected update to fail when bin/mailpit is a directory")
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

	binPath := filepath.Join(config.MailpitBinDir(DefaultVersion()), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	oldBinary := []byte("#!/bin/sh\necho old mailpit\n")
	if err := os.WriteFile(binPath, oldBinary, 0o755); err != nil {
		t.Fatalf("write existing binary: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeMailpitArchiveEntries(t, w, []mailpitArchiveEntry{
			{name: "bin/real-mailpit", body: "#!/bin/sh\necho fake\n", mode: 0o755, typeflag: tar.TypeReg},
			{name: "bin/mailpit", mode: 0o755, typeflag: tar.TypeSymlink, linkname: "real-mailpit"},
		})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", srv.URL)

	err := Update(srv.Client(), DefaultVersion())
	if err == nil {
		t.Fatal("expected update to fail when bin/mailpit is a symlink")
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

	versionDir := config.MailpitVersionDir(DefaultVersion())
	binPath := filepath.Join(versionDir, "bin", Binary().Name)
	oldBinary := []byte("#!/bin/sh\necho old mailpit\n")
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

// TestUninstall_BinaryAlreadyRemoved verifies that an idempotent retry
// after a previous run that left the registry intact but removed the
// binary file completes successfully.
func TestUninstall_BinaryAlreadyRemoved(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(DefaultVersion(), false); err != nil {
		t.Fatalf("Uninstall with no binary file should succeed: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Versions[DefaultVersion()]; ok {
		t.Error("version state should be removed after uninstall")
	}
}

// TestUninstall_DeleteData verifies that --force/data-deletion actually
// wipes the data directory. This is the irreversible postgres-style
// :uninstall semantic; a regression here would silently spare user data
// the user explicitly asked to be deleted.
func TestUninstall_DeleteData(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.MailpitDataDir(DefaultVersion())
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "mailpit.db")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(DefaultVersion(), true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); !os.IsNotExist(err) {
		t.Errorf("data directory must be deleted; sentinel still exists (err=%v)", err)
	}
}

func TestApplyFallbacksToLinkedProjects_RewritesEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	if err := os.WriteFile(envPath, []byte("MAIL_MAILER=smtp\n"), 0o644); err != nil {
		t.Fatalf("write .env: %v", err)
	}

	reg := &registry.Registry{
		Projects: []registry.Project{
			{
				Name:     "myapp",
				Path:     projectDir,
				Type:     "laravel",
				Services: &registry.ProjectServices{Mail: DefaultVersion()},
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
	if got := env["MAIL_MAILER"]; got != "log" {
		t.Errorf("MAIL_MAILER = %q, want %q", got, "log")
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
		t.Fatalf("WantedVersions should ignore not-installed mailpit, got %v", versions)
	}

	binPath := filepath.Join(config.MailpitBinDir(DefaultVersion()), Binary().Name)
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
		t.Fatal("expected latest mailpit version to fail")
	}
}

func TestUninstall_KeepsDataDirByDefault(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.MailpitDataDir(DefaultVersion())
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "mailpit.db")
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
	_, err := EnvVars("bad-version", "anyproject")
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

func writeMailpitArchive(t *testing.T, w http.ResponseWriter, files map[string]string) {
	t.Helper()

	entries := make([]mailpitArchiveEntry, 0, len(files))
	for name, body := range files {
		entries = append(entries, mailpitArchiveEntry{name: name, body: body, mode: 0o644, typeflag: tar.TypeReg})
	}
	writeMailpitArchiveEntries(t, w, entries)
}

type mailpitArchiveEntry struct {
	name     string
	body     string
	linkname string
	mode     int64
	typeflag byte
}

func writeMailpitArchiveEntries(t *testing.T, w http.ResponseWriter, entries []mailpitArchiveEntry) {
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
