package main

import (
	"fmt"
	"io"
	"os"

	"github.com/prvious/pv/internal/cli"
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout io.Writer, stderr io.Writer) int {
	trackedStderr := &countingWriter{writer: stderr}
	if err := cli.Run(args, stdout, trackedStderr); err != nil {
		if trackedStderr.count == 0 {
			fmt.Fprintf(stderr, "pv: %v\n", err)
		}
		return 1
	}
	return 0
}

type countingWriter struct {
	writer io.Writer
	count  int
}

func (w *countingWriter) Write(data []byte) (int, error) {
	n, err := w.writer.Write(data)
	w.count += n
	return n, err
}
