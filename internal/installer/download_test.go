package installer

import (
	"context"
	"sync"
	"testing"
)

func TestDownloadBoundsParallelismAndKeepsResultOrder(t *testing.T) {
	items := []Item{
		{ID: Identity{Kind: KindRuntime, Name: "php", Version: "8.4.1"}},
		{ID: Identity{Kind: KindTool, Name: "composer", Version: "2.9.2"}},
		{ID: Identity{Kind: KindService, Name: "mailpit", Version: "1.0.0"}},
	}
	downloader := &countingDownloader{release: make(chan struct{})}
	done := make(chan []DownloadResult, 1)
	go func() {
		results, err := Download(context.Background(), items, 2, downloader)
		if err != nil {
			t.Errorf("Download returned error: %v", err)
		}
		done <- results
	}()

	downloader.waitForActive(t, 2)
	if got := downloader.maxActive(); got > 2 {
		t.Fatalf("max active downloads = %d, want <= 2", got)
	}
	close(downloader.release)
	results := <-done
	for i, result := range results {
		if result.ID != items[i].ID {
			t.Fatalf("results[%d] = %s, want %s", i, result.ID, items[i].ID)
		}
		if result.Err != nil {
			t.Fatalf("results[%d] error = %v", i, result.Err)
		}
	}
}

type countingDownloader struct {
	release chan struct{}

	mu       sync.Mutex
	active   int
	max      int
	activeCh chan struct{}
}

func (d *countingDownloader) Download(context.Context, Item) error {
	d.mu.Lock()
	if d.activeCh == nil {
		d.activeCh = make(chan struct{})
	}
	d.active++
	d.max = max(d.max, d.active)
	close(d.activeCh)
	d.activeCh = make(chan struct{})
	d.mu.Unlock()

	<-d.release

	d.mu.Lock()
	d.active--
	d.mu.Unlock()
	return nil
}

func (d *countingDownloader) waitForActive(t *testing.T, want int) {
	t.Helper()
	for {
		d.mu.Lock()
		active := d.active
		ch := d.activeCh
		if ch == nil {
			ch = make(chan struct{})
			d.activeCh = ch
		}
		d.mu.Unlock()
		if active >= want {
			return
		}
		<-ch
	}
}

func (d *countingDownloader) maxActive() int {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.max
}
