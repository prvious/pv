package cmd

import "github.com/prvious/pv/internal/commands/ca"

func init() {
	ca.Register(rootCmd)
}
