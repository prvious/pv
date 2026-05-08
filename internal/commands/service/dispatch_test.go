package service

import (
	"strings"
	"testing"
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
