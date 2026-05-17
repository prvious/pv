package status

import (
	"fmt"
	"sort"
	"strings"
)

type State string

const (
	StateHealthy        State = "healthy"
	StateStopped        State = "stopped"
	StateMissingInstall State = "missing_install"
	StateBlocked        State = "blocked"
	StateCrashed        State = "crashed"
	StateFailed         State = "failed"
	StatePartial        State = "partial"
	StateUnknown        State = "unknown"
)

type View string

const (
	ViewProject  View = "project"
	ViewRuntime  View = "runtime"
	ViewResource View = "resource"
	ViewGateway  View = "gateway"
)

type Entry struct {
	View          View
	Name          string
	Desired       string
	Observed      string
	State         State
	LogPath       string
	LastReconcile string
	LastError     string
	NextAction    string
	Values        map[string]string
}

func Render(entries []Entry, view View) (string, error) {
	if view != "" {
		if err := ValidateView(view); err != nil {
			return "", err
		}
	}
	filtered := make([]Entry, 0, len(entries))
	for _, entry := range entries {
		if view == "" || entry.View == view {
			entry.Values = Redact(entry.Values)
			filtered = append(filtered, entry)
		}
	}
	sort.Slice(filtered, func(i, j int) bool {
		if filtered[i].View == filtered[j].View {
			return filtered[i].Name < filtered[j].Name
		}
		return filtered[i].View < filtered[j].View
	})
	var b strings.Builder
	for _, entry := range filtered {
		fmt.Fprintf(&b, "%s %s: %s\n", entry.View, entry.Name, entry.State)
		if entry.Desired != "" {
			fmt.Fprintf(&b, "desired: %s\n", entry.Desired)
		}
		if entry.Observed != "" {
			fmt.Fprintf(&b, "observed: %s\n", entry.Observed)
		}
		if entry.LastReconcile != "" {
			fmt.Fprintf(&b, "last reconcile: %s\n", entry.LastReconcile)
		}
		if entry.LogPath != "" {
			fmt.Fprintf(&b, "log: %s\n", entry.LogPath)
		}
		if entry.LastError != "" {
			fmt.Fprintf(&b, "last error: %s\n", entry.LastError)
		}
		if entry.NextAction != "" {
			fmt.Fprintf(&b, "next action: %s\n", entry.NextAction)
		}
		for _, key := range sortedKeys(entry.Values) {
			fmt.Fprintf(&b, "%s=%s\n", key, entry.Values[key])
		}
	}
	if b.Len() == 0 {
		return "status: none\n", nil
	}
	return b.String(), nil
}

func ValidateView(view View) error {
	switch view {
	case ViewProject, ViewRuntime, ViewResource, ViewGateway:
		return nil
	default:
		return fmt.Errorf("unknown status view %q", view)
	}
}

func Redact(values map[string]string) map[string]string {
	if values == nil {
		return nil
	}
	redacted := make(map[string]string, len(values))
	for key, value := range values {
		if isSecretKey(key) {
			redacted[key] = "<redacted>"
			continue
		}
		redacted[key] = value
	}
	return redacted
}

func isSecretKey(key string) bool {
	upper := strings.ToUpper(key)
	for _, marker := range []string{"SECRET", "PASSWORD", "TOKEN", "ACCESS_KEY"} {
		if strings.Contains(upper, marker) {
			return true
		}
	}
	return false
}

func sortedKeys(values map[string]string) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}
