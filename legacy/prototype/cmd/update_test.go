package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

func newUpdateCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	update := &cobra.Command{
		Use:  "update",
		RunE: updateCmd.RunE,
	}
	root.AddCommand(update)
	return root
}

func TestUpdateCmd_Structure(t *testing.T) {
	root := newUpdateCmd()
	cmd, _, err := root.Find([]string{"update"})
	if err != nil {
		t.Fatalf("Find() error = %v", err)
	}
	if cmd.Use != "update" {
		t.Errorf("Use = %q, want %q", cmd.Use, "update")
	}
	if cmd.RunE == nil {
		t.Error("RunE is nil")
	}
}

func TestRedisVersionsForUpdateUsesInstalledVersions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	for _, version := range []string{"8.6", "8.7"} {
		versionDir := config.RedisVersionDir(version)
		if err := os.MkdirAll(versionDir, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte{}, 0o755); err != nil {
			t.Fatal(err)
		}
	}

	versions, err := redisVersionsForUpdate()
	if err != nil {
		t.Fatal(err)
	}
	if len(versions) != 1 || versions[0] != "8.6" {
		t.Fatalf("versions = %v, want [8.6]", versions)
	}
}
