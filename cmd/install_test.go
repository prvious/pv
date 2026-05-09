package cmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/spf13/cobra"
)

func newInstallCmd() *cobra.Command {
	var force bool
	var tld string
	var with string

	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	install := &cobra.Command{
		Use: "install",
		RunE: func(cmd *cobra.Command, args []string) error {
			forceInstall = force
			installTLD = tld
			installWith = with
			return installCmd.RunE(cmd, args)
		},
	}
	install.Flags().BoolVar(&force, "force", false, "Reinstall")
	install.Flags().StringVar(&tld, "tld", "test", "TLD")
	install.Flags().StringVar(&with, "with", "", "Optional tools and services")
	root.AddCommand(install)
	return root
}

func TestInstallCmd_Structure(t *testing.T) {
	root := newInstallCmd()
	cmd, _, err := root.Find([]string{"install"})
	if err != nil {
		t.Fatalf("Find() error = %v", err)
	}
	if cmd.Use != "install" {
		t.Errorf("Use = %q, want %q", cmd.Use, "install")
	}
	if cmd.RunE == nil {
		t.Error("RunE is nil")
	}
}

func TestInstallCmd_HasForceFlag(t *testing.T) {
	root := newInstallCmd()
	cmd, _, _ := root.Find([]string{"install"})
	flag := cmd.Flags().Lookup("force")
	if flag == nil {
		t.Error("--force flag not found")
	}
}

func TestInstallCmd_HasTLDFlag(t *testing.T) {
	root := newInstallCmd()
	cmd, _, _ := root.Find([]string{"install"})
	flag := cmd.Flags().Lookup("tld")
	if flag == nil {
		t.Error("--tld flag not found")
	}
}

func TestInstallCmd_HasWithFlag(t *testing.T) {
	root := newInstallCmd()
	cmd, _, _ := root.Find([]string{"install"})
	flag := cmd.Flags().Lookup("with")
	if flag == nil {
		t.Error("--with flag not found")
	}
}

func TestParseWith_Empty(t *testing.T) {
	spec, err := parseWith("")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if spec.phpVersion != "" || spec.mago || len(spec.services) != 0 {
		t.Errorf("expected empty spec, got %+v", spec)
	}
}

func TestParseWith_PHPVersion(t *testing.T) {
	spec, err := parseWith("php:8.2")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if spec.phpVersion != "8.2" {
		t.Errorf("phpVersion = %q, want %q", spec.phpVersion, "8.2")
	}
}

func TestParseWith_Mago(t *testing.T) {
	spec, err := parseWith("mago")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !spec.mago {
		t.Error("mago should be true")
	}
}

func TestParseWith_Services(t *testing.T) {
	// After redis migrated to a native binary (pv redis:install), the docker
	// registry is empty. mail is the remaining binary service that still parses
	// through `service[...]`. Keep one `:version` form to cover the parser.
	spec, err := parseWith("service[mail:1.20],service[s3]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(spec.services) != 2 {
		t.Fatalf("expected 2 services, got %d", len(spec.services))
	}
	if spec.services[0].name != "mail" || spec.services[0].version != "1.20" {
		t.Errorf("service[0] = %+v, want mail:1.20", spec.services[0])
	}
	if spec.services[1].name != "s3" || spec.services[1].version != "" {
		t.Errorf("service[1] = %+v, want s3", spec.services[1])
	}
}

func TestParseWith_Full(t *testing.T) {
	spec, err := parseWith("php:8.3,mago,service[mail],service[s3]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if spec.phpVersion != "8.3" {
		t.Errorf("phpVersion = %q, want %q", spec.phpVersion, "8.3")
	}
	if !spec.mago {
		t.Error("mago should be true")
	}
	if len(spec.services) != 2 {
		t.Fatalf("expected 2 services, got %d", len(spec.services))
	}
}

func TestParseWith_UnknownTool(t *testing.T) {
	_, err := parseWith("unknown")
	if err == nil {
		t.Error("expected error for unknown tool")
	}
}

func TestParseWith_UnknownService(t *testing.T) {
	_, err := parseWith("service[fakesvc:1]")
	if err == nil {
		t.Error("expected error for unknown service")
	}
}

func TestParseWith_ServiceNoVersion(t *testing.T) {
	spec, err := parseWith("service[mail]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(spec.services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(spec.services))
	}
	if spec.services[0].name != "mail" || spec.services[0].version != "" {
		t.Errorf("service = %+v, want mail with empty version", spec.services[0])
	}
}

func TestParseWith_BinaryServiceS3(t *testing.T) {
	spec, err := parseWith("service[s3]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(spec.services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(spec.services))
	}
	if spec.services[0].name != "s3" {
		t.Errorf("service[0].name = %q, want s3", spec.services[0].name)
	}
}

func TestParseWith_BinaryServiceMail(t *testing.T) {
	spec, err := parseWith("service[mail]")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(spec.services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(spec.services))
	}
	if spec.services[0].name != "mail" {
		t.Errorf("service[0].name = %q, want mail", spec.services[0].name)
	}
}

func TestParseWith_UnknownServiceMongodb(t *testing.T) {
	_, err := parseWith("service[mongodb]")
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
}

func TestInstallCmd_AlreadyInstalled(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Create ~/.pv to simulate existing install.
	if err := os.MkdirAll(filepath.Join(home, ".pv"), 0755); err != nil {
		t.Fatal(err)
	}

	root := newInstallCmd()
	root.SetArgs([]string{"install"})
	err := root.Execute()
	if err == nil {
		t.Fatal("expected error for existing install without --force, got nil")
	}
}
