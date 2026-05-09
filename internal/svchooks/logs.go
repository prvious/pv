package svchooks

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/services"
)

// TailLog tails ~/.pv/logs/<binary>.log to stdout. When follow is true,
// the function blocks polling for new bytes every 250ms and exits when
// ctx is cancelled (Ctrl-C). When follow is false, it dumps existing
// content and returns.
func TailLog(ctx context.Context, svc services.BinaryService, follow bool) error {
	logPath := filepath.Join(config.PvDir(), "logs", svc.Binary().Name+".log")
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
