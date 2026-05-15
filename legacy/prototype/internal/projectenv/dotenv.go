package projectenv

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

const managedMarker = "# pv-managed"

// ReadDotEnv reads a .env file into a map of key=value pairs.
func ReadDotEnv(path string) (map[string]string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	result := make(map[string]string)
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			result[parts[0]] = parts[1]
		}
	}
	return result, nil
}

// MergeDotEnv reads an existing .env file, replaces matching keys in-place,
// appends new keys, and writes the result. Creates a backup at backupPath
// only if one does not already exist. Preserves the original file's
// permission bits and writes atomically via temp-file + rename.
func MergeDotEnv(envPath, backupPath string, newVars map[string]string) error {
	return mergeDotEnv(envPath, backupPath, newVars, false)
}

// MergeManagedDotEnv reads an existing .env file, replaces matching keys
// in-place, appends new keys, and labels every key it writes with a
// preceding # pv-managed marker. Existing keys that are not present in newVars
// are left untouched, including any old markers they already have.
// Creates a backup at backupPath only if one does not already exist.
// Preserves the original file's permission bits and writes atomically.
func MergeManagedDotEnv(envPath, backupPath string, newVars map[string]string) error {
	return mergeDotEnv(envPath, backupPath, newVars, true)
}

func mergeDotEnv(envPath, backupPath string, newVars map[string]string, managed bool) error {
	// Validate keys and values.
	for key, val := range newVars {
		if key == "" || strings.ContainsAny(key, "\n\r=") || strings.ContainsAny(val, "\n\r") {
			return fmt.Errorf("invalid env key or value: %q", key)
		}
	}

	existing, err := os.ReadFile(envPath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}

	// Preserve original file permissions.
	var mode os.FileMode = 0o644
	if err == nil {
		if info, statErr := os.Stat(envPath); statErr == nil {
			mode = info.Mode().Perm()
		}
	}

	// Create backup only if one does not already exist.
	if err == nil && backupPath != "" {
		if _, statErr := os.Stat(backupPath); os.IsNotExist(statErr) {
			if writeErr := os.WriteFile(backupPath, existing, mode); writeErr != nil {
				return writeErr
			}
		}
	}

	replaced := make(map[string]bool)
	var lines []string

	if len(existing) > 0 {
		raw := strings.TrimSuffix(string(existing), "\n")
		for _, line := range strings.Split(raw, "\n") {
			trimmed := strings.TrimSpace(line)
			if trimmed != "" && !strings.HasPrefix(trimmed, "#") {
				parts := strings.SplitN(trimmed, "=", 2)
				if len(parts) == 2 {
					key := parts[0]
					if val, ok := newVars[key]; ok {
						if replaced[key] {
							// Already updated the first occurrence; preserve duplicates as-is.
							lines = append(lines, line)
							continue
						}
						if managed && !hasManagedMarker(lines) {
							lines = append(lines, managedMarker)
						}
						lines = append(lines, key+"="+val)
						replaced[key] = true
						continue
					}
				}
			}
			lines = append(lines, line)
		}
	}

	// Sort keys for deterministic output.
	keys := make([]string, 0, len(newVars))
	for k := range newVars {
		if !replaced[k] {
			keys = append(keys, k)
		}
	}
	sort.Strings(keys)

	for _, key := range keys {
		val := newVars[key]
		if managed {
			lines = append(lines, managedMarker)
		}
		lines = append(lines, key+"="+val)
	}

	content := strings.Join(lines, "\n")
	if !strings.HasSuffix(content, "\n") {
		content += "\n"
	}

	// Atomic write: temp file in the same directory, then rename.
	dir := filepath.Dir(envPath)
	tmp, err := os.CreateTemp(dir, ".env.tmp-*")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()

	if _, err := tmp.WriteString(content); err != nil {
		tmp.Close()
		os.Remove(tmpPath)
		return err
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmpPath)
		return err
	}
	if err := os.Chmod(tmpPath, mode); err != nil {
		os.Remove(tmpPath)
		return err
	}
	return os.Rename(tmpPath, envPath)
}

// hasManagedMarker reports whether the last non-empty line in lines is
// the pv-managed marker. It skips blank lines when looking backward.
func hasManagedMarker(lines []string) bool {
	for i := len(lines) - 1; i >= 0; i-- {
		trimmed := strings.TrimSpace(lines[i])
		if trimmed == "" {
			continue
		}
		return trimmed == managedMarker
	}
	return false
}
