package phpenv

import "github.com/prvious/pv/internal/tools"

func init() {
	// Wire up the expose function to break the import cycle.
	// phpenv.updateSymlinks() uses this to delegate to tools.Expose().
	ExposeFunc = func(name string) error {
		t := tools.Get(name)
		if t == nil {
			return nil
		}
		return tools.Expose(t)
	}
}

// WriteShims delegates to tools.ExposeAll() which creates all shims and symlinks.
func WriteShims() error {
	return tools.ExposeAll()
}
