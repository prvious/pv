package cmd

import "github.com/prvious/pv/internal/commands/composer"

func init() {
	composer.Register(rootCmd)
}
