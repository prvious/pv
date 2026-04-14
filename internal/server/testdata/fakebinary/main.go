// Package main is a test-only fake binary compiled by manager_test.go.
// It binds 127.0.0.1:<port> (default 9000) so the supervisor's TCP
// ready-check succeeds, then sleeps indefinitely until killed.
//
// This exists to remove a hidden dependency on python3 from the test suite.
package main

import (
	"flag"
	"fmt"
	"net"
	"os"
	"time"
)

func main() {
	port := flag.Int("port", 9000, "port to bind on 127.0.0.1")
	flag.Parse()

	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", *port))
	if err != nil {
		fmt.Fprintf(os.Stderr, "fakebinary: listen %d: %v\n", *port, err)
		os.Exit(1)
	}
	defer ln.Close()

	// Accept+close connections forever so the listener is live for the
	// entire test, but never block waiting for a specific client.
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			_ = c.Close()
		}
	}()

	// Stay alive until the supervisor kills us. An hour is plenty of time
	// for any test that uses this helper.
	time.Sleep(1 * time.Hour)
}
