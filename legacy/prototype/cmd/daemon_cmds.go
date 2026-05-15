package cmd

import "github.com/prvious/pv/internal/commands/daemon"

func init() {
	daemon.Register(rootCmd)
}
