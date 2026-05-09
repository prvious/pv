package mailpit

import (
	"fmt"

	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/spf13/cobra"
)

var logsFollow bool

var logsCmd = &cobra.Command{
	Use:     "mailpit:logs",
	GroupID: "mailpit",
	Short:   "Tail the Mailpit log file",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		svc, ok := services.LookupBinary("mail")
		if !ok {
			return fmt.Errorf("mailpit binary service not registered (build issue)")
		}
		return svchooks.TailLog(cmd.Context(), svc, logsFollow)
	},
}

func init() {
	logsCmd.Flags().BoolVarP(&logsFollow, "follow", "f", false, "Follow the log (tail -f)")
}
