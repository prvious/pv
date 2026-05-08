//go:build ignore

// Synthetic mysqld used by version_test.go.
// Compiled into the test temp dir at test time.
package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) >= 2 && os.Args[1] == "--version" {
		// Mirror real mysqld 8.4.9 output verbatim — the parser in
		// internal/mysql/version.go must match this exactly.
		fmt.Println("mysqld  Ver 8.4.9 for macos15 on arm64 (MySQL Community Server - GPL)")
		return
	}
	fmt.Fprintln(os.Stderr, "fake mysqld: unexpected args")
	os.Exit(2)
}
