package cmd

import "github.com/prvious/pv/internal/commands/mago"

func init() {
	mago.Register(rootCmd)
}
