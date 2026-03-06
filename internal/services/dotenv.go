package services

import (
	"os"
	"strings"
)

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
	// Read existing content.
	existing, err := os.ReadFile(envPath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}

	// Create backup if file exists.
	if err == nil && backupPath != "" {
		if err := os.WriteFile(backupPath, existing, 0644); err != nil {
			return err
		}
	}

	// Track which keys we've already replaced.
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

	// Append keys that weren't replaced.
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
