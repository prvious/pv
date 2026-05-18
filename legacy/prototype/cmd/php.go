package cmd

import "github.com/prvious/pv/internal/commands/php"

func init() {
	php.Register(rootCmd)
}
