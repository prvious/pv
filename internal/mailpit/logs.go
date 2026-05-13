package mailpit

import (
	"context"
	"fmt"
	"io"
	"os"
	"time"
)

// TailLog writes the contents of the service log file for the given
// version to stdout. If follow is true, it polls the file every
// 250 ms and continues writing new content until ctx is cancelled.
func TailLog(ctx context.Context, version string, follow bool) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	logPath, err := LogPath(version)
	if err != nil {
		return err
	}
	f, err := os.Open(logPath)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("no log file yet (%s). Has the service run?", logPath)
		}
		return err
	}
	defer f.Close()

	if _, err := io.Copy(os.Stdout, f); err != nil {
		return err
	}
	if !follow {
		return nil
	}
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(250 * time.Millisecond):
		}
		if _, err := io.Copy(os.Stdout, f); err != nil {
			if err == io.EOF {
				continue
			}
			return err
		}
	}
}
