package installer

import (
	"context"
	"errors"
	"sync"
)

type Downloader interface {
	Download(context.Context, Item) error
}

type DownloadResult struct {
	ID  Identity
	Err error
}

func Download(ctx context.Context, items []Item, maxParallel int, downloader Downloader) ([]DownloadResult, error) {
	if maxParallel < 1 {
		return nil, errors.New("max parallel downloads must be at least 1")
	}
	if downloader == nil {
		return nil, errors.New("downloader is required")
	}

	results := make([]DownloadResult, len(items))
	jobs := make(chan int)
	var wg sync.WaitGroup
	workers := min(maxParallel, len(items))
	for range workers {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for index := range jobs {
				item := items[index]
				results[index] = DownloadResult{
					ID:  item.ID,
					Err: downloader.Download(ctx, item),
				}
			}
		}()
	}

	for index := range items {
		if err := ctx.Err(); err != nil {
			close(jobs)
			wg.Wait()
			for i := index; i < len(items); i++ {
				results[i] = DownloadResult{ID: items[i].ID, Err: err}
			}
			return results, err
		}
		jobs <- index
	}
	close(jobs)
	wg.Wait()
	return results, ctx.Err()
}
