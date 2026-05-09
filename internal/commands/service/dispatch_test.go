package service

import (
	"strings"
	"testing"

	"github.com/spf13/cobra"
)

func TestRedirectIfBinary_S3(t *testing.T) {
	err := redirectIfBinary("s3", "install")
	if err == nil {
		t.Fatal("expected redirect error for s3, got nil")
	}
	msg := err.Error()
	if !strings.Contains(msg, "rustfs:install") {
		t.Errorf("error should suggest rustfs:install; got %q", msg)
	}
	if !strings.Contains(msg, "s3:install") {
		t.Errorf("error should mention s3:install alias; got %q", msg)
	}
}

func TestRedirectIfBinary_Mail(t *testing.T) {
	err := redirectIfBinary("mail", "uninstall")
	if err == nil {
		t.Fatal("expected redirect error for mail, got nil")
	}
	msg := err.Error()
	if !strings.Contains(msg, "mailpit:uninstall") {
		t.Errorf("error should suggest mailpit:uninstall; got %q", msg)
	}
	if !strings.Contains(msg, "mail:uninstall") {
		t.Errorf("error should mention mail:uninstall alias; got %q", msg)
	}
}

func TestRedirectIfBinary_DockerName_ReturnsNil(t *testing.T) {
	if err := redirectIfBinary("mysql", "install"); err != nil {
		t.Errorf("docker name should not redirect: %v", err)
	}
	if err := redirectIfBinary("redis", "start"); err != nil {
		t.Errorf("docker name should not redirect: %v", err)
	}
}

func TestRedirectIfBinary_UnknownName_ReturnsNil(t *testing.T) {
	// Unknown names flow through to services.Lookup which produces the
	// real "unknown service" error. The redirect helper only handles
	// the binary-name case.
	if err := redirectIfBinary("bogus", "install"); err != nil {
		t.Errorf("unknown name should not redirect: %v", err)
	}
}

// TestServiceCommands_RedirectBinaryWiring locks every service:* RunE
// against the redirect contract. Each command must call
// redirectIfBinary before any colima/registry side-effects so that
// `pv service:add s3` (and friends, including the mail spelling) fast-
// fails with a redirect error pointing at the first-class command —
// not a confused docker-engine error.
func TestServiceCommands_RedirectBinaryWiring(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	cases := []struct {
		cmd    *cobra.Command
		name   string // command label, for failure messages
		arg    string // "s3" or "mail"
		expect string // substring that must appear in the error
	}{
		{addCmd, "service:add s3", "s3", "rustfs:install"},
		{addCmd, "service:add mail", "mail", "mailpit:install"},
		{startCmd, "service:start s3", "s3", "rustfs:start"},
		{startCmd, "service:start mail", "mail", "mailpit:start"},
		{stopCmd, "service:stop s3", "s3", "rustfs:stop"},
		{stopCmd, "service:stop mail", "mail", "mailpit:stop"},
		{logsCmd, "service:logs s3", "s3", "rustfs:logs"},
		{logsCmd, "service:logs mail", "mail", "mailpit:logs"},
		{statusCmd, "service:status s3", "s3", "rustfs:status"},
		{statusCmd, "service:status mail", "mail", "mailpit:status"},
		{removeCmd, "service:remove s3", "s3", "rustfs:uninstall"},
		{removeCmd, "service:remove mail", "mail", "mailpit:uninstall"},
		{destroyCmd, "service:destroy s3", "s3", "rustfs:uninstall"},
		{destroyCmd, "service:destroy mail", "mail", "mailpit:uninstall"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := tc.cmd.RunE(tc.cmd, []string{tc.arg})
			if err == nil {
				t.Fatalf("%s with arg %q should redirect, got nil error", tc.name, tc.arg)
			}
			if !strings.Contains(err.Error(), tc.expect) {
				t.Errorf("%s should mention %q in error; got %q", tc.name, tc.expect, err)
			}
		})
	}
}
