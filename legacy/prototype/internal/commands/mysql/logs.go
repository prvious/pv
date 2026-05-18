package mysql

import (
	"io"
	"os"
	"os/exec"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var logsFollow bool

var logsCmd = &cobra.Command{
	Use:     "mysql:logs [version]",
	GroupID: "mysql",
	Short:   "Tail a MySQL version's log file",
	Long:    "Reads ~/.pv/logs/mysql-<version>.log. With -f / --follow, tails the file like `tail -f`.",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := ResolveVersion(args)
		if err != nil {
			return err
		}
		path := config.MysqlLogPath(version)
		if logsFollow {
			c := exec.Command("tail", "-f", path)
			c.Stdout = os.Stdout
			c.Stderr = os.Stderr
			return c.Run()
		}
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		defer f.Close()
		_, err = io.Copy(os.Stdout, f)
		return err
	},
}

func init() {
	logsCmd.Flags().BoolVarP(&logsFollow, "follow", "f", false, "Follow the log (tail -f)")
}
