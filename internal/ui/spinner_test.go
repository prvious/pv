package ui

import (
	"errors"
	"fmt"
	"testing"
)

func TestStep_Success(t *testing.T) {
	err := Step("test", func() (string, error) {
		return "done", nil
	})
	if err != nil {
		t.Errorf("expected nil, got %v", err)
	}
}

func TestStep_ReturnsErrAlreadyPrinted(t *testing.T) {
	err := Step("test", func() (string, error) {
		return "", fmt.Errorf("inner error")
	})
	if !errors.Is(err, ErrAlreadyPrinted) {
		t.Errorf("expected ErrAlreadyPrinted, got %v", err)
	}
}

func TestStepVerbose_Success(t *testing.T) {
	err := StepVerbose("test", func() (string, error) {
		return "done", nil
	})
	if err != nil {
		t.Errorf("expected nil, got %v", err)
	}
}

func TestStepVerbose_ReturnsErrAlreadyPrinted(t *testing.T) {
	err := StepVerbose("test", func() (string, error) {
		return "", fmt.Errorf("inner error")
	})
	if !errors.Is(err, ErrAlreadyPrinted) {
		t.Errorf("expected ErrAlreadyPrinted, got %v", err)
	}
}

func TestStepProgress_Success(t *testing.T) {
	err := StepProgress("test", func(progress func(written, total int64)) (string, error) {
		return "done", nil
	})
	if err != nil {
		t.Errorf("expected nil, got %v", err)
	}
}

func TestStepProgress_ReturnsErrAlreadyPrinted(t *testing.T) {
	err := StepProgress("test", func(progress func(written, total int64)) (string, error) {
		return "", fmt.Errorf("inner error")
	})
	if !errors.Is(err, ErrAlreadyPrinted) {
		t.Errorf("expected ErrAlreadyPrinted, got %v", err)
	}
}
