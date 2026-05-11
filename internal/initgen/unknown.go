package initgen

import (
	"fmt"
	"strings"
)

func unknown(opts Options) string {
	var b strings.Builder
	fmt.Fprintf(&b, "php: %q\n", opts.PHP)
	b.WriteString("\n")
	b.WriteString("# pv couldn't identify this project's type. Add the blocks you need:\n")
	b.WriteString("# - aliases:    extra hostnames Caddy should serve\n")
	b.WriteString("# - env:        project-level env keys (e.g., APP_URL)\n")
	b.WriteString("# - postgresql / mysql / redis / mailpit / rustfs: backing service declarations\n")
	b.WriteString("# - setup:      shell commands to run after `pv link`\n")
	return b.String()
}
