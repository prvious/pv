package packages

import (
	"context"
	"net/http"
	"time"
)

const retryDelay = 30 * time.Second

// StartBackgroundUpdater starts a goroutine that checks for package updates
// immediately, then on every tick of the given interval. Stops when ctx is cancelled.
// The client parameter enables testability via urlRewriteTransport.
func StartBackgroundUpdater(ctx context.Context, client *http.Client, interval time.Duration) {
	go func() {
		updateAllSilent(client)

		ticker := time.NewTicker(interval)
		defer ticker.Stop()

		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				updateAllSilent(client)
			}
		}
	}()
}

// updateAllSilent updates all packages, retrying once on failure per package.
// All errors are swallowed — this runs in the background.
func updateAllSilent(client *http.Client) {
	for _, pkg := range Managed {
		_, _, err := Update(client, pkg)
		if err != nil {
			time.Sleep(retryDelay)
			Update(client, pkg) //nolint:errcheck // best-effort retry
		}
	}
}
