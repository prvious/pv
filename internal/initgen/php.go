package initgen

import (
	"fmt"
	"strings"
)

func php(opts Options) string {
	var b strings.Builder
	fmt.Fprintf(&b, "php: %q\n\n", opts.PHP)
	b.WriteString("# Each line runs in its own bash -c with the pinned PHP on PATH.\n")
	b.WriteString("setup:\n")
	b.WriteString("  - composer install\n")
	return b.String()
}
