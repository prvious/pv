//go:build ignore

// Synthetic redis-server used by version_test.go, install_test.go,
// process_test.go, and the server manager reconcile tests. Two modes:
//
//  1. --version: prints a real-looking redis-server version banner, exits 0.
//  2. long-run:  parse --port=<n> (or "--port <n>"), bind 127.0.0.1:<n>,
//     sleep until SIGTERM.
//
// This is a Go program, not a shell/python/ruby/node stub — per CLAUDE.md
// the only allowed runtime dependency is `go`.
package main

import (
	"fmt"
	"net"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
)

func main() {
	var (
		versionMode bool
		port        int
	)
	args := os.Args[1:]
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case a == "--version":
			versionMode = true
		case a == "--port" && i+1 < len(args):
			if n, err := strconv.Atoi(args[i+1]); err == nil {
				port = n
			}
			i++
		case strings.HasPrefix(a, "--port="):
			if n, err := strconv.Atoi(strings.TrimPrefix(a, "--port=")); err == nil {
				port = n
			}
		}
	}

	if versionMode {
		// Mirror real redis-server output verbatim. parseRedisVersion in
		// internal/redis/version.go must match this.
		fmt.Println("Redis server v=7.4.1 sha=00000000:0 malloc=libc bits=64 build=fakefakefakefake")
		return
	}

	if port == 0 {
		port = 6379
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
