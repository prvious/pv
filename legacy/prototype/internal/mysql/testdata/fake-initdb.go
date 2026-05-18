//go:build ignore

// Synthetic mysqld --initialize-insecure used by initdb_test.go. Reads
// --datadir=<dir> from the args, creates auto.cnf inside it, and exits 0.
// Mirrors the real mysqld's "writes auto.cnf with a generated server-uuid
// during init" behavior — auto.cnf's presence is what RunInitdb uses to
// decide whether init has already run.
package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

func main() {
	var dir string
	for _, a := range os.Args[1:] {
		if strings.HasPrefix(a, "--datadir=") {
			dir = strings.TrimPrefix(a, "--datadir=")
		}
	}
	if dir == "" {
		fmt.Fprintln(os.Stderr, "fake-initdb: --datadir= required")
		os.Exit(2)
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := os.WriteFile(filepath.Join(dir, "auto.cnf"), []byte("[auto]\nserver-uuid=fake-0000-0000-0000-000000000000\n"), 0o644); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
