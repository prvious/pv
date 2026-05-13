package mailpit

import (
	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/spf13/cobra"
)

var logsFollow bool

var logsCmd = &cobra.Command{
	Use:     "mailpit:logs [version]",
	GroupID: "mailpit",
	Short:   "Tail the Mailpit log file",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		return pkg.TailLog(cmd.Context(), resolved, logsFollow)
	},
}

func init() {
	logsCmd.Flags().BoolVarP(&logsFollow, "follow", "f", false, "Follow the log (tail -f)")
}
