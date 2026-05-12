package projectenv

import (
	"os"
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
// appends new keys, and writes the result. Creates a backup at backupPath.
func MergeDotEnv(envPath, backupPath string, newVars map[string]string) error {
	existing, err := os.ReadFile(envPath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}

	if err == nil && backupPath != "" {
		if err := os.WriteFile(backupPath, existing, 0644); err != nil {
			return err
		}
	}

	replaced := make(map[string]bool)
	var lines []string

	if len(existing) > 0 {
		for _, line := range strings.Split(string(existing), "\n") {
			trimmed := strings.TrimSpace(line)
			if trimmed != "" && !strings.HasPrefix(trimmed, "#") {
				parts := strings.SplitN(trimmed, "=", 2)
				if len(parts) == 2 {
					key := parts[0]
					if val, ok := newVars[key]; ok {
						lines = append(lines, key+"="+val)
						replaced[key] = true
						continue
					}
				}
			}
			lines = append(lines, line)
		}
	}

	for key, val := range newVars {
		if !replaced[key] {
			lines = append(lines, key+"="+val)
		}
	}

	content := strings.Join(lines, "\n")
	if !strings.HasSuffix(content, "\n") {
		content += "\n"
	}

	return os.WriteFile(envPath, []byte(content), 0644)
}

// MergeManagedDotEnv reads an existing .env file, replaces matching keys
// in-place, appends new keys, and labels every key it writes with a
// preceding # pv-managed marker. Existing keys that are not present in newVars
// are left untouched, including any old markers they already have.
func MergeManagedDotEnv(envPath, backupPath string, newVars map[string]string) error {
	existing, err := os.ReadFile(envPath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}

	if err == nil && backupPath != "" {
		if err := os.WriteFile(backupPath, existing, 0644); err != nil {
			return err
		}
	}

	replaced := make(map[string]bool)
	var lines []string

	if len(existing) > 0 {
		lines = strings.Split(string(existing), "\n")
		if len(lines) > 0 && lines[len(lines)-1] == "" {
			lines = lines[:len(lines)-1]
		}

		merged := make([]string, 0, len(lines)+len(newVars)*2)
		for _, line := range lines {
			trimmed := strings.TrimSpace(line)
			if trimmed != "" && !strings.HasPrefix(trimmed, "#") {
				parts := strings.SplitN(trimmed, "=", 2)
				if len(parts) == 2 {
					key := parts[0]
					if val, ok := newVars[key]; ok {
						if len(merged) == 0 || strings.TrimSpace(merged[len(merged)-1]) != managedMarker {
							merged = append(merged, managedMarker)
						}
						merged = append(merged, key+"="+val)
						replaced[key] = true
						continue
					}
				}
			}
			merged = append(merged, line)
		}
		lines = merged
	}

	for key, val := range newVars {
		if !replaced[key] {
			lines = append(lines, managedMarker, key+"="+val)
		}
	}

	content := strings.Join(lines, "\n")
	if !strings.HasSuffix(content, "\n") {
		content += "\n"
	}

	return os.WriteFile(envPath, []byte(content), 0644)
}
