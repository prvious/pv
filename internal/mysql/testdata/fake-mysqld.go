//go:build ignore

// Synthetic mysqld used by version_test.go, install_test.go,
// process_test.go, and the server manager reconcile tests. Three modes:
//
//  1. --version: prints a real-looking mysqld version banner, exits 0.
//  2. --initialize-insecure: read --datadir=<dir>, create auto.cnf, exit 0.
//  3. long-run: parse --port=<n>, bind 127.0.0.1:<n>, sleep until SIGTERM.
//
// This is a Go program, not a shell/python/ruby/node stub — per CLAUDE.md
// the only allowed runtime dependency is `go`.
package main

import (
	"fmt"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
)

func main() {
	var (
		initMode    bool
		versionMode bool
		dataDir     string
		port        int
	)
	for _, a := range os.Args[1:] {
		switch {
		case a == "--version":
			versionMode = true
		case a == "--initialize-insecure":
			initMode = true
		case strings.HasPrefix(a, "--datadir="):
			dataDir = strings.TrimPrefix(a, "--datadir=")
		case strings.HasPrefix(a, "--port="):
			if n, err := strconv.Atoi(strings.TrimPrefix(a, "--port=")); err == nil {
				port = n
			}
		}
	}

	if versionMode {
		// Mirror real mysqld 8.4.9 output verbatim — the parser in
		// internal/mysql/version.go must match this exactly.
		fmt.Println("mysqld  Ver 8.4.9 for macos15 on arm64 (MySQL Community Server - GPL)")
		return
	}

	if initMode {
		if dataDir == "" {
			os.Exit(2)
		}
		if err := os.MkdirAll(dataDir, 0o755); err != nil {
			os.Exit(1)
		}
		if err := os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\nserver-uuid=fake-0000-0000-0000-000000000000\n"), 0o644); err != nil {
			os.Exit(1)
		}
		return
	}

	if port == 0 {
		os.Exit(3)
	}
	l, err := net.Listen("tcp", "127.0.0.1:"+strconv.Itoa(port))
	if err != nil {
		os.Exit(4)
	}
	sigs := make(chan os.Signal, 1)
	signal.Notify(sigs, syscall.SIGTERM, syscall.SIGINT)
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			c.Close()
		}
	}()
	<-sigs
	l.Close()
}
