package initgen

import (
	"fmt"
	"strings"
)

func static(opts Options) string {
	var b strings.Builder
	fmt.Fprintf(&b, "php: %q\n", opts.PHP)
	b.WriteString("\n")
	b.WriteString("# Static site — no setup pipeline needed.\n")
	b.WriteString("# Add `aliases:`, `env:`, or `setup:` blocks as your project grows.\n")
	return b.String()
}
