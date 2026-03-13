package packages

import (
	"context"
	"encoding/json"
	"net/http"
	"sync/atomic"
	"testing"
	"time"

	"github.com/prvious/pv/internal/config"
)

func TestStartBackgroundUpdater_RunsImmediately(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	var callCount atomic.Int32

	// Mock composer commands since Managed[0] is MethodComposer.
	orig := runComposer
	t.Cleanup(func() { runComposer = orig })
	runComposer = func(args ...string) ([]byte, error) {
		callCount.Add(1)
		if len(args) >= 3 && args[0] == "global" && args[1] == "show" {
			out, _ := json.Marshal(map[string]any{
				"versions": []string{"v5.3.0"},
			})
			return out, nil
		}
		return []byte(""), nil
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	StartBackgroundUpdater(ctx, &http.Client{}, 1*time.Hour)

	// Give the immediate check time to complete.
	time.Sleep(500 * time.Millisecond)

	if callCount.Load() == 0 {
		t.Error("background updater did not run immediately")
	}
}

func TestStartBackgroundUpdater_StopsOnCancel(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config.EnsureDirs()

	// Mock composer commands since Managed[0] is MethodComposer.
	orig := runComposer
	t.Cleanup(func() { runComposer = orig })
	runComposer = func(args ...string) ([]byte, error) {
		if len(args) >= 3 && args[0] == "global" && args[1] == "show" {
			out, _ := json.Marshal(map[string]any{
				"versions": []string{"v5.3.0"},
			})
			return out, nil
		}
		return []byte(""), nil
	}

	ctx, cancel := context.WithCancel(context.Background())
	StartBackgroundUpdater(ctx, &http.Client{}, 50*time.Millisecond)

	time.Sleep(200 * time.Millisecond)
	cancel()

	// Should not panic or hang after cancel.
	time.Sleep(100 * time.Millisecond)
}
