//go:build !windows

package installer

import "os"

func replaceFile(tempPath string, path string) error {
	return os.Rename(tempPath, path)
}
