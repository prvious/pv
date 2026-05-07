//go:build ignore

// Synthetic initdb used by initdb_test.go. Creates a PG_VERSION file at
// the path passed as `-D <dir>`, then exits 0.
package main

import (
	"fmt"
	"os"
	"path/filepath"
)

func main() {
	var dir string
	for i, a := range os.Args {
		if a == "-D" && i+1 < len(os.Args) {
			dir = os.Args[i+1]
		}
	}
	if dir == "" {
		fmt.Fprintln(os.Stderr, "fake-initdb: -D required")
		os.Exit(2)
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(filepath.Join(dir, "PG_VERSION"), []byte("17\n"), 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(filepath.Join(dir, "postgresql.conf"), []byte("# fake initdb\n"), 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
