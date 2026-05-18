package ui

import (
	"fmt"
	"os"
	"time"
)

// Footer prints the completion summary with elapsed time.
func Footer(start time.Time, docsURL string) {
	elapsed := time.Since(start).Round(time.Second)
	fmt.Fprintf(os.Stderr, "\n  %s Run %s in a project to get started.\n",
		Green.Render(fmt.Sprintf("Ready in %s.", elapsed)),
		Bold.Render("pv link"),
	)
	if docsURL != "" {
		fmt.Fprintf(os.Stderr, "  %s\n", Muted.Render("Docs: "+docsURL))
	}
	fmt.Fprintln(os.Stderr)
}
