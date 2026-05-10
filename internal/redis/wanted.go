package redis

import (
	"fmt"
	"os"
)

// IsWanted reports whether redis should currently be supervised:
// state says wanted=running AND the binary is on disk. Stale entries
// (state says running but binary is missing) emit a stderr warning and
// return false — recovery is `redis:install` after the binary is
// restored.
func IsWanted() bool {
	st, err := LoadState()
	if err != nil {
		fmt.Fprintf(os.Stderr, "redis: load state: %v\n", err)
		return false
	}
	if st.Wanted != WantedRunning {
		return false
	}
	if !IsInstalled() {
		fmt.Fprintln(os.Stderr, "redis: state.json wants redis running but binary is missing; skipping")
		return false
	}
	return true
}
