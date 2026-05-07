//go:build ignore

// Synthetic postgres server used by manager_test. Reads -D <dir>, opens
// the data dir's postgresql.conf to discover the port, and binds a TCP
// listener so the supervisor's TCP ready-check passes.
package main

import (
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"syscall"
)

func main() {
	var dir string
	for i, a := range os.Args {
		if a == "-D" && i+1 < len(os.Args) {
			dir = os.Args[i+1]
		}
	}
	if dir == "" {
		os.Exit(2)
	}
	conf, _ := os.ReadFile(filepath.Join(dir, "postgresql.conf"))
	re := regexp.MustCompile(`(?m)^port\s*=\s*(\d+)`)
	m := re.FindStringSubmatch(string(conf))
	port := 54017
	if len(m) == 2 {
		if n, err := strconv.Atoi(strings.TrimSpace(m[1])); err == nil {
			port = n
		}
	}
	l, err := net.Listen("tcp", "127.0.0.1:"+strconv.Itoa(port))
	if err != nil {
		os.Exit(3)
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
