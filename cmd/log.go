package cmd

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var (
	logFollow bool
	logLines  int
)

var logCmd = &cobra.Command{
	Use:   "log [site]",
	Short: "Tail the FrankenPHP log",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		logPath := config.CaddyLogPath()
		f, err := os.Open(logPath)
		if err != nil {
			if os.IsNotExist(err) {
				return fmt.Errorf("no log file found at %s (has the server been started?)", logPath)
			}
			return fmt.Errorf("cannot open log file: %w", err)
		}
		defer f.Close()

		var filter string
		if len(args) > 0 {
			filter = args[0]
		}

		// Read last N lines.
		lines, err := tailLines(f, logLines, filter)
		if err != nil {
			return err
		}
		for _, line := range lines {
			fmt.Println(line)
		}

		if !logFollow {
			return nil
		}

		// Follow mode: seek to end and poll for new content.
		if _, err := f.Seek(0, io.SeekEnd); err != nil {
			return fmt.Errorf("cannot seek: %w", err)
		}

		scanner := bufio.NewScanner(f)
		for {
			for scanner.Scan() {
				line := scanner.Text()
				if filter == "" || strings.Contains(line, filter) {
					fmt.Println(line)
				}
			}
			time.Sleep(200 * time.Millisecond)
		}
	},
}

// tailLines reads the last n lines from f that match the optional filter.
func tailLines(f *os.File, n int, filter string) ([]string, error) {
	if _, err := f.Seek(0, io.SeekStart); err != nil {
		return nil, err
	}

	var all []string
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := scanner.Text()
		if filter == "" || strings.Contains(line, filter) {
			all = append(all, line)
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}

	if len(all) > n {
		all = all[len(all)-n:]
	}
	return all, nil
}

func init() {
	logCmd.Flags().BoolVarP(&logFollow, "follow", "f", false, "Follow log output")
	logCmd.Flags().IntVarP(&logLines, "lines", "n", 50, "Number of lines to show")
	rootCmd.AddCommand(logCmd)
}
