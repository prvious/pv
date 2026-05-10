package projectenv

import (
	"regexp"
	"strings"
)

var safeIdentifier = regexp.MustCompile(`[^a-zA-Z0-9_]`)

// SanitizeProjectName converts a directory name to a database-safe identifier.
// Only alphanumeric characters and underscores are kept; everything else is stripped.
func SanitizeProjectName(name string) string {
	name = strings.ReplaceAll(name, "-", "_")
	return safeIdentifier.ReplaceAllString(name, "")
}
