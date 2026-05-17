package installer

import (
	"context"
	"errors"
	"testing"
)

func TestPersistThenSignalRunsSignalAfterSuccessfulPersist(t *testing.T) {
	var order []string
	err := PersistThenSignal(context.Background(), func(context.Context) error {
		order = append(order, "persist")
		return nil
	}, func(context.Context) error {
		order = append(order, "signal")
		return nil
	})

	if err != nil {
		t.Fatalf("PersistThenSignal returned error: %v", err)
	}
	if got := order; len(got) != 2 || got[0] != "persist" || got[1] != "signal" {
		t.Fatalf("order = %v, want [persist signal]", got)
	}
}

func TestPersistThenSignalDoesNotSignalAfterPersistFailure(t *testing.T) {
	cause := errors.New("disk full")
	signaled := false
	err := PersistThenSignal(context.Background(), func(context.Context) error {
		return cause
	}, func(context.Context) error {
		signaled = true
		return nil
	})

	if !errors.Is(err, cause) {
		t.Fatalf("error = %v, want %v", err, cause)
	}
	if signaled {
		t.Fatal("signal ran after failed persist")
	}
}
