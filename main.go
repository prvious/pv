package main

import (
	"github.com/prvious/pv/internal/app"

	// Import action packages to trigger init() functions
	_ "github.com/prvious/pv/internal/actions/docker"
)

func main() {
	app.Run()
}
