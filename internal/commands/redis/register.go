// Package redis holds cobra commands for the redis:* group. There is
// intentionally no alias namespace — `redis:` is already short.
package redis

import (
	"github.com/spf13/cobra"
)

// Register wires every redis:* command onto parent.
func Register(parent *cobra.Command) {
	cmds := []*cobra.Command{}
	for _, c := range cmds {
		parent.AddCommand(c)
	}
}
