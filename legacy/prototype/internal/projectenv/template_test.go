package projectenv

import (
	"strings"
	"testing"
)

func TestRender_SubstitutesVars(t *testing.T) {
	got, err := Render("host={{ .host }} port={{ .port }}", map[string]string{
		"host": "127.0.0.1",
		"port": "5432",
	})
	if err != nil {
		t.Fatalf("Render() error = %v", err)
	}
	want := "host=127.0.0.1 port=5432"
	if got != want {
		t.Errorf("Render() = %q, want %q", got, want)
	}
}

func TestRender_PassesThroughLiteralStrings(t *testing.T) {
	got, err := Render("MyApp", map[string]string{})
	if err != nil {
		t.Fatalf("Render() error = %v", err)
	}
	if got != "MyApp" {
		t.Errorf("Render() = %q, want %q", got, "MyApp")
	}
}

func TestRender_ErrorsOnUnknownVar(t *testing.T) {
	_, err := Render("{{ .nonexistent }}", map[string]string{"other": "x"})
	if err == nil {
		t.Fatal("Render() with unknown var: want error, got nil")
	}
	if !strings.Contains(err.Error(), "nonexistent") {
		t.Errorf("error should mention the missing key, got: %v", err)
	}
}

func TestRender_ErrorsOnInvalidSyntax(t *testing.T) {
	_, err := Render("{{ .unterminated", map[string]string{})
	if err == nil {
		t.Fatal("Render() with invalid syntax: want error, got nil")
	}
}
