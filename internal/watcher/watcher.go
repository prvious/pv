package watcher

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/fsnotify/fsnotify"
)

// EventType describes the kind of pv.yml change detected.
type EventType int

const (
	// ConfigChanged indicates pv.yml was created or modified.
	ConfigChanged EventType = iota
	// ConfigDeleted indicates pv.yml was removed or renamed away.
	ConfigDeleted
)

// Event carries information about a pv.yml change in a watched project.
type Event struct {
	ProjectName string
	ProjectPath string
	Type        EventType
}

const debounceDelay = 200 * time.Millisecond
const configFile = "pv.yml"

// Watcher monitors project directories for pv.yml changes using fsnotify.
type Watcher struct {
	fsWatcher *fsnotify.Watcher
	events    chan Event
	projects  map[string]string // dir path -> project name
	mu        sync.RWMutex
	done      chan struct{}
	timers    map[string]*time.Timer
	timerMu   sync.Mutex
}

// New creates a new Watcher and starts its internal event loop.
func New() (*Watcher, error) {
	fsw, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}

	w := &Watcher{
		fsWatcher: fsw,
		events:    make(chan Event, 64),
		projects:  make(map[string]string),
		done:      make(chan struct{}),
		timers:    make(map[string]*time.Timer),
	}

	go w.loop()

	return w, nil
}

// Watch adds a project directory to the watch list.
func (w *Watcher) Watch(projectName, projectPath string) error {
	w.mu.Lock()
	w.projects[projectPath] = projectName
	w.mu.Unlock()

	return w.fsWatcher.Add(projectPath)
}

// Unwatch removes a project directory from the watch list and cancels pending timers.
func (w *Watcher) Unwatch(projectPath string) error {
	w.mu.Lock()
	delete(w.projects, projectPath)
	w.mu.Unlock()

	w.timerMu.Lock()
	if t, ok := w.timers[projectPath]; ok {
		t.Stop()
		delete(w.timers, projectPath)
	}
	w.timerMu.Unlock()

	return w.fsWatcher.Remove(projectPath)
}

// Events returns a read-only channel that emits pv.yml change events.
func (w *Watcher) Events() <-chan Event {
	return w.events
}

// Close shuts down the watcher and its internal goroutine.
func (w *Watcher) Close() error {
	close(w.done)
	return w.fsWatcher.Close()
}

// loop is the internal goroutine that processes fsnotify events.
func (w *Watcher) loop() {
	for {
		select {
		case <-w.done:
			return
		case ev, ok := <-w.fsWatcher.Events:
			if !ok {
				return
			}

			// Only react to pv.yml files.
			if filepath.Base(ev.Name) != configFile {
				continue
			}

			dir := filepath.Dir(ev.Name)

			w.mu.RLock()
			projectName, watched := w.projects[dir]
			w.mu.RUnlock()

			if !watched {
				continue
			}

			// Determine event type.
			var evType EventType
			if ev.Has(fsnotify.Remove) || ev.Has(fsnotify.Rename) {
				evType = ConfigDeleted
			} else if ev.Has(fsnotify.Write) || ev.Has(fsnotify.Create) {
				evType = ConfigChanged
			} else {
				continue
			}

			w.debounce(dir, Event{
				ProjectName: projectName,
				ProjectPath: dir,
				Type:        evType,
			})

		case err, ok := <-w.fsWatcher.Errors:
			if !ok {
				return
			}
			fmt.Fprintf(os.Stderr, "Watcher: filesystem notification error: %v\n", err)
		}
	}
}

// debounce coalesces rapid events for the same directory into a single event
// after debounceDelay. The last event type wins.
func (w *Watcher) debounce(dir string, event Event) {
	w.timerMu.Lock()
	defer w.timerMu.Unlock()

	if t, ok := w.timers[dir]; ok {
		t.Stop()
	}

	w.timers[dir] = time.AfterFunc(debounceDelay, func() {
		select {
		case <-w.done:
		case w.events <- event:
		}

		w.timerMu.Lock()
		delete(w.timers, dir)
		w.timerMu.Unlock()
	})
}
