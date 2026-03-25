package cmd

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"strings"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var (
	logFollow bool
	logLines  int
	logError  bool
	logDaemon bool
)

var logCmd = &cobra.Command{
	Use:     "log [site]",
	GroupID: "core",
	Short:   "Tail the FrankenPHP log",
	Example: `# Tail all logs
pv log

# Follow logs in real time
pv log -f

# Show last 50 lines for a specific site
pv log myapp -n 50`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		logPath := config.CaddyLogPath()
		if logError {
			logPath = config.DaemonErrLogPath()
		} else if logDaemon {
			logPath = config.DaemonLogPath()
		}
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
		// Uses inode detection to handle log rotation (like tail -F).
		if _, err := f.Seek(0, io.SeekEnd); err != nil {
			return fmt.Errorf("cannot seek: %w", err)
		}

		scanner := bufio.NewScanner(f)
		checkCount := 0
		for {
			for scanner.Scan() {
				line := scanner.Text()
				if filter == "" || strings.Contains(line, filter) {
					fmt.Println(line)
				}
			}
			time.Sleep(200 * time.Millisecond)

			// Every 5 polls (~1s), check if the file was rotated.
			checkCount++
			if checkCount >= 5 {
				checkCount = 0
				openInfo, err := f.Stat()
				if err != nil {
					continue
				}
				diskInfo, err := os.Stat(logPath)
				if err != nil {
					continue
				}
				if getInode(openInfo) != getInode(diskInfo) {
					f.Close()
					f, err = os.Open(logPath)
					if err != nil {
						return fmt.Errorf("cannot reopen rotated log: %w", err)
					}
					scanner = bufio.NewScanner(f)
				}
			}
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

func getInode(info os.FileInfo) uint64 {
	if stat, ok := info.Sys().(*syscall.Stat_t); ok {
		return stat.Ino
	}
	return 0
}

func init() {
	logCmd.Flags().BoolVarP(&logFollow, "follow", "f", false, "Follow log output")
	logCmd.Flags().IntVarP(&logLines, "lines", "n", 50, "Number of lines to show")
	logCmd.Flags().BoolVar(&logError, "error", false, "Show daemon stderr log")
	logCmd.Flags().BoolVar(&logDaemon, "daemon", false, "Show daemon stdout log")
	rootCmd.AddCommand(logCmd)
}
