//go:build ignore

// Synthetic pg_config used by version_test.go.
// Compiled into the test temp dir at test time.
package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) >= 2 && os.Args[1] == "--version" {
		fmt.Println("PostgreSQL 17.5")
		return
	}
	fmt.Fprintln(os.Stderr, "fake pg_config: unexpected args")
	os.Exit(2)
}
