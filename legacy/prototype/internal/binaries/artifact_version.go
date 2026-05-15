package binaries

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// ReadArtifactVersion reads the normalized VERSION metadata from an extracted
// pv-managed archive root.
func ReadArtifactVersion(rootDir, artifactName string) (string, error) {
	data, err := os.ReadFile(filepath.Join(rootDir, "VERSION"))
	if err != nil {
		if os.IsNotExist(err) {
			return "", fmt.Errorf("%s artifact VERSION is missing", artifactName)
		}
		return "", fmt.Errorf("read %s artifact VERSION: %w", artifactName, err)
	}
	version := strings.TrimSpace(string(data))
	if version == "" {
		return "", fmt.Errorf("%s artifact VERSION is empty", artifactName)
	}
	return version, nil
}
