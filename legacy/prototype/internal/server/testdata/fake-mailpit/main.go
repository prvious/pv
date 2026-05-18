// Package main is a test-only fake mailpit binary compiled by manager_test.go.
// It starts a minimal HTTP server on 127.0.0.1:8025 with a /livez endpoint
// so the supervisor's HTTP ready-check succeeds, then sleeps indefinitely.
package main

import (
	"fmt"
	"net/http"
	"os"
	"time"
)

func main() {
	http.HandleFunc("/livez", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(200)
	})
	if err := http.ListenAndServe("127.0.0.1:8025", nil); err != nil {
		fmt.Fprintf(os.Stderr, "fake-mailpit: %v\n", err)
		os.Exit(1)
	}
	time.Sleep(1 * time.Hour)
}
