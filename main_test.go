package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestRunPrintsReturnedErrorsBeforeExit(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	code := run([]string{"status", "daemon"}, &stdout, &stderr)

	if code != 1 {
		t.Fatalf("run exit code = %d, want 1", code)
	}
	if stdout.Len() != 0 {
		t.Fatalf("run wrote stdout: %q", stdout.String())
	}
	if got := stderr.String(); !strings.Contains(got, `unknown status view "daemon"`) {
		t.Fatalf("stderr = %q, want returned error", got)
	}
}
