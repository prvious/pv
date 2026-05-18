package packages

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"time"
)

const retryDelay = 30 * time.Second

// StartBackgroundUpdater starts a goroutine that checks for package updates
// immediately, then on every tick of the given interval. Stops when ctx is cancelled.
func StartBackgroundUpdater(ctx context.Context, client *http.Client, interval time.Duration) {
	go func() {
		updateAllSilent(ctx, client)

		ticker := time.NewTicker(interval)
		defer ticker.Stop()

		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				updateAllSilent(ctx, client)
			}
		}
	}()
}

// updateAllSilent updates all packages, retrying once on failure per package.
// Errors are logged to stderr but do not propagate — this runs in the background.
// Note: the retry delay is context-aware and will abort on shutdown.
func updateAllSilent(ctx context.Context, client *http.Client) {
	for _, pkg := range Managed {
		select {
		case <-ctx.Done():
			return
		default:
		}

		_, _, err := Update(ctx, client, pkg)
		if err != nil {
			fmt.Fprintf(os.Stderr, "pv: background update for %s failed, retrying in %s: %v\n", pkg.Name, retryDelay, err)
			select {
			case <-ctx.Done():
				return
			case <-time.After(retryDelay):
			}
			_, _, retryErr := Update(ctx, client, pkg)
			if retryErr != nil {
				fmt.Fprintf(os.Stderr, "pv: background update for %s failed after retry: %v\n", pkg.Name, retryErr)
			}
		}
	}
}
