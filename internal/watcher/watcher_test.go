package watcher

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestWatcher_DetectsFileChange(t *testing.T) {
	dir := t.TempDir()

	w, err := New()
	if err != nil {
		t.Fatalf("New() error = %v", err)
	}
	defer w.Close()

	if err := w.Watch("myproject", dir); err != nil {
		t.Fatalf("Watch() error = %v", err)
	}

	// Let the watcher settle.
	time.Sleep(50 * time.Millisecond)

	// Write pv.yml.
	if err := os.WriteFile(filepath.Join(dir, "pv.yml"), []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	select {
	case ev := <-w.Events():
		if ev.Type != ConfigChanged {
			t.Errorf("expected ConfigChanged, got %d", ev.Type)
		}
		if ev.ProjectName != "myproject" {
			t.Errorf("expected project name 'myproject', got %q", ev.ProjectName)
		}
		if ev.ProjectPath != dir {
			t.Errorf("expected project path %q, got %q", dir, ev.ProjectPath)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for ConfigChanged event")
	}
}

func TestWatcher_DetectsFileDelete(t *testing.T) {
	dir := t.TempDir()

	// Create the file first so we can delete it.
	pvYml := filepath.Join(dir, "pv.yml")
	if err := os.WriteFile(pvYml, []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	w, err := New()
	if err != nil {
		t.Fatalf("New() error = %v", err)
	}
	defer w.Close()

	if err := w.Watch("delproject", dir); err != nil {
		t.Fatalf("Watch() error = %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	// Remove pv.yml.
	if err := os.Remove(pvYml); err != nil {
		t.Fatalf("Remove error = %v", err)
	}

	select {
	case ev := <-w.Events():
		if ev.Type != ConfigDeleted {
			t.Errorf("expected ConfigDeleted, got %d", ev.Type)
		}
		if ev.ProjectName != "delproject" {
			t.Errorf("expected project name 'delproject', got %q", ev.ProjectName)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for ConfigDeleted event")
	}
}

func TestWatcher_Debounce(t *testing.T) {
	dir := t.TempDir()

	w, err := New()
	if err != nil {
		t.Fatalf("New() error = %v", err)
	}
	defer w.Close()

	if err := w.Watch("debounce-proj", dir); err != nil {
		t.Fatalf("Watch() error = %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	// Perform 5 rapid writes.
	pvYml := filepath.Join(dir, "pv.yml")
	for i := 0; i < 5; i++ {
		if err := os.WriteFile(pvYml, []byte("php: \"8.4\"\n"), 0644); err != nil {
			t.Fatalf("WriteFile error = %v", err)
		}
		time.Sleep(10 * time.Millisecond)
	}

	// We should get exactly 1 event (debounced).
	select {
	case <-w.Events():
		// Got the expected single event.
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for debounced event")
	}

	// Wait a bit and confirm no more events arrive.
	select {
	case ev := <-w.Events():
		t.Errorf("expected no more events after debounce, got %+v", ev)
	case <-time.After(500 * time.Millisecond):
		// Good, no extra events.
	}
}

func TestWatcher_Unwatch(t *testing.T) {
	dir := t.TempDir()

	w, err := New()
	if err != nil {
		t.Fatalf("New() error = %v", err)
	}
	defer w.Close()

	if err := w.Watch("unwatch-proj", dir); err != nil {
		t.Fatalf("Watch() error = %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	// Unwatch the directory.
	if err := w.Unwatch(dir); err != nil {
		t.Fatalf("Unwatch() error = %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	// Write to pv.yml after unwatching.
	if err := os.WriteFile(filepath.Join(dir, "pv.yml"), []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	// Expect no event.
	select {
	case ev := <-w.Events():
		t.Errorf("expected no event after unwatch, got %+v", ev)
	case <-time.After(500 * time.Millisecond):
		// Good, no event received.
	}
}

func TestWatcher_IgnoresNonPvYml(t *testing.T) {
	dir := t.TempDir()

	w, err := New()
	if err != nil {
		t.Fatalf("New() error = %v", err)
	}
	defer w.Close()

	if err := w.Watch("ignore-proj", dir); err != nil {
		t.Fatalf("Watch() error = %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	// Write to a non-pv.yml file.
	if err := os.WriteFile(filepath.Join(dir, "other.txt"), []byte("hello\n"), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	// Expect no event.
	select {
	case ev := <-w.Events():
		t.Errorf("expected no event for other.txt, got %+v", ev)
	case <-time.After(500 * time.Millisecond):
		// Good, no event received.
	}
}
