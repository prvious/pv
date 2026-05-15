package mailpit

import (
	"fmt"

	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "mailpit:stop [version]",
	GroupID: "mailpit",
	Short:   "Mark Mailpit as wanted-stopped",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		resolved, err := pkg.ResolveVersion(argVersion(args))
		if err != nil {
			return err
		}
		if err := pkg.SetWanted(resolved, pkg.WantedStopped); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("%s %s marked stopped.", pkg.DisplayName(), resolved))
		return signalDaemon(pkg.DisplayName())
	},
}
