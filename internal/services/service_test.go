package services

import (
	"testing"
)

func TestLookup_Invalid(t *testing.T) {
	_, err := Lookup("mongodb")
	if err == nil {
		t.Error("expected error for unknown service, got nil")
	}
}

func TestLookup_BinaryService(t *testing.T) {
	svc, err := Lookup("mail")
	if err != nil {
		t.Fatalf("Lookup(\"mail\") error = %v", err)
	}
	if svc == nil {
		t.Error("Lookup(\"mail\") returned nil service")
	}
}

func TestAvailable(t *testing.T) {
	names := Available()
	// 2 binary services: s3, mail.
	if len(names) != 2 {
		t.Errorf("Available() returned %d services, want 2", len(names))
	}
}
